# pw_kernel: A Modern Microkernel for Embedded Systems

---

## What is a Microkernel?

**Minimal code in privileged mode**

Only essential functions in kernel:
- Scheduling
- Memory protection
- IPC (Inter-Process Communication)
- Exception handling

Everything else runs in **user space**

---

## Microkernel Architecture

```
┌─────────────────────────────────────────────────┐
│              User Space                          │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐   │
│  │ Driver │ │ Driver │ │  App   │ │Service │   │
│  └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘   │
│      └──────────┴─────┬────┴──────────┘        │
│                       │ IPC                     │
├───────────────────────┼─────────────────────────┤
│               Kernel  │                         │
│  ┌─────────┐ ┌───────┴───┐ ┌──────────┐        │
│  │Scheduler│ │    IPC    │ │  Memory  │        │
│  │         │ │ Channels  │ │Protection│        │
│  └─────────┘ └───────────┘ └──────────┘        │
└─────────────────────────────────────────────────┘
```

---

## pw_kernel: Microkernel for MCUs

**Target:** Resource-constrained embedded systems

| Feature | Implementation |
|---------|---------------|
| Memory Protection | ARM MPU, RISC-V PMP |
| Language | Rust (memory safe) |
| Allocation | Static only |
| Kernel Size | ~10KB |

---

## Minimal Trusted Computing Base

| Component | Kernel | User Space |
|-----------|:------:|:----------:|
| Scheduling | ✅ | |
| Memory protection | ✅ | |
| IPC channels | ✅ | |
| System calls | ✅ | |
| Device drivers | | ✅ |
| Applications | | ✅ |
| Protocol stacks | | ✅ |

---

## Hardware-Enforced Isolation

| Architecture | Protection | Capability |
|--------------|-----------|------------|
| ARM Cortex-M | MPU | Region-based access |
| RISC-V | PMP | Region-based access |

**Each process gets:**
- Isolated memory regions
- Unprivileged execution
- Controlled kernel access (syscalls only)

---

## IPC: Message Passing

```rust
// Initiator process
let response = syscall::channel_transact(
    channel, &request
)?;

// Handler process  
let request = syscall::channel_read(
    channel, &mut buffer
)?;
syscall::channel_respond(channel, &response)?;
```

**Properties:** Synchronous, zero-copy, kernel-mediated

---

## Signals: Event Notification

Every kernel object has a **signal mask** for async notification

| Signal | Bit | Meaning |
|--------|-----|---------|
| `READABLE` | 0 | Data available to read |
| `WRITEABLE` | 1 | Ready to accept data |
| `ERROR` | 2 | Error condition |
| `USER` | 15 | Custom out-of-band signal |
| `INTERRUPT_A..P` | 16-31 | Hardware interrupt (for IRQ objects) |

---

## Waiting on Signals

```rust
// Block until channel has data or timeout
let signals = syscall::object_wait(
    handle,
    Signals::READABLE,
    timeout
)?;

if signals.contains(Signals::READABLE) {
    // Data available - read it
    syscall::channel_read(handle, 0, &mut buf)?;
}
```

Similar to `poll()`/`select()` on Unix

---

## Signal Flow: Channel Example

1. Initiator calls `channel_transact()` → blocks
2. Kernel asserts `READABLE` on handler
3. Handler's `object_wait()` returns
4. Handler calls `channel_read()` to get data
5. Handler processes, calls `channel_respond()`
6. Kernel asserts `READABLE` on initiator
7. Initiator unblocks with response

```
Initiator                          Handler
    │                                  │
    │ channel_transact(request)        │
    │─────────────────────────────────►│
    │                                  │
    │    ┌─────────────────────────────┤
    │    │ READABLE asserted           │
    │    └─────────────────────────────┤
    │                                  │
    │                    object_wait() │◄─ wakes up
    │                    channel_read()│
    │                                  │
    │         channel_respond(reply)   │
    │◄─────────────────────────────────│
    │                                  │
    ├─────────────────────────────┐    │
    │ READABLE asserted           │    │
    ├─────────────────────────────┘    │
    │                                  │
```

---

## Fault Isolation

**Traditional RTOS:**
```
Driver Bug → Kernel Crash → System Dead 💥
```

**pw_kernel:**
```
Driver Bug → HardFault → Kernel Catches
                              ↓
                       Fault Contained
                       (other tasks unaffected)
```

Kernel and other processes remain intact

---

## Security Boundaries

```
┌─────────────┐        ┌─────────────┐
│   Network   │◄──IPC──►│   Crypto    │
│   Driver    │        │   Service   │
├─────────────┤        ├─────────────┤
│ Can access: │        │ Can access: │
│ • Ethernet  │        │ • Crypto HW │
│ • DMA       │        │ • Key store │
├─────────────┤        ├─────────────┤
│ Cannot:     │        │ Cannot:     │
│ • Read keys │        │ • Network   │
└─────────────┘        └─────────────┘
```

Hardware-enforced via MPU regions

## Comparison: vs Hubris (Oxide Computer)

| Aspect | Hubris | pw_kernel |
|--------|--------|-----------|
| Language | Rust | Rust |
| Target | Cortex-M | Cortex-M, RISC-V |
| Protection | MPU-based | MPU/PMP-based |
| Allocation | Static | Static |
| IPC | Synchronous | Synchronous channels |
| Scheduling | Preemptive priority | Preemptive priority |
| Build System | Cargo + xtask | Bazel |
| Ecosystem | Standalone | Pigweed integration |

**Both:** Rust microkernels for embedded, static allocation, MPU isolation

---

## Why Rust?

**Compile-time safety guarantees:**

- ✅ No buffer overflows
- ✅ No use-after-free  
- ✅ No data races
- ✅ No null pointer dereferences

**Zero-cost abstractions**

---

## Static Allocation

**No dynamic memory in kernel:**

```rust
// Everything allocated at build time
static THREADS: [Thread; MAX_THREADS] = [...];
static CHANNELS: [Channel; MAX_CHANNELS] = [...];
```

**Benefits:**
- Predictable memory
- No allocation failures
- No fragmentation
- Easier certification

---

## Flexible Protection Modes

| Mode | Protection | Overhead | Use Case |
|------|------------|----------|----------|
| **Protected** | Full MPU | Higher | Security-critical |
| **Lightweight** | None | Minimal | Resource-constrained |

Scale security to your needs

---

## Kernel Mode Thread Tests

**End-to-end tests** validating scheduler, sync primitives, and threading

```rust
// Two threads sharing a mutex-protected counter
fn thread_a<K: Kernel>(kernel: K, counter: &Mutex<K, u64>) {
    for _ in 0..3 {
        let mut guard = counter.lock();
        kernel::sleep_until(kernel, kernel.now() + Duration::from_secs(1));
        info!("Thread A: Incrementing counter");
        *guard = (*guard).saturating_add(1);
    }
}

fn thread_b<K: Kernel>(kernel: K, args: &ThreadArgs<K>) {
    for _ in 0..4 {
        let deadline = kernel.now() + Duration::from_millis(600);
        let Ok(guard) = args.counter.lock_until(deadline) else {
            info!("Thread B: Timeout");  // Expected behavior!
            continue;
        };
        info!("Thread B: Counter = {}", *guard);
    }
    args.done_signaler.signal();  // Notify completion
}
```

---

## Thread Test Structure

```
pw_kernel/tests/threads/kernel/
└── main.rs
    ├── AppState         - Static allocation for threads
    ├── thread_a()       - Holds mutex, sleeps, increments
    ├── thread_b()       - Tries lock with timeout
    └── Event signaling  - Synchronize test completion
```

Thread A sleeps for 1 second while holding the lock. Thread B only waits 600ms before giving up. Since 600ms < 1 second, Thread B will always time out—exactly what we want to test!

**What's tested:**
- Thread creation and scheduling
- Mutex lock/unlock with timeouts
- `sleep_until()` and `yield_timeslice()`
- Event signaling between threads
- Priority-based preemption

**Run:**
```bash
bazelisk test --config=k_qemu_ast1030 \
    //pw_kernel/target/ast1030/threads/kernel:threads_test
```

---

## Supported Targets

| Platform | Architecture | Status |
|----------|--------------|--------|
| Host (Linux/macOS) | x86_64 | ✅ Testing |
| QEMU MPS2-AN505 | Cortex-M33 | ✅ |
| QEMU AST1030 | Cortex-M4 | ✅ |
| QEMU virt | RISC-V 32 | ✅ |
| RP2350 | Cortex-M33 | ✅ Hardware |

---

## Building pw_kernel

```bash
# Clone Pigweed
git clone https://pigweed.googlesource.com/pigweed/pigweed
cd pigweed

# Build for ARM Cortex-M33 (QEMU)
bazelisk build --config=k_qemu_mps2_an505 \
    //pw_kernel/...

# Run tests
bazelisk test --config=k_qemu_mps2_an505 \
    //pw_kernel/...
```

---

## Key Takeaways

1. **Microkernel principles** adapted for MCUs
2. **Hardware isolation** via MPU/PMP
3. **Rust** for memory safety
4. **Static allocation** for predictability
5. **IPC-centric** communication model
6. **~10KB kernel** footprint

---

## Learn More

- **Quickstart:** `pw_kernel/docs/QUICKSTART.md`
- **Design:** `pw_kernel/design.rst`
- **Pigweed:** https://pigweed.dev

---

## Questions?

```
   ____  _                           _ 
  |  _ \(_) __ ___      _____  ___  __| |
  | |_) | |/ _` \ \ /\ / / _ \/ _ \/ _` |
  |  __/| | (_| |\ V  V /  __/  __/ (_| |
  |_|   |_|\__, | \_/\_/ \___|\___|\__,_|
           |___/                         
```

*pw_kernel - Microkernel for the embedded world*
