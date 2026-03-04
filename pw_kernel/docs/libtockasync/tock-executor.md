# libtockasync — Specification

Async executor and future primitives for Rust applications running on Tock OS.

## Overview

`libtockasync` provides a minimal async runtime that adapts Tock's event-driven
upcall/subscribe syscall model into Rust `async`/`await`. It has two core
abstractions:

| Component | Role |
|---|---|
| `TockExecutor` | Single-threaded task executor (Embassy-based), yields to kernel when idle |
| `TockSubscribe` | `Future` that completes when a Tock kernel upcall fires |

Together they form a cooperative loop: tasks issue subscribe syscalls, suspend,
and resume when the kernel delivers upcall events.

---

## Module Structure

```
libtockasync/src/
├── lib.rs             Crate root — re-exports, null critical section, entry points
├── tock_executor.rs   TockExecutor — Embassy raw executor + Tock yield integration
└── future.rs          TockSubscribe — Future wrapping Tock SUBSCRIBE/ALLOW syscalls
```

### lib.rs — Crate Root

**Exports:** `TockSubscribe`, `TockExecutor`

**NullCriticalSection:**
Implements `critical_section::Impl` as a no-op. This is sound because Tock
userspace is single-threaded — the kernel only re-enters the app at explicit
yield points, so no mutual exclusion is needed.

**Entry points:**

```rust
/// Spawn the main task onto an already-created executor.
pub fn init<S>(spawner: Spawner, main: SpawnToken<S>);

/// Create executor, spawn main task, run forever.  Never returns.
pub fn start_async<S>(main: SpawnToken<S>) -> !;
```

`start_async` allocates a `TockExecutor` on the stack, transmutes it to
`'static` (safe because the function diverges), and enters `executor.run()`.

---

### tock_executor.rs — Executor

Derived from Embassy's RISC-V executor.

#### Struct

```rust
pub struct TockExecutor {
    inner: raw::Executor,      // Embassy raw executor
    not_send: PhantomData<*mut ()>,  // enforce !Send
}
```

#### Global pender

```rust
static SIGNAL_WORK_THREAD_MODE: AtomicBool;

#[export_name = "__pender"]
fn __pender(_context: *mut ()) {
    SIGNAL_WORK_THREAD_MODE.store(true, SeqCst);
}
```

Embassy calls `__pender` whenever a task becomes ready (e.g. a waker is invoked
from inside `kernel_upcall`).  The flag tells `poll()` to loop again instead of
yielding.

#### Methods

| Method | Signature | Description |
|---|---|---|
| `new` | `() -> Self` | Create executor with null pender context |
| `spawner` | `(&'static self) -> Spawner` | Get handle to spawn tasks |
| `poll` | `(&'static self)` | Run one polling round (see below) |
| `run` | `(&'static mut self, init: FnOnce(Spawner)) -> !` | Spawn via `init`, then `loop { poll() }` |

#### Poll loop detail

```
poll():
  1. inner.poll()                 — drive all ready tasks
  2. critical_section {
       if SIGNAL_WORK_THREAD_MODE:
         clear flag, return        — loop back to step 1
       else:
         yield1(1)                 — yield-wait to Tock kernel
     }
```

When there is no pending work, the app sleeps in `yield1`. The kernel will
wake it when an upcall is ready, at which point `kernel_upcall` runs, sets
the work flag via the waker → `__pender` path, and `poll()` resumes.

#### yield1 — platform-dependent

| Target | Implementation |
|---|---|
| `riscv32` | Inline `asm!("ecall", ...)` with full register clobber list (RISC-V ABI) |
| Other (test) | `libtock_unittest::fake::Syscalls::yield_wait()` |

---

### future.rs — TockSubscribe

#### Struct

```rust
pub struct TockSubscribe {
    result: Cell<Option<(u32, u32, u32)>>,  // upcall arguments
    waker:  Cell<Option<Waker>>,            // task waker
    error:  Option<ErrorCode>,              // syscall error, if any
}
```

Implements `Future<Output = Result<(u32, u32, u32), ErrorCode>>`.

#### Subscribe constructors

All constructors return `Pin<Box<TockSubscribe>>`.  Pinning is mandatory
because a raw pointer to the struct is passed to the kernel as upcall data.

| Constructor | Syscalls issued (in order) | Buffer args |
|---|---|---|
| `subscribe::<S>(driver, sub)` | SUBSCRIBE | — |
| `subscribe_allow_rw::<S,C>(driver, sub, buf_num, &mut [u8])` | ALLOW_RW → SUBSCRIBE | one mutable buffer |
| `subscribe_allow_ro::<S,C>(driver, sub, buf_num, &[u8])` | ALLOW_RO → SUBSCRIBE | one immutable buffer |
| `subscribe_allow_ro_rw::<S,C>(driver, sub, ro_num, &[u8], rw_num, &mut [u8])` | ALLOW_RO → ALLOW_RW → SUBSCRIBE | one of each |

Each constructor:
1. Allocates and pins a `TockSubscribe`
2. Computes `upcall_fcn` (pointer to `kernel_upcall::<S>`) and `upcall_data`
   (pointer to the pinned struct)
3. Issues ALLOW syscall(s) via `S::syscall4`, checks return variant
4. If a non-zero buffer is returned from ALLOW, calls `C::returned_nonzero_buffer`
5. Issues SUBSCRIBE syscall, checks return variant
6. On any failure: stores `ErrorCode` — the future resolves to `Err` on next poll

#### Finishing and cancelling

```rust
/// Convert Pin<Box<TockSubscribe>> into impl Future (for .await).
pub fn subscribe_finish(f: Pin<Box<TockSubscribe>>)
    -> impl Future<Output = Result<(u32, u32, u32), ErrorCode>>;

/// Set error so the future can be safely dropped without panic.
pub fn cancel(&mut self);
```

#### kernel_upcall — the C callback

```rust
extern "C" fn kernel_upcall<S: Syscalls>(
    arg0: u32, arg1: u32, arg2: u32, data: Register,
)
```

Called by the Tock kernel when the subscribed event fires:
1. Creates `ExitOnDrop<S>` RAII guard (manages syscall exit context)
2. Casts `data` back to `*mut TockSubscribe`
3. Stores `(arg0, arg1, arg2)` into `result`
4. Takes and wakes the stored `Waker` (triggers `__pender` → work flag)
5. Forgets the `ExitOnDrop` guard (normal exit path)

#### Future::poll

```
if error is set       → Ready(Err(error))
if result is set      → Ready(Ok(result))
otherwise             → store waker, return Pending
```

#### Drop guard

Panics if dropped while `result` is `None` and `error` is `None`.  This
prevents the kernel from calling back into freed memory.  Callers must
either `.await` the future to completion or call `cancel()` before dropping.

---

## Tock OS Dependencies

### Syscall classes used

| Class | Constant | Purpose |
|---|---|---|
| Subscribe | `syscall_class::SUBSCRIBE` | Register upcall callback for a driver event |
| Allow Read-Write | `syscall_class::ALLOW_RW` | Share a mutable buffer with a kernel driver |
| Allow Read-Only | `syscall_class::ALLOW_RO` | Share an immutable buffer with a kernel driver |
| Yield | yield-wait (class 0, id 1) | Sleep until the kernel delivers an upcall |

### Crate dependencies from the Tock ecosystem

| Crate | Used for |
|---|---|
| `libtock_platform` | `Syscalls` trait, `Register`, `ReturnVariant`, `ErrorCode`, `ExitOnDrop`, `allow_ro`/`allow_rw` config traits |
| `libtock_runtime` | Tock process entry point and startup |
| `libtock` | High-level Tock bindings (transitive) |
| `libtock_console` | Console I/O (transitive/debug) |
| `libtock_debug_panic` | Panic handler for Tock |
| `libtock_unittest` | Fake syscalls for host-side testing (non-riscv32 only) |

### External dependencies

| Crate | Used for |
|---|---|
| `embassy-executor` | `raw::Executor`, `Spawner`, `SpawnToken`; `arch-riscv32` feature on target |
| `critical-section` | `set_impl!` macro, `with()` for interrupt-safe flag access |
| `portable-atomic` | `AtomicBool` (works on targets without native atomics) |
| `embedded-alloc` | Heap allocator (riscv32 only — needed for `Box` in `TockSubscribe`) |

---

## Execution Model

```
                         ┌──────────────────────┐
                         │    start_async()      │
                         │  create TockExecutor  │
                         │  spawn main task      │
                         └──────────┬─────────────┘
                                    │
                         ┌──────────▼─────────────┐
                    ┌───►│   executor.poll()       │
                    │    │   inner.poll() runs all  │
                    │    │   ready tasks             │
                    │    └──────────┬─────────────┘
                    │               │
                    │    ┌──────────▼─────────────┐
                    │    │  Tasks call              │
                    │    │  TockSubscribe::subscribe│
                    │    │  → ALLOW + SUBSCRIBE     │
                    │    │  → return Pending         │
                    │    └──────────┬─────────────┘
                    │               │
                    │    ┌──────────▼─────────────┐
                    │    │  No work flag set?       │
                    │    │  yield1(1) — sleep       │◄── app is idle
                    │    └──────────┬─────────────┘
                    │               │
                    │    ┌──────────▼─────────────┐
                    │    │  Kernel fires upcall     │
                    │    │  kernel_upcall() runs:   │
                    │    │    store result           │
                    │    │    wake task              │
                    │    │    → __pender sets flag   │
                    │    └──────────┬─────────────┘
                    │               │
                    └───────────────┘
```

**Invariants:**
- Exactly one executor instance exists (enforced by `!Send` + diverging entry)
- Tasks run cooperatively — no preemption within userspace
- A `TockSubscribe` must not be dropped before completion or cancellation
- All kernel interaction goes through the `Syscalls` trait (mockable for tests)

---

## Downstream Usage Pattern

Typical consumer code (from `runtime/userspace/syscall/`):

```rust
// 1. Set up command via Tock COMMAND syscall (not shown — done separately)

// 2. Create subscribe future, optionally sharing buffers
let sub = TockSubscribe::subscribe_allow_rw::<S, DefaultConfig>(
    DRIVER_NUM, subscribe::DONE, buffer_num::DATA, &mut buf,
);

// 3. Issue the triggering command
S::command(DRIVER_NUM, command::START, 0, 0)?;

// 4. Await the upcall
let (status, arg1, arg2) = TockSubscribe::subscribe_finish(sub).await?;
```

Consumers include: `mctp`, `mailbox`, `dma`, `flash`, `logging`, `doe`,
`mbox_sram`, `mcu_mbox`, `pldm-lib` (timer), and example/user apps.
