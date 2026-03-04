# pw_kernel Async Runtime — Design Document

## Glossary

| Term | Definition |
|------|-----------|
| **Executor** | The task scheduler. Maintains a queue of async tasks, polls those that are ready, and idles when none are. Wraps Embassy's `raw::Executor`. |
| **Reactor** | The I/O bridge. Provides futures that convert kernel events (signals, interrupts) into Rust `async`/`await`. Separate crate from the executor. |
| **Task** | A `'static` async function registered with the executor via `Spawner`. Defined with `#[embassy_executor::task]`. |
| **Spawner** | Embassy handle for dynamically adding tasks to a running executor. Copyable — tasks can spawn other tasks. |
| **Pender** | Callback (`__pender`) invoked by Embassy whenever a waker fires. Sets `SIGNAL_WORK` flag so the executor knows to poll again. Can be called from any context. |
| **Waker** | Rust's built-in mechanism for a future to say "poll me again." Embassy manages wakers internally; the pender is the bridge to the executor's main loop. |
| **Signal** | A 32-bit bitflag on a kernel object. Represents readiness state (e.g., `READABLE`, `INTERRUPT_A`). Checked via `object_wait`. |
| **Handle** | A `u32` identifying a kernel object (channel, interrupt, wait group) in userspace. Assigned at compile time from `system.json5`. |
| **`object_wait`** | Kernel syscall that blocks until specified signals become pending on an object, or a deadline expires. The core primitive for both idle blocking and I/O polling. |
| **WaitGroup** | Kernel object that aggregates multiple objects. `object_wait` on a WaitGroup returns when *any* member is signaled, with `user_data` identifying which one. Like `epoll`. |
| **Idle closure** | Caller-provided `impl Fn()` passed to `Executor::run()`. Called when no tasks are ready. Can spin, block via `object_wait`, or sleep via `wfi`. |

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│              Userspace Process                    │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │          Application Tasks                 │  │
│  │  #[task] async fn sensor_read() { ... }    │  │
│  │  #[task] async fn handle_ipc() { ... }     │  │
│  └──────────────┬─────────────────────────────┘  │
│                 │ .await                          │
│  ┌──────────────┴─────────────────────────────┐  │
│  │  Reactor (pw_kernel/lib/reactor)           │  │
│  │  ┌─────────────────┐ ┌──────────────────┐  │  │
│  │  │ ObjectWaitFuture│ │ InterruptFuture  │  │  │
│  │  │ (any signals)   │ │ (IRQ + auto-ack) │  │  │
│  │  └────────┬────────┘ └────────┬─────────┘  │  │
│  └───────────┼───────────────────┼────────────┘  │
│              │ syscalls           │               │
│  ┌───────────┴───────────────────┴────────────┐  │
│  │  Executor (pw_kernel/lib/executor)         │  │
│  │  ┌──────────────┐  ┌───────────────────┐   │  │
│  │  │ Embassy raw  │  │ SIGNAL_WORK flag  │   │  │
│  │  │ poll + spawn │  │ (AtomicBool)      │   │  │
│  │  └──────────────┘  └───────────────────┘   │  │
│  │           │ idle()                          │  │
│  └───────────┼─────────────────────────────────┘  │
│              │ syscall                             │
├──────────────┼────────────────────────────────────┤
│         pw_kernel                                  │
│  Scheduler · Signals · object_wait · WaitGroup     │
│  Channels · InterruptObject · Timer · MPU          │
├────────────────────────────────────────────────────┤
│         ARM Cortex-M Hardware                      │
│  SVCall · PendSV · SysTick · NVIC · MPU            │
└────────────────────────────────────────────────────┘
```

---

## Theory of Operation

### Main loop

The executor runs in a single pw_kernel userspace thread. Its loop is:

```
loop {
    1. poll()        — Embassy drives all ready tasks
    2. check flag    — did any waker fire during poll?
       yes → go to 1
       no  → idle()  — caller-provided blocking strategy
}
```

### How a waker propagates

When an async task wakes another task (e.g., via a shared channel or completion flag):

```
Task A calls waker.wake()
  → Embassy marks Task B as ready
  → Embassy calls __pender()
  → __pender sets SIGNAL_WORK = true
  → Executor main loop sees flag, calls poll()
  → Embassy polls Task B
```

This happens synchronously within the same `poll()` call if Task A wakes Task B during its own poll. The flag ensures the executor loops back even if the wake happens after the current poll pass.

### How an I/O future works

When a task awaits a kernel event (interrupt, channel readable):

```
Task calls reactor::wait_interrupt(handle, signals).await
  → Future::poll() calls object_wait(handle, signals, Instant::MIN)
  → Kernel checks: are signals pending?
     YES → return Ok(WaitReturn) → future completes → task resumes
     NO  → return Err(DeadlineExceeded)
           → future calls cx.waker().wake_by_ref()
           → returns Poll::Pending
           → executor re-polls on next iteration
```

The non-blocking `object_wait(Instant::MIN)` acts as a readiness check. The `wake_by_ref()` self-wake ensures the executor re-polls this future. This is correct but busy-polls — the WaitGroup optimization (below) eliminates this.

### Idle strategies

The executor's `idle` parameter controls what happens when no tasks are ready:

| Strategy | Code | When to use |
|----------|------|-------------|
| **Spin** | `\|\| core::hint::spin_loop()` | Testing, tasks that self-wake (e.g., `yield_once`) |
| **Block on object** | `\|\| object_wait(handle, Signals::USER, Instant::MAX)` | Production — sleeps until external event signals the handle |
| **Block on WaitGroup** | `\|\| reactor.wait_for_events(Instant::MAX)` | Future optimization — sleeps until any registered I/O source fires |

---

## Crate Structure

```
pw_kernel/lib/
├── executor/          # Task scheduling (no kernel dependency)
│   ├── lib.rs         # Executor, Spawner re-export, YieldOnce, __pender
│   └── BUILD.bazel    # deps: embassy-executor
│
└── reactor/           # I/O futures (depends on pw_kernel userspace)
    ├── lib.rs         # ObjectWaitFuture, InterruptFuture
    └── BUILD.bazel    # deps: pw_kernel/userspace, pw_status
```

The split is intentional: the executor is OS-agnostic (only depends on Embassy), while the reactor is pw_kernel-specific (uses syscalls). This means:
- The executor can be tested on host with no kernel
- The reactor can be swapped for a different OS backend
- Neither crate depends on the other

---

## Key Types

### Executor crate (`pw_kernel/lib/executor`)

| Type | Role |
|------|------|
| `Executor` | Wraps `embassy_executor::raw::Executor`. Owns the poll loop and idle strategy. |
| `Spawner` | Re-exported from Embassy. Handle for spawning tasks. `Copy` + `Send`. |
| `YieldOnce` | Utility future — yields once then completes. Self-wakes. |
| `SIGNAL_WORK` | Global `AtomicBool`. Set by pender, cleared by executor each iteration. |

### Reactor crate (`pw_kernel/lib/reactor`)

| Type | Role |
|------|------|
| `ObjectWaitFuture` | Future that completes when signals become pending on a kernel object. |
| `InterruptFuture` | Future that completes when an interrupt fires. Auto-acknowledges the interrupt. |

---

## Syscalls Used

| Syscall | Used by | Purpose |
|---------|---------|---------|
| `object_wait(handle, signals, deadline)` | Reactor futures, idle closure | Non-blocking readiness check (`Instant::MIN`) or blocking wait (`Instant::MAX`) |
| `interrupt_ack(handle, signals)` | `InterruptFuture` | Clear and re-enable hardware interrupt after handling |
| `wait_group_add(wg, object, signals, user_data)` | Future: WaitGroup reactor | Register object for multiplexed waiting |
| `wait_group_remove(wg, object)` | Future: WaitGroup reactor | Deregister object |
| `channel_read(handle, offset, buf)` | Application tasks | Read from IPC channel after `READABLE` signal |
| `channel_respond(handle, buf)` | Application tasks | Respond to IPC request |

---

## Future Work

### WaitGroup-based reactor (eliminates busy-polling)

The current reactor futures self-wake and re-poll via non-blocking `object_wait`. This works but burns CPU when I/O is slow. The optimization:

1. A `Reactor` struct holds a WaitGroup handle and a waker-per-slot table
2. I/O futures register their handle + waker with the reactor instead of self-waking
3. The executor's idle closure calls `reactor.wait_for_events()` which does a single blocking `object_wait` on the WaitGroup
4. When any member fires, the reactor wakes only the corresponding task

This turns O(N) non-blocking polls per iteration into O(1) blocking wait.

### Async IPC (`channel_async_transact`)

The syscall ABI is defined in `syscall_defs.rs` but the kernel handler is not yet wired. Once implemented, this enables zero-copy async channel transactions — a future initiates the transaction non-blockingly and completes when the response arrives.

### Timer future

`object_wait` already accepts a deadline. A timer future is trivial:
```rust
async fn sleep_until(deadline: Instant) {
    // Wait on any handle with impossible signals — just use the deadline.
    // Or: wait on a dedicated timer object if one is added.
}
```

Currently blocked on `Clock::now()` returning a placeholder (no `get_time` syscall yet).
