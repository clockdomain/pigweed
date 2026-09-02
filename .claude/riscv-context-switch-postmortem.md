# RISC-V Context Switch Post-Mortem

**pw_kernel failure analysis** — branch `riscv-irq-deferred-reschedule`, commits
`6977180eb` (fix 1) and `ca7d45591` (fix 2). Reproducer:
`//target/veer/tests/i3c_user_irq` (downstream openprot; Caliptra VeeR + PIC + I3C
userspace-interrupt test). Written 2026-09-01. Shareable version:
https://claude.ai/code/artifact/6ca6db86-35af-4964-bf94-80ae3be50309

The first time an interrupt handler ever woke a thread on the RISC-V port, it
uncovered two stacked defects in `Arch::context_switch` — one hiding behind the
other.

## Summary

Both bugs share one root cause: the RISC-V port was written for a purely
*cooperative* world, where every context switch is initiated from a thread's own
call chain (a `wait()`, a yield, an exit). Its correctness rested on two implicit
conventions — interrupts stay disabled across every switch, and whoever resumes on
the far side of a switch releases the scheduler lock the outgoing thread carried
into it. Neither convention was written down or enforced, and the first
*interrupt-initiated* switch violated both.

| | Bug 1 (fixed by `6977180eb`) | Bug 2 (fixed by `ca7d45591`) |
|---|---|---|
| Defect | Context switches performed synchronously from inside the trap handler | Scheduler lock handed off across voluntary blocks instead of released |
| Symptom | Instruction access fault at PC=0 | Panic: `"recursively locked spinlock"` |

Bug 1 masked bug 2 completely: the PC=0 crash happened before the lock state could
ever be observed. Fixing bug 1 alone converted the crash into bug 2's deterministic
assertion; fixing both produced 8+ consecutive clean passes of the reproducer.

## Background: how the RISC-V port switches threads

The port's primitive is `riscv_context_switch`
(`pw_kernel/arch/riscv/threads.rs:419`): a classic cooperative fiber switch. It
pushes `ra` and `s0–s11` onto the outgoing thread's stack, stores `sp` into the
outgoing thread's frame pointer, loads the incoming thread's saved registers, and
`ret`s — resuming the incoming thread wherever *it* last called the same function.
This is only sound from an ordinary function-call context: the compiler has
spilled caller-saved state around the call site, and the outgoing thread's stack
is in a shape it will understand when it is eventually resumed.

Three kernel-side facts complete the picture:

- The `Arch::context_switch` contract (`pw_kernel/kernel/lib.rs:74`) explicitly
  permits deferral: it may return `switched = false` if the switch will be
  completed later by a pending mechanism "(like PendSV)". The scheduler's only
  hard requirement is that `block()`-initiated switches actually switch.
- The scheduler lock is a single-hart spinlock
  (`pw_kernel/arch/riscv/spinlock.rs`) that disables interrupts while held and
  **panics on recursive acquisition** — there is no spinning on one hart, so a
  second `lock()` while the flag is set can only be a deadlock, and it asserts
  instead.
- Every `SpinLockGuard` embeds a `PreemptDisableGuard` whose `Drop` decrements
  `preempt_disable_count` — a *per-thread* counter reached through the global
  `THREAD_LOCAL_STATE` pointer, which `context_switch` itself repoints at the
  incoming thread. This detail matters for the ordering trap in fix 2.

## Bug 1: switching mid-trap abandons the trap frame

On Cortex-M, `Arch::context_switch` never switches directly — it pends PendSV, and
the hardware runs that handler at the lowest priority only after every other
exception has unwound. The RISC-V port had no equivalent: it called
`riscv_context_switch` unconditionally, no matter who invoked it.

Failure chain:

1. Thread B is running. The I3C interrupt fires; the CPU vectors to `trap_handler`
   (`pw_kernel/arch/riscv/exceptions.rs`), which pushes B's trap frame and
   dispatches to the driver's interrupt handler.
2. The handler signals a wait queue. The wake path takes the scheduler lock, sees
   that woken thread A outranks B, and calls `Arch::context_switch` — from
   interrupt context, with the scheduler lock and the wait-queue's own guards all
   live on the trap handler's call chain.
3. **The old code performed the cooperative switch immediately**: it saved B's
   callee-saved registers *mid-trap-handler* and `ret` into thread A. The trap
   never completes: B's trap frame sits abandoned on its stack — the
   `mepc`/`mret` bookkeeping that must restore the interrupted context never
   runs — and every RAII guard on the interrupt's call chain, including the
   scheduler lock passed into this very call, is frozen mid-flight rather than
   unwound.
4. Scheduler and thread state are now inconsistent (locks that will never be
   released on time, a thread suspended halfway through its own interrupt).
   Execution continues briefly on corrupted state and dies with an instruction
   access fault at PC=0.

### The fix: a software PendSV

Commit `6977180eb` ports Cortex-M's stash-and-defer pattern. RISC-V has no
tail-chaining hardware, so the "run only after everything else has unwound"
guarantee is reproduced in software at the tail of `trap_handler`:

```rust
// pw_kernel/arch/riscv/threads.rs — Arch::context_switch
if crate::exceptions::in_hw_interrupt() {
    unsafe {
        if DEFERRED_OLD_THREAD.is_null() {
            DEFERRED_OLD_THREAD = old_thread_state; // first wins: thread physically running
        }
        DEFERRED_NEW_THREAD = new_thread_state;     // latest wins: scheduler's newest decision
    }
    return (sched_state, false); // permitted by the Arch::context_switch contract
}
```

`trap_handler` sets an `IN_HW_INTERRUPT` flag around hardware-interrupt dispatch
(syscalls and exceptions run from a thread's own synchronous call chain and still
switch inline), then calls `complete_deferred_context_switch()` after the handler
returns — at which point every guard the interrupt took has been dropped normally,
and the cooperative switch is safe. The interrupted thread is descheduled from the
trap tail with a fully intact trap frame; when it is later resumed, it finishes
the last few lines of `trap_handler` and `mret`s back to whatever it was doing.

Two deliberate asymmetries:

- **First-wins vs. latest-wins.** If several wakes happen inside one trap,
  `DEFERRED_OLD_THREAD` keeps the *first* outgoing thread — the one whose
  registers are actually on the CPU — while `DEFERRED_NEW_THREAD` is overwritten
  every time, tracking the scheduler's latest choice, exactly as
  `current_arch_thread_state` does for inline switches.
- **Stash, don't re-lock.** The completion routine reads the stashed new-thread
  pointer instead of re-acquiring the scheduler lock the way Cortex-M's
  `pendsv_swap_sp` does. The scheduling decision was already made under the lock
  during the wake; re-locking at the trap tail would collide with the possibility
  that a blocked thread's own last voluntary switch still holds the lock — which,
  before fix 2 landed, was the norm.

## Bug 2: the scheduler lock was handed off, not released

The pre-existing RISC-V code passed the caller's live `SpinLockGuard` straight
through the switch: a thread blocking in `wait()` carried the held lock into
`riscv_context_switch`, and that guard could only drop when the thread was resumed
and its call chain unwound past it — potentially the entire time it stayed
blocked.

Remarkably, this worked — as an *implicit lock handoff*. Every resume path
compensated for the lock the outgoing thread carried in:

- A thread resuming from its own earlier voluntary block returned *its own* stale
  guard as the "reacquired" lock; its caller's eventual drop released the flag.
- A brand-new thread's trampoline called `break_scheduler_lock()`
  (`pw_kernel/arch/riscv/threads.rs`, `prepare_userspace_thread`) — explicitly
  documented as releasing the lock "held by the previous thread".
- And because the guard's `InterruptGuard` kept interrupts disabled across the
  whole handoff window, nothing could ever *observe* the flag mid-handoff.

Fix 1 broke the convention, necessarily: it introduced a third resume edge —
resumption from `trap_handler`'s tail — that has no stale guard to drop and
re-enables interrupts via `mret`. The failure then becomes deterministic:

1. An interrupt wakes thread A while thread B runs; the deferred switch
   deschedules B at the trap tail. B now "parks" inside
   `complete_deferred_context_switch`, holding no locks.
2. Thread A later blocks in `wait()` again: it acquires the scheduler lock
   (`is_locked = true`, interrupts off) and switches to B.
3. **B resumes at the trap tail, finishes `trap_handler`, and `mret`s back to its
   interrupted code — dropping no guard on the way.** The lock flag stays latched
   by blocked thread A, and `mret` restores B's interrupt-enable state: B is now
   running with interrupts *on* while the scheduler lock reads as held.
4. The very next attempt to take the scheduler lock — the next timer tick, the
   next interrupt-driven wake, anything — finds the flag set and panics:
   `"recursively locked spinlock"`.

### The fix: release before switching, reacquire on resume

Commit `ca7d45591` retires the handoff and mirrors what Cortex-M has always done
around its PendSV trigger: drop the guard before switching away, take a fresh one
after being resumed.

```rust
// pw_kernel/arch/riscv/threads.rs — Arch::context_switch
drop(sched_state); // release lock + this thread's PreemptDisableGuard

unsafe { THREAD_LOCAL_STATE = NonNull::from_ref(&(*new_thread_state).local) }

riscv_context_switch(old_thread_frame, new_thread_frame);
// ==== resumed much later, when something switches back to this thread ====
let sched_state = crate::Arch.get_scheduler().lock(crate::Arch);

(sched_state, true)
```

**Ordering trap:** the `drop` must precede the `THREAD_LOCAL_STATE` repoint.
Dropping the guard drops its embedded `PreemptDisableGuard`, which decrements
`preempt_disable_count` through `Arch::thread_local_state()` — i.e. through
whatever `THREAD_LOCAL_STATE` points at *right now*. Repoint first and the
decrement lands on the incoming thread's fresh, zeroed counter: an underflow
(`debug_panic` in debug builds) that corrupts preemption accounting before the
new thread executes a single instruction.

**Interrupt window:** dropping the guard also restores the interrupt-enable
state, so a hardware interrupt can land between the `drop` and the end of the
register swap in `riscv_context_switch`. This is benign, for a reason worth
writing down: any switch that interrupt triggers is deferred to the trap tail,
which saves whatever registers are live into the frame slot of the scheduler's
*current* thread (the incoming one). When that thread is later "resumed", what
actually resumes is the interrupted context, which finishes the original switch
using the frame pointer it read *before* the `drop`. The interrupted context
acts as a relay into the thread the scheduler intended. Cortex-M makes this
explicit with its `active_thread` pointer; on RISC-V it falls out of the frame
pointers being read before the `drop`, so those reads must stay ahead of it.

## Why Cortex-M never had either problem

| | Cortex-M port | RISC-V port (before / after) |
|---|---|---|
| Switch mechanism | PendSV exception; hardware guarantees it runs last, after all other exceptions unwind | Immediate cooperative switch, any context / inline when voluntary, deferred to trap tail when in a hardware interrupt |
| Switch requested from an ISR | Sets `ACTIVE_THREAD`, pends PendSV, returns `false` | Switched mid-trap, abandoning the trap frame / stashes deferred old+new threads, returns `false` |
| Scheduler lock across a block | Dropped before the PendSV trigger, reacquired on resume | Carried through the switch as an implicit handoff / dropped before the switch, reacquired on resume |

The deeper point: PendSV is not just a convenience — it *is* the enforcement
mechanism for "switches only happen from a safe context". Porting the switch
primitive without porting that guarantee left the RISC-V code correct only for the
inputs it had been tested with.

## Verification and takeaways

- `//target/veer/tests/i3c_user_irq` reproduced the PC=0 crash reliably. Fix 1
  alone converts it into the deterministic recursive-lock assertion — confirming
  fix 1 is necessary and correct on its own. Both fixes together: 8+ consecutive
  clean passes. Both commits also build cleanly for
  `//pw_kernel/target/qemu_virt_riscv32`.
- **Layered latent defects:** bug 2 predates bug 1's fix but was unobservable
  until it landed — the crash always came first, and the interrupts-off handoff
  window hid the lock state. Expect a "fixed" crash to become a different,
  cleaner failure; that is progress, not regression.
- **Implicit invariants don't survive new callers:** both conventions the old
  code relied on were real and sound — for the call graph that existed. The
  `Arch::context_switch` doc contract had anticipated interrupt-context deferral
  all along; only one of the two ports implemented it.
- **Single-hart assumptions are load-bearing:** `IN_HW_INTERRUPT` and the two
  deferred-thread statics use plain/relaxed accesses, sound on one hart where the
  trap handler cannot race itself. An SMP port would need to revisit both this
  and the per-hart lock handling.
