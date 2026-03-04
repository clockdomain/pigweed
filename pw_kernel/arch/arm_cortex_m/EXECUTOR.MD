# Building an Async Executor on pw_kernel (Rust Only)

## Overview

This document analyzes the feasibility of building an async executor (Embassy-style)
on top of the Pigweed kernel (`pw_kernel`), using **only Rust** — no C++ involved.

**Verdict: Yes, with two small syscall gaps to fill.**

pw_kernel is written entirely in Rust and already provides the core primitives
needed. The missing pieces are a userspace `get_time()` syscall and a
userspace `yield_wait()` / `sleep_until()` syscall — both straightforward
additions to the existing syscall infrastructure.

---

## pw_kernel Primitive Inventory

### What Already Exists

| Primitive | Kernel API | Location | Executor Use |
|-----------|-----------|----------|-------------|
| **Blocking wait on signals** | `object_wait(handle, signal_mask, deadline)` | `syscall_defs.rs` | Core reactor — block until I/O ready or timeout |
| **Multiplexed wait** | `wait_group_add/remove` + `object_wait` | `kernel/object/wait_group.rs` | epoll-like: wait on N handles simultaneously |
| **Channel IPC** | `channel_transact` (blocking), `channel_read/respond` (non-blocking) | `kernel/object/channel.rs` | Synchronous IPC with deadline support |
| **Async IPC (ABI defined)** | `channel_async_transact`, `channel_async_cancel` | `syscall_defs.rs` | Non-blocking IPC for futures — **ABI exists, handler not yet wired** |
| **Signal notifications** | `Signals` bitflags on every `KernelObject` | `kernel/object.rs` | Readiness notification mapped to `Future::poll` |
| **Events** | `Event` + `EventSignaler` (ISR-safe) | `kernel/sync/event.rs` | Waker-like: signaler can wake from interrupt context |
| **Interrupt objects** | `InterruptObject` + `interrupt_ack` | `kernel/object/interrupt.rs` | Hardware IRQ → signal → userspace async wake |
| **Timer queue** | `Timer`, `schedule_timer`, `cancel_timer` | `kernel/scheduler/timer.rs` | Deadline-driven wakeups for sleep/timeout futures |
| **Kernel-side sleep** | `sleep_until(kernel, deadline)` | `kernel/scheduler.rs` | Blocks thread until deadline — **kernel-side only** |
| **Kernel-side yield** | `yield_timeslice(kernel)` | `kernel/scheduler.rs` | Cooperative yield — **kernel-side only** |
| **Mutex with deadline** | `Mutex::lock()`, `Mutex::lock_until(deadline)` | `kernel/sync/mutex.rs` | Async mutex foundation |
| **SpinLock** | `SpinLock::lock()` (disables preemption) | `kernel/sync/spinlock.rs` | Executor-internal critical sections |
| **Preemption control** | `PreemptDisableGuard` | `kernel/scheduler.rs` | Protect executor internals |
| **Clock / time** | `Instant<Clock>`, `Duration<Clock>`, SysTick-based | `lib/time/`, `arch/arm_cortex_m/timer.rs` | Timekeeping, deadlines |
| **Idle / WFI** | `Arch::idle()` → `cortex_m::asm::wfi()` | `arch/arm_cortex_m/threads.rs` | Power-efficient sleep when no tasks ready |
| **Priority scheduler** | Priority-based round-robin, N priority levels | `kernel/scheduler/algorithm.rs` | Thread scheduling between executor instances |
| **Process/MPU support** | Per-process memory config, privilege separation | `arch/arm_cortex_m/protection_v7.rs`, `v8.rs` | Isolation between executor processes |

### What's Missing (Two Gaps)

| Gap | Status | Effort | Details |
|-----|--------|--------|---------|
| **Userspace `get_time()` syscall** | Placeholder returns 0 | ~30 lines | `userspace/time.rs` has `Clock::now()` returning `Instant::from_ticks(0)`. Need new `SysCallId::GetTime` that reads `Arch::now()`. |
| **Userspace `sleep_until()` or `yield_wait()` syscall** | Kernel-side only | ~40 lines | `sleep_until()` and `yield_timeslice()` exist in `kernel/scheduler.rs` but have no syscall wrappers. Need new `SysCallId` entries. |

---

## Architecture: Executor on pw_kernel

### Design

```
┌─────────────────────────────────────────────┐
│            Userspace Process                 │
│ ┌─────────────────────────────────────────┐ │
│ │         Async Application Code          │ │
│ │   task_a.await    task_b.await    ...   │ │
│ ├─────────────────────────────────────────┤ │
│ │          PwExecutor (Embassy raw)       │ │
│ │  ┌──────────┐ ┌────────┐ ┌──────────┐  │ │
│ │  │ Reactor  │ │ Timer  │ │ Spawner  │  │ │
│ │  │(WaitGroup│ │Queue   │ │          │  │ │
│ │  │+obj_wait)│ │        │ │          │  │ │
│ │  └──────────┘ └────────┘ └──────────┘  │ │
│ └─────────────────────────────────────────┘ │
│                    │ syscalls                │
├────────────────────┼────────────────────────┤
│              pw_kernel                       │
│  ┌──────────┐ ┌────────┐ ┌──────────────┐  │
│  │Scheduler │ │ Timer  │ │  Object /    │  │
│  │(priority │ │ Queue  │ │  Channel /   │  │
│  │ RR)      │ │        │ │  WaitGroup   │  │
│  └──────────┘ └────────┘ └──────────────┘  │
│  ┌──────────┐ ┌────────┐ ┌──────────────┐  │
│  │ Event /  │ │ Mutex  │ │ Interrupt    │  │
│  │ Signaler │ │        │ │ Objects      │  │
│  └──────────┘ └────────┘ └──────────────┘  │
├──────────────────────────────────────────────┤
│        ARM Cortex-M Hardware                 │
│   SVCall │ PendSV │ SysTick │ NVIC │ MPU    │
└──────────────────────────────────────────────┘
```

### Key Mapping: Executor Concepts → pw_kernel Primitives

| Executor Concept | pw_kernel Primitive | Notes |
|-----------------|---------------------|-------|
| **Pender / Waker** | `EventSignaler::signal()` | ISR-safe, wakes blocked threads |
| **Reactor (I/O readiness)** | `object_wait(handle, signals, deadline)` | Blocks until signals match or deadline; per-object signal bits |
| **Multiplexed reactor** | `WaitGroup` + `object_wait` on group handle | Wait on N objects; returns `user_data` identifying which fired |
| **Timer / sleep** | `object_wait` with `deadline` **or** `sleep_until` (needs syscall) | Every blocking syscall already accepts a deadline parameter |
| **Spawn** | `embassy_executor::raw::Executor::spawner()` | Embassy handles task management; OS provides the thread |
| **Yield** | `yield_timeslice` (needs syscall) **or** `object_wait` with `Instant::MIN` | Cooperative yield between executor poll loops |
| **Critical section** | PRIMASK-based `BareSpinLock` (via SVC if unprivileged) | Interrupt disable/enable — already implemented |
| **Idle / low-power** | `Arch::idle()` → `wfi()` | Called by scheduler when no threads are ready |

---

## Executor Implementation

### Core Executor (Runs in a pw_kernel Thread)

```rust
#![no_std]

use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_executor::{raw, Spawner};
use pw_kernel_userspace as syscall;
use pw_kernel_userspace::time::{Instant, Duration};
use syscall_defs::Signals;

/// Atomic flag: interrupt handlers set this to signal pending work.
static SIGNAL_WORK: AtomicBool = AtomicBool::new(false);

/// Pender called by Embassy when a task is woken.
/// This can be called from any context (ISR, another thread, etc.).
#[export_name = "__pender"]
fn __pender(_context: *mut ()) {
    SIGNAL_WORK.store(true, Ordering::SeqCst);
}

/// Async executor backed by pw_kernel primitives.
pub struct PwExecutor {
    inner: raw::Executor,
    not_send: PhantomData<*mut ()>,
}

impl PwExecutor {
    pub const fn new() -> Self {
        Self {
            inner: raw::Executor::new(core::ptr::null_mut()),
            not_send: PhantomData,
        }
    }

    pub fn spawner(&'static self) -> Spawner {
        self.inner.spawner()
    }

    /// Run the executor forever. Call from a pw_kernel thread entry point.
    pub fn run(&'static self, init: impl FnOnce(Spawner)) -> ! {
        init(self.spawner());

        loop {
            // Poll all ready async tasks.
            unsafe { self.inner.poll() };

            // If no new work was signaled, yield to the kernel scheduler.
            // The kernel will put this thread to sleep until an event
            // (interrupt, signal, timer) wakes it.
            if !SIGNAL_WORK.swap(false, Ordering::SeqCst) {
                // Option A: sleep until next timer deadline
                // syscall::sleep_until(next_deadline);
                //
                // Option B: wait on a dedicated event handle
                // syscall::object_wait(self.wake_handle, Signals::USER, Instant::MAX);
                //
                // Option C: yield timeslice (spin-waits, less power efficient)
                // syscall::yield_timeslice();
                //
                // Best option: object_wait on a kernel Event object
                // that the pender signals via EventSignaler.
                yield_to_kernel();
            }
        }
    }
}

/// Yield to the kernel, blocking until the pender signals new work.
///
/// Uses `object_wait` on a dedicated signal object. The pender
/// sets `Signals::USER` on this object from interrupt/thread context.
fn yield_to_kernel() {
    // In the real implementation, this would be:
    //   syscall::object_wait(WAKE_HANDLE, Signals::USER, Instant::MAX)
    // where WAKE_HANDLE is a kernel object whose EventSignaler is
    // used by __pender().
    //
    // For now, with only existing syscalls, we can use object_wait
    // with a very short deadline as a yielding sleep:
    let _ = syscall::object_wait(
        WAKE_HANDLE,
        Signals::USER,
        Instant::MAX,
    );
}
```

### I/O Future Example: Async Channel Transact

```rust
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Future that performs a channel transaction without blocking the executor.
pub struct ChannelTransactFuture<'a> {
    handle: u32,
    send_buf: &'a [u8],
    recv_buf: &'a mut [u8],
    deadline: Instant,
    started: bool,
}

impl<'a> Future for ChannelTransactFuture<'a> {
    type Output = Result<usize, pw_status::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.started {
            // Initiate async transaction (non-blocking).
            // Uses channel_async_transact once it's wired up,
            // or falls back to checking signals.
            self.started = true;
        }

        // Check if the channel's READABLE signal is set (transaction complete).
        match syscall::object_wait(self.handle, Signals::READABLE, Instant::MIN) {
            Ok(wait_return) => {
                // Transaction complete — read the response.
                let len = syscall::channel_read(
                    self.handle, 0, self.recv_buf
                )?;
                Poll::Ready(Ok(len))
            }
            Err(pw_status::Error::DeadlineExceeded) => {
                // Not ready yet. Register waker so we get polled again
                // when signals change.
                //
                // TODO: register waker with WaitGroup so the executor
                // thread is woken when this handle becomes readable.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}
```

### Interrupt-Driven Async I/O

```rust
/// Future that waits for a hardware interrupt via an InterruptObject.
pub struct InterruptFuture {
    handle: u32,
    signal_mask: Signals,
}

impl Future for InterruptFuture {
    type Output = Result<Signals, pw_status::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Non-blocking check: did the interrupt fire?
        match syscall::object_wait(self.handle, self.signal_mask, Instant::MIN) {
            Ok(wait_return) => {
                // Acknowledge the interrupt so it can fire again.
                let _ = syscall::interrupt_ack(self.handle, wait_return.pending_signals);
                Poll::Ready(Ok(wait_return.pending_signals))
            }
            Err(pw_status::Error::DeadlineExceeded) => {
                // Not yet — register for wakeup.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}
```

### Efficient Reactor: WaitGroup-Based Multiplexing

The naive approach (polling each handle with `Instant::MIN` deadline) works
but wastes CPU. The efficient approach uses `WaitGroup`:

```rust
/// Reactor that multiplexes waits across many kernel objects.
///
/// Uses a pw_kernel WaitGroup to block on all registered handles
/// at once, then dispatches wakeups to the correct futures.
pub struct Reactor {
    wait_group_handle: u32,
    wakers: [Option<core::task::Waker>; MAX_HANDLES],
}

impl Reactor {
    /// Register a kernel object handle for async readiness notification.
    pub fn register(
        &mut self,
        handle: u32,
        signal_mask: Signals,
        waker: core::task::Waker,
    ) -> Result<(), pw_status::Error> {
        let slot = self.find_slot(handle);
        self.wakers[slot] = Some(waker);

        syscall::wait_group_add(
            self.wait_group_handle,
            handle,
            signal_mask,
            slot, // user_data = slot index
        )
    }

    /// Block until any registered handle becomes ready, then wake
    /// the corresponding future.
    pub fn wait_for_events(&mut self, deadline: Instant) {
        match syscall::object_wait(
            self.wait_group_handle,
            Signals::READABLE | Signals::USER,
            deadline,
        ) {
            Ok(wait_return) => {
                let slot = wait_return.user_data;
                if let Some(waker) = self.wakers[slot].take() {
                    waker.wake();
                }
            }
            Err(_) => {
                // Timeout or error — wake all to let futures check deadlines.
                for waker in self.wakers.iter_mut().flatten() {
                    waker.wake_by_ref();
                }
            }
        }
    }
}
```

---

## Gaps to Fill

### 1. `get_time()` Syscall (~30 Lines)

**Current state**: `userspace/time.rs` has a placeholder:
```rust
fn now() -> Instant<Self> {
    // Placeholder until a get_time() system call exists.
    Instant::from_ticks(0)
}
```

**What to add**:

```rust
// In syscall_defs.rs — add to SysCallId enum:
SysCallId::GetTime = 0x0007,

// In kernel/syscall.rs — add handler:
fn handle_get_time<K: Kernel>(kernel: K) -> Result<u64> {
    Ok(kernel.now().ticks())
}

// In userspace/time.rs — fix Clock::now():
fn now() -> Instant<Self> {
    Instant::from_ticks(syscall::get_time())
}
```

### 2. `sleep_until()` / `yield_wait()` Syscall (~40 Lines)

**Current state**: `sleep_until()` and `yield_timeslice()` are kernel-internal.

**What to add**:

```rust
// In syscall_defs.rs — add to SysCallId enum:
SysCallId::SleepUntil    = 0x0008,
SysCallId::YieldTimeslice = 0x0009,

// In kernel/syscall.rs — add handlers:
fn handle_sleep_until<K: Kernel>(kernel: K, mut args: K::SyscallArgs<'_>) -> Result<u64> {
    let deadline: Instant<K::Clock> = args.next_instant()?;
    scheduler::sleep_until(kernel, deadline)?;
    Ok(0)
}

fn handle_yield_timeslice<K: Kernel>(kernel: K) -> Result<u64> {
    scheduler::yield_timeslice(kernel);
    Ok(0)
}

// In userspace/syscall.rs — add wrappers:
pub fn sleep_until(deadline: Instant) -> Result<()> { ... }
pub fn yield_timeslice() -> Result<()> { ... }
```

**Note:** Strictly speaking, neither syscall is required for a minimal executor.
`object_wait` with a deadline already provides both sleeping and yielding
behavior. These syscalls are convenience optimizations.

---

## What pw_kernel Already Provides (No Porting Needed)

Compared to typical embedded async executor requirements:

| Executor Requirement | pw_kernel Equivalent | Status |
|--------------------------|---------------------|--------|
| Yield-wait syscall | `object_wait` with deadline / `yield_timeslice` | ✅ Exists (object_wait); yield needs syscall wrapper |
| Task scheduling | Priority-based round-robin scheduler | ✅ Exceeds requirement |
| Context switching | PendSV-based, full register save/restore | ✅ Exists |
| Global work flag / Pender | `EventSignaler::signal()` (ISR-safe) | ✅ Exists |
| Interrupt notification | `InterruptObject` + signal→wait pipeline | ✅ Exists |
| AtomicBool operations | `core::sync::atomic::AtomicBool` | ✅ Exists |
| Critical sections | PRIMASK-based `BareSpinLock` | ✅ Exists |
| Timer / deadline | SysTick clock + `TimerQueue` + deadline on all waits | ✅ Exists |
| WFI / idle | `Arch::idle()` → `cortex_m::asm::wfi()` | ✅ Exists |
| Memory protection | MPU support (ARMv7-M and ARMv8-M) | ✅ Bonus |
| Process isolation | Per-process memory config | ✅ Bonus |
| Mutex with deadlines | `Mutex::lock_until(deadline)` | ✅ Bonus |

---

## Implementation Plan

### Phase 1: Minimal Executor (1–2 days)
1. Add `SysCallId::GetTime` syscall
2. Add `SysCallId::SleepUntil` syscall (optional — can use `object_wait`)
3. Implement `PwExecutor` using `embassy_executor::raw`
4. Pender uses `EventSignaler::signal()` via a dedicated handle
5. Executor loop: `poll()` → check `SIGNAL_WORK` → `object_wait` on wake handle

### Phase 2: Efficient Reactor (2–3 days)
1. Build `Reactor` using `WaitGroup` for multiplexed I/O
2. Implement `ChannelTransactFuture` for async IPC
3. Implement `InterruptFuture` for hardware interrupt wakeup
4. Implement `TimerFuture` using `object_wait` deadlines

### Phase 3: Wire Async IPC (1–2 days)
1. Implement `channel_async_transact` kernel handler (ABI already defined)
2. Implement `channel_async_cancel` kernel handler
3. Build zero-copy async channel futures

### Phase 4: Integration & Testing (2–3 days)
1. Run executor in a pw_kernel userspace process
2. Test with multiple concurrent async tasks
3. Test interrupt-driven I/O
4. Benchmark: task switch latency, interrupt-to-wake latency
5. Power: verify WFI idle works correctly

---

## Conclusion

**Yes — pw_kernel provides everything needed to build a Rust async executor.**

The kernel already has:
- Signal-based I/O readiness (`object_wait` + `Signals`)
- Multiplexed waiting (`WaitGroup`)
- ISR-safe wakeup (`EventSignaler`)
- Deadline-aware blocking (every wait syscall accepts a deadline)
- Hardware interrupt integration (`InterruptObject`)
- Priority-based preemptive scheduling
- Process isolation and MPU support

The only gaps are two trivial syscall additions (`get_time`, `sleep_until`)
that expose existing kernel-internal functionality to userspace. The executor
itself is ~100 lines of Rust wrapping `embassy_executor::raw`, with the
heavy lifting done by pw_kernel's existing object/signal/wait infrastructure.

pw_kernel is significantly more capable than the "bare minimum OS" originally
described — it provides a full RTOS with IPC, synchronization, memory
protection, and priority scheduling, all in Rust with no C++ dependency.