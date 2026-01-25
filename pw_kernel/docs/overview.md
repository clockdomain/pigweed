# pw_kernel: A Modern Microkernel for Embedded Systems

## What is a Microkernel?

A **microkernel** is an operating system architecture that minimizes the code running in privileged (kernel) mode. Unlike monolithic kernels where device drivers, file systems, and services run in kernel space, a microkernel keeps only the essential functions in the kernel:

- **Scheduling** - Deciding which thread runs next
- **Memory management** - Address space isolation and protection
- **IPC (Inter-Process Communication)** - Message passing between processes
- **Basic exception handling** - Traps, interrupts, system calls

Everything else—drivers, protocols, services—runs in user space as isolated processes.

```
┌─────────────────────────────────────────────────────────────────┐
│                      User Space                                  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐        │
│  │  Driver  │  │  Driver  │  │   App    │  │ Service  │        │
│  │  (GPIO)  │  │  (UART)  │  │          │  │ (Crypto) │        │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘        │
│       │             │             │             │               │
│       └─────────────┴──────┬──────┴─────────────┘               │
│                            │ IPC                                │
├────────────────────────────┼────────────────────────────────────┤
│                     Kernel │ Space                              │
│  ┌─────────────────────────┴─────────────────────────────────┐  │
│  │                    pw_kernel                               │  │
│  │  ┌───────────┐  ┌───────────┐  ┌───────────┐             │  │
│  │  │ Scheduler │  │    IPC    │  │  Memory   │             │  │
│  │  │           │  │ Channels  │  │Protection │             │  │
│  │  └───────────┘  └───────────┘  └───────────┘             │  │
│  └───────────────────────────────────────────────────────────┘  │
│                            │                                    │
├────────────────────────────┼────────────────────────────────────┤
│                     Hardware                                    │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐        │
│  │   CPU    │  │   MPU    │  │  Timer   │  │   NVIC   │        │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘        │
└─────────────────────────────────────────────────────────────────┘
```

## pw_kernel as a Microkernel

`pw_kernel` follows microkernel principles adapted for **resource-constrained embedded systems** (microcontrollers without an MMU):

### Minimal Trusted Computing Base (TCB)

| Component | In Kernel | In User Space |
|-----------|:---------:|:-------------:|
| Thread scheduling | ✅ | |
| Memory protection (MPU/PMP) | ✅ | |
| IPC channels | ✅ | |
| System call interface | ✅ | |
| Device drivers | | ✅ |
| Application logic | | ✅ |
| Protocol stacks | | ✅ |

The kernel is ~10KB of code, minimizing the attack surface and simplifying security audits.

### Hardware-Enforced Isolation

Unlike traditional embedded RTOSes (FreeRTOS, Zephyr in flat mode), pw_kernel leverages hardware protection:

| Architecture | Protection Mechanism | Capability |
|--------------|---------------------|------------|
| ARM Cortex-M | Memory Protection Unit (MPU) | Region-based access control |
| RISC-V | Physical Memory Protection (PMP) | Region-based access control |

Each user process has:
- **Isolated memory regions** - Cannot access other processes' memory
- **Unprivileged execution** - Cannot execute privileged instructions
- **Controlled kernel access** - Only through system calls

### IPC-Centric Communication

Following microkernel philosophy, processes communicate via **message passing**, not shared memory (by default):

```rust
// User process: Initiator
let response = syscall::channel_transact(channel, &request)?;

// User process: Handler  
let request = syscall::channel_read(channel, &mut buffer)?;
syscall::channel_respond(channel, &response)?;
```

**Channel properties:**
- Synchronous request-response model
- Zero-copy where possible (shared buffer regions)
- Kernel mediates all communication
- Statically allocated at build time

## Channels: The IPC Primitive

A **Channel** is pw_kernel's primary inter-process communication mechanism. It provides a unidirectional, request-response connection between two asymmetric peers.

### Channel Roles

| Role | Description | Operations |
|------|-------------|------------|
| **Initiator** | Sends requests, receives responses | `channel_transact()`, `channel_async_transact()` |
| **Handler** | Receives requests, sends responses | `channel_read()`, `channel_respond()` |

### Transaction Flow

```
┌──────────────┐                           ┌──────────────┐
│   Initiator  │                           │   Handler    │
│   (Client)   │                           │   (Server)   │
└──────┬───────┘                           └──────┬───────┘
       │                                          │
       │  1. channel_transact(request)            │
       │─────────────────────────────────────────►│
       │         [blocks waiting]                 │
       │                                          │
       │                           2. READABLE signal raised
       │                                          │
       │                           3. channel_read(buffer)
       │◄─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─│
       │         [kernel copies data]             │
       │                                          │
       │                           4. Process request...
       │                                          │
       │                           5. channel_respond(response)
       │◄─────────────────────────────────────────│
       │         [kernel copies response]         │
       │                                          │
       │  6. Initiator unblocks with response     │
       │                                          │
```

### Key Design Principles

1. **One Transaction at a Time**
   - A channel supports at most one pending transaction
   - Simple state machine, predictable behavior
   - No need for message queues or buffering

2. **No Intermediate Kernel Buffers**
   - Data copies directly between initiator and handler buffers
   - Kernel only mediates, doesn't store
   - Memory efficient for constrained systems

3. **Static Allocation**
   - Channels are allocated at build time via configuration
   - No runtime allocation failures
   - Handles are pre-assigned to processes

4. **Signal-Based Notification**
   - `READABLE` - Data available to read / response ready
   - `WRITABLE` - Ready to start a new transaction
   - `ERROR` - Transaction error occurred
   - `USER` - Custom signal from peer

### Synchronous vs Asynchronous API

**Synchronous (blocking):**
```rust
// Initiator blocks until handler responds
let len = syscall::channel_transact(
    handle,
    &send_buffer,
    &mut recv_buffer,
    timeout
)?;
```

**Asynchronous (non-blocking):**
```rust
// Initiator starts transaction and continues
syscall::channel_async_transact(handle, &send_buffer)?;

// Later: wait for response
syscall::object_wait(handle, Signals::READABLE, timeout)?;

// Read the response
let len = syscall::channel_read_response(handle, &mut recv_buffer)?;
```

### Example: IPC Between Driver and Application

```rust
// ─────────────────────────────────────────────────────────
// UART Driver (Handler process)
// ─────────────────────────────────────────────────────────
fn uart_driver_main() {
    loop {
        // Wait for a request
        syscall::object_wait(handle::UART_CHANNEL, Signals::READABLE, Instant::MAX)?;
        
        // Read the request
        let len = syscall::channel_read(handle::UART_CHANNEL, 0, &mut buffer)?;
        
        // Process: write data to UART hardware
        uart_write(&buffer[..len]);
        
        // Respond with status
        let response = [0u8; 1]; // Success
        syscall::channel_respond(handle::UART_CHANNEL, &response)?;
    }
}

// ─────────────────────────────────────────────────────────
// Application (Initiator process)
// ─────────────────────────────────────────────────────────
fn app_main() {
    let message = b"Hello, UART!";
    let mut response = [0u8; 1];
    
    // Send message to UART driver, wait for acknowledgment
    syscall::channel_transact(
        handle::UART_CHANNEL,
        message,
        &mut response,
        Instant::MAX
    )?;
    
    if response[0] == 0 {
        // Success!
    }
}
```

### Wait Groups: Multiplexing Channels

For handlers serving multiple channels, **Wait Groups** allow waiting on multiple objects:

```rust
// Add channels to wait group
syscall::wait_group_add(wait_group, channel_a, Signals::READABLE, USER_DATA_A)?;
syscall::wait_group_add(wait_group, channel_b, Signals::READABLE, USER_DATA_B)?;

loop {
    // Wait for any channel to have data
    let (signals, user_data) = syscall::object_wait(wait_group, Signals::READABLE, Instant::MAX)?;
    
    match user_data {
        USER_DATA_A => handle_channel_a(),
        USER_DATA_B => handle_channel_b(),
        _ => {}
    }
}
```

This is similar to `epoll` on Linux or `kqueue` on BSD—efficient multiplexed I/O for embedded systems.

## Comparison with Other Architectures

### vs. Monolithic RTOS (FreeRTOS, Zephyr flat mode)

| Aspect | Monolithic RTOS | pw_kernel |
|--------|-----------------|-----------|
| Isolation | None (shared address space) | Hardware-enforced (MPU/PMP) |
| Driver bugs | Can crash entire system | Contained to driver process |
| Code size | Smaller | Slightly larger (protection overhead) |
| Latency | Lower | Slightly higher (context switch cost) |
| Security | Trust everything | Trust only kernel |

### vs. Traditional Microkernels (seL4, QNX, MINIX 3)

| Aspect | Traditional Microkernel | pw_kernel |
|--------|------------------------|-----------|
| Target | Servers, phones, safety-critical | Microcontrollers (Cortex-M, RISC-V) |
| Memory | MMU-based virtual memory | MPU/PMP physical protection |
| Resources | MB-GB RAM | KB-MB RAM |
| IPC | Async + Sync | Synchronous channels |
| Formal verification | seL4: proven | Not yet (future goal) |

### vs. Separation Kernels

| Aspect | Separation Kernel | pw_kernel |
|--------|-------------------|-----------|
| Use case | High-assurance systems | General embedded |
| Flexibility | Fixed partitions | Dynamic scheduling |
| Certification | DO-178C, CC EAL7 | Path towards certification |

## Microkernel Benefits for Embedded Systems

### 1. Fault Isolation

A bug in a driver cannot corrupt the kernel or other processes:

```
┌─────────────────────────────────────────────────┐
│ Traditional RTOS: Driver bug crashes everything │
│                                                 │
│   Driver ──── BUG! ────► Kernel ────► System 💥 │
└─────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────┐
│ pw_kernel: Driver bug is contained              │
│                                                 │
│   Driver ──── BUG! ────► HardFault             │
│                          │                      │
│                          ▼                      │
│                    Kernel detects,              │
│                    restarts driver              │
│                          │                      │
│                          ▼                      │
│                    System continues ✅          │
└─────────────────────────────────────────────────┘
```

### 2. Security Boundaries

Sensitive operations can be isolated:

```
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│  ┌─────────────┐        ┌─────────────┐                    │
│  │   Network   │        │   Crypto    │                    │
│  │   Driver    │◄──IPC──►│  Service   │                    │
│  │             │        │             │                    │
│  │ Can access: │        │ Can access: │                    │
│  │ - Ethernet  │        │ - Crypto HW │                    │
│  │ - DMA       │        │ - Key store │                    │
│  │             │        │             │                    │
│  │ Cannot:     │        │ Cannot:     │                    │
│  │ - Read keys │        │ - Network   │                    │
│  └─────────────┘        └─────────────┘                    │
│                                                             │
│  Hardware-enforced separation via MPU regions               │
└─────────────────────────────────────────────────────────────┘
```

### 3. Updatability

User-space components can be updated independently:

- Update a driver without touching the kernel
- A/B update partitions for reliability
- Smaller update payloads

### 4. Testability

Each component can be tested in isolation:

```rust
// Test driver behavior with mock IPC
#[test]
fn test_uart_driver_handles_framing_error() {
    let mock_channel = MockChannel::new();
    let driver = UartDriver::new(mock_channel);
    
    driver.inject_framing_error();
    
    assert!(driver.error_count() == 1);
    assert!(driver.is_operational()); // Recovered
}
```

## pw_kernel Design Choices

### Rust for Memory Safety

The kernel is written in Rust, eliminating entire classes of vulnerabilities:

- No buffer overflows
- No use-after-free
- No data races
- Enforced at compile time

### Static Allocation

No dynamic memory allocation in the kernel:

- Predictable memory usage
- No allocation failures at runtime
- No fragmentation
- Easier to analyze and certify

```rust
// All threads, channels, and resources are statically allocated
static THREAD_POOL: [Thread; MAX_THREADS] = [...];
static CHANNEL_POOL: [Channel; MAX_CHANNELS] = [...];
```

### Flexible Protection Modes

Configurable security/performance tradeoff:

| Mode | Protection | Overhead | Use Case |
|------|------------|----------|----------|
| **Protected** | Full MPU/PMP isolation | Higher | Security-critical |
| **Lightweight** | None (all kernel threads) | Minimal | Resource-constrained |

## Getting Started

See the [Quickstart Guide](QUICKSTART.md) for building and running pw_kernel.

## Further Reading

- [Design Document](../design.rst) - Detailed design philosophy
- [Guides](../guides.rst) - How-to guides for specific features
- [seL4 Whitepaper](https://sel4.systems/About/seL4-whitepaper.pdf) - Formal microkernel design
- [MINIX 3](https://www.minix3.org/) - Classic microkernel OS
- [QNX Neutrino](https://blackberry.qnx.com/en/software-solutions/embedded-software/qnx-neutrino-rtos) - Commercial microkernel RTOS

---

*pw_kernel brings microkernel principles to the embedded world, enabling secure, reliable, and maintainable firmware on resource-constrained devices.*
