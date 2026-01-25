# Pigweed Kernel IPC Subsystem Architecture

## Executive Summary

The Pigweed kernel IPC (Inter-Process Communication) subsystem provides a synchronous, request-response channel mechanism between user-space processes. It is designed for zero-copy transfers where kernel buffers are avoided by copying directly between process address spaces during syscalls.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Core Components](#2-core-components)
3. [Data Structures](#3-data-structures)
4. [System Call Interface](#4-system-call-interface)
5. [Signal Semantics](#5-signal-semantics)
6. [Code Generation Pipeline](#6-code-generation-pipeline)
7. [Transaction Flow](#7-transaction-flow)
8. [Component Interdependencies](#8-component-interdependencies)
9. [File Reference Map](#9-file-reference-map)
10. [Known Issues](#10-known-issues)

---

## 1. Overview

### Design Philosophy

The IPC subsystem follows these principles:
- **Asymmetric Roles**: Channels have distinct initiator and handler endpoints
- **Synchronous Semantics**: Initiator blocks until handler responds
- **Zero Kernel Buffers**: Data copied directly between process address spaces
- **Static Allocation**: All channel objects allocated at compile time via code generation
- **Signal-Based Coordination**: Uses kernel object signal mechanism for synchronization

### Channel Model

```
┌─────────────────┐                     ┌─────────────────┐
│  Initiator      │                     │    Handler      │
│  Process        │                     │    Process      │
│                 │                     │                 │
│  ┌───────────┐  │    Transaction      │  ┌───────────┐  │
│  │ Initiator │◄─┼─────────────────────┼──┤ Handler   │  │
│  │ Object    │  │    Send Buffer      │  │ Object    │  │
│  │           │──┼─────────────────────┼─►│           │  │
│  │           │◄─┼─────────────────────┼──┤           │  │
│  └───────────┘  │    Recv Buffer      │  └───────────┘  │
└─────────────────┘                     └─────────────────┘
```

---

## 2. Core Components

### 2.1 ChannelInitiatorObject

**Location**: `pw_kernel/kernel/object/channel.rs:84-160`

The initiator endpoint that can start transactions:

```rust
pub struct ChannelInitiatorObject<K: Kernel> {
    base: ObjectBase<K>,                                    // Signal/wait infrastructure
    handler: ForeignRc<K::AtomicUsize, ChannelHandlerObject<K>>, // Reference to handler
}
```

**Responsibilities**:
- Initiates transactions via `channel_transact()`
- Holds reference to its paired handler object
- Blocks until handler responds
- Manages initiator-side signal state

### 2.2 ChannelHandlerObject

**Location**: `pw_kernel/kernel/object/channel.rs:29-82`

The handler endpoint that receives and responds to transactions:

```rust
pub struct ChannelHandlerObject<K: Kernel> {
    base: ObjectBase<K>,                              // Signal/wait infrastructure
    active_transaction: Mutex<K, Option<Transaction<K>>>, // Current transaction state
}
```

**Responsibilities**:
- Stores active transaction state
- Provides `channel_read()` to read initiator's send buffer
- Provides `channel_respond()` to write response and complete transaction
- Manages handler-side signal state

### 2.3 Transaction

**Location**: `pw_kernel/kernel/object/channel.rs:23-27`

Internal structure holding transaction state:

```rust
struct Transaction<K: Kernel> {
    send_buffer: SyscallBuffer,    // Initiator's send data (read-only)
    recv_buffer: SyscallBuffer,    // Initiator's receive buffer (write)
    initiator: ForeignRc<K::AtomicUsize, ChannelInitiatorObject<K>>, // Back-reference
}
```

### 2.4 SyscallBuffer

**Location**: `pw_kernel/kernel/object/buffer.rs`

Memory buffer abstraction for cross-process data transfer:

```rust
pub struct SyscallBuffer {
    addr: NonNull<u8>,           // Virtual address in process space
    size: usize,                 // Buffer size
    access_type: MemoryRegionType, // Read-only or read-write
}
```

**Key Methods**:
- `new_in_current_process()` - Creates buffer with access validation
- `copy_into()` - Copies data to another buffer (cross-process safe)
- `truncate()` - Reduces buffer size (used for response length)

### 2.5 ObjectBase

**Location**: `pw_kernel/kernel/object.rs:175-188`

Common infrastructure for all kernel objects:

```rust
pub struct ObjectBase<K: Kernel> {
    state: SpinLock<K, ObjectBaseState<K>>,
}

pub struct ObjectBaseState<K: Kernel> {
    active_signals: Signals,                          // Current signal state
    waiters: RandomAccessForeignList<ObjectWaiter<K>>, // Threads waiting on signals
}
```

**Key Methods**:
- `wait_until()` - Block until signals match mask or deadline
- `signal()` - Set signals and wake waiting threads

---

## 3. Data Structures

### 3.1 Signals

**Location**: `pw_kernel/syscall/syscall_defs.rs:269-310`

Bitfield for object state signaling:

```rust
pub struct Signals(u32);

bitflags! {
    impl Signals: u32 {
        const READABLE  = 1 << 0;   // Data available to read
        const WRITEABLE = 1 << 1;   // Ready for writing
        const ERROR     = 1 << 2;   // Error condition
        const USER      = 1 << 15;  // User-defined signal
        // Bits 16-31: Interrupt signals (not used for IPC)
    }
}
```

### 3.2 Handle Table

**Location**: `pw_kernel/kernel/object.rs:134-171`

Maps u32 handles to kernel objects per-process:

```rust
pub trait ObjectTable<K: Kernel> {
    fn get_object(&self, kernel: K, handle: u32) 
        -> Option<ForeignRc<K::AtomicUsize, dyn KernelObject<K>>>;
}
```

Handles are compile-time constants generated into app-specific `handle` modules.

---

## 4. System Call Interface

### 4.1 User-Space API

**Location**: `pw_kernel/userspace/syscall.rs`

#### channel_transact (Initiator)
```rust
pub fn channel_transact(
    object_handle: u32,
    send_data: &[u8],
    recv_data: &mut [u8],
    deadline: Instant,
) -> Result<usize>
```
Sends data to handler and blocks until response. Returns response length.

#### channel_read (Handler)
```rust
pub fn channel_read(
    object_handle: u32, 
    offset: usize, 
    buffer: &mut [u8]
) -> Result<usize>
```
Reads data from initiator's send buffer. Can be called multiple times with offset.

#### channel_respond (Handler)
```rust
pub fn channel_respond(
    object_handle: u32, 
    buffer: &[u8]
) -> Result<()>
```
Sends response to initiator, completing the transaction.

#### object_wait
```rust
pub fn object_wait(
    object_handle: u32, 
    signal_mask: Signals, 
    deadline: Instant
) -> Result<Signals>
```
Waits for any of the specified signals on an object.

### 4.2 Kernel Syscall Handlers

**Location**: `pw_kernel/kernel/syscall.rs:105-180`

```
┌─────────────────────────────────────────────────────────────────┐
│                      Syscall Dispatch                           │
├─────────────────────────────────────────────────────────────────┤
│  SysCallId::ChannelTransact  → handle_channel_transact()        │
│  SysCallId::ChannelRead      → handle_channel_read()            │
│  SysCallId::ChannelRespond   → handle_channel_respond()         │
│  SysCallId::ObjectWait       → handle_object_wait()             │
└─────────────────────────────────────────────────────────────────┘
```

Each handler:
1. Extracts arguments from syscall args
2. Looks up object handle in current process's table
3. Creates `SyscallBuffer` with access validation
4. Calls the object's trait method

---

## 5. Signal Semantics

### 5.1 Initiator Signals

| Signal | Meaning | Set When | Cleared When |
|--------|---------|----------|--------------|
| `READABLE` | Response available | Handler calls `channel_respond()` | Transaction started |
| `WRITEABLE` | Ready for new transaction | Transaction completes | Transaction in progress |
| `ERROR` | Transaction error | Error occurs | Transaction started |

### 5.2 Handler Signals

| Signal | Meaning | Set When | Cleared When |
|--------|---------|----------|--------------|
| `READABLE` | Transaction pending | Initiator calls `channel_transact()` | Handler calls `channel_respond()` |
| `WRITEABLE` | Transaction pending | Initiator calls `channel_transact()` | Handler calls `channel_respond()` |

---

## 6. Code Generation Pipeline

### 6.1 System Configuration

**Location**: `pw_kernel/tooling/system_generator/system_config.rs`

Configuration parsed from `system.json5`:

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChannelInitiatorConfig {
    pub name: String,
    pub handler_app: String,          // Target app name
    pub handler_object_name: String,  // Target object name
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChannelHandlerConfig {
    pub name: String,
}
```

Example `system.json5`:
```json5
{
    apps: [
        {
            name: "initiator",
            process: {
                name: "initiator process",
                objects: [{
                    name: "IPC",
                    type: "channel_initiator",
                    handler_app: "handler",
                    handler_object_name: "IPC",
                }],
                threads: [{ name: "initiator thread", stack_size_bytes: 2048 }],
            },
        },
        {
            name: "handler",
            process: {
                name: "handler process",
                objects: [{
                    name: "IPC",
                    type: "channel_handler",
                }],
                threads: [{ name: "handler thread", stack_size_bytes: 2048 }],
            },
        },
    ],
}
```

### 6.2 Code Generation Templates

**Location**: `pw_kernel/tooling/system_generator/templates/`

#### Handler Template (`objects/channel_handler.rs.jinja`)
```rust
let (object_{{app.name}}_{{object.name}}, object_{{app.name}}_{{object.name}}_handler) = {
    let handler = static_foreign_rc!(
        AtomicUsize, 
        ChannelHandlerObject<K>, 
        ChannelHandlerObject::new(kernel)
    );
    (upcast_foreign_rc!(handler.clone() => dyn KernelObject<K>), handler)
};
```

#### Initiator Template (`objects/channel_initiator.rs.jinja`)
```rust
let initiator = static_foreign_rc!(
    AtomicUsize,
    ChannelInitiatorObject<K>,
    ChannelInitiatorObject::new(
        object_{{object.handler_app}}_{{object.handler_object_name}}_handler
    )
);
upcast_foreign_rc!(initiator => dyn KernelObject<K>)
```

### 6.3 Generated Code Structure

The system template (`system.rs.jinja`) generates:

```rust
pub fn start() {
    // 1. Create handler objects first (needed by initiators)
    let (object_handler_ipc, object_handler_ipc_handler) = /* handler creation */;
    
    // 2. Create initiator objects (reference handler)
    let object_initiator_ipc = /* initiator creation with handler reference */;
    
    // 3. Build object tables for each process
    let object_table_initiator: ForeignBox<dyn ObjectTable<K>> = 
        static_foreign_box!([object_initiator_ipc]);
    let object_table_handler: ForeignBox<dyn ObjectTable<K>> = 
        static_foreign_box!([object_handler_ipc]);
    
    // 4. Create processes with their object tables
    let process_initiator = init_non_priv_process!(..., object_table_initiator);
    let process_handler = init_non_priv_process!(..., object_table_handler);
    
    // 5. Create and start threads
    let thread_initiator = init_non_priv_thread!(..., process_initiator, ...);
    let thread_handler = init_non_priv_thread!(..., process_handler, ...);
    start_thread(thread_initiator);
    start_thread(thread_handler);
}
```

### 6.4 App Handle Generation

**Location**: `pw_kernel/tooling/system_generator/templates/app.rs.jinja`

Generates handle constants for user-space:
```rust
// Generated into app_initiator crate
pub mod handle {
    pub const IPC: u32 = 0;  // Index into object table
}
```

---

## 7. Transaction Flow

### 7.1 Sequence Diagram

```
    Initiator                    Kernel                      Handler
    Process                                                  Process
       │                                                        │
       │  channel_transact(IPC, send, recv, deadline)          │
       ├─────────────────────────►│                             │
       │                          │ 1. Validate buffers         │
       │                          │ 2. Lock handler.active_transaction
       │                          │ 3. Store Transaction{send, recv, initiator}
       │                          │ 4. Clear initiator READABLE|WRITEABLE|ERROR
       │                          │ 5. Signal handler READABLE  │
       │                          │◄────────────────────────────┤ object_wait(IPC, READABLE)
       │                          │ 6. Wake handler thread      │
       │  [BLOCKED]               │                             │
       │                          │                             │
       │                          │◄────────────────────────────┤ channel_read(IPC, 0, buf)
       │                          │ 7. Copy send_buffer → buf   │
       │                          ├────────────────────────────►│
       │                          │                             │ [process data]
       │                          │◄────────────────────────────┤ channel_respond(IPC, response)
       │                          │ 8. Copy response → recv_buffer
       │                          │ 9. Clear handler READABLE|WRITEABLE
       │                          │ 10. Signal initiator READABLE
       │  [WOKEN]                 │                             │
       │◄─────────────────────────┤                             │
       │  returns recv_buffer.size()                            │
       │                                                        │
```

### 7.2 Detailed Steps

#### Initiator Side (`channel_transact`)

1. **Buffer Creation**: Create `SyscallBuffer` for send (read-only) and recv (read-write)
2. **Lock Transaction**: Acquire mutex on `handler.active_transaction`
3. **Check Availability**: Return `Error::Unavailable` if transaction already active
4. **Store Transaction**: Save send/recv buffers and initiator reference
5. **Clear Signals**: Remove `READABLE | WRITEABLE | ERROR` from initiator
6. **Signal Handler**: Set `READABLE` on handler, waking any waiting thread
7. **Wait for Response**: `object_wait(READABLE | ERROR, deadline)`
8. **Extract Result**: Read `recv_buffer.size()` from completed transaction
9. **Cleanup**: Clear `active_transaction`

#### Handler Side

1. **Wait for Work**: `object_wait(IPC, Signals::READABLE, Instant::MAX)`
2. **Read Data**: `channel_read(IPC, 0, &mut buffer)` - copies from initiator's send buffer
3. **Process**: Application-specific data processing
4. **Respond**: `channel_respond(IPC, &response)` - copies to initiator's recv buffer

---

## 8. Component Interdependencies

```
┌────────────────────────────────────────────────────────────────────────────┐
│                          BUILD-TIME DEPENDENCIES                           │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  system.json5 ──► system_generator ──► codegen.rs ──► target.rs           │
│                          │                                                 │
│                          ├──► app_initiator (handles)                      │
│                          └──► app_handler (handles)                        │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│                          RUNTIME DEPENDENCIES                              │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  ┌─────────────┐      ┌─────────────────┐      ┌──────────────────┐       │
│  │ userspace/  │      │ kernel/syscall  │      │ kernel/object/   │       │
│  │ syscall.rs  │ ───► │ .rs             │ ───► │ channel.rs       │       │
│  └─────────────┘      └─────────────────┘      └──────────────────┘       │
│         │                     │                        │                   │
│         │                     │                        ▼                   │
│         │                     │               ┌──────────────────┐        │
│         │                     └───────────────┤ kernel/object/   │        │
│         │                                     │ buffer.rs        │        │
│         │                                     └──────────────────┘        │
│         ▼                                              │                   │
│  ┌─────────────┐                                       ▼                   │
│  │ syscall_user│      ┌─────────────────┐     ┌──────────────────┐        │
│  │ (arch-      │ ───► │ syscall_defs.rs │ ◄── │ kernel/object.rs │        │
│  │  specific)  │      │ (Signals, IDs)  │     │ (ObjectBase)     │        │
│  └─────────────┘      └─────────────────┘     └──────────────────┘        │
│                                                        │                   │
│                                                        ▼                   │
│                                               ┌──────────────────┐        │
│                                               │ sync/event.rs    │        │
│                                               │ sync/mutex.rs    │        │
│                                               └──────────────────┘        │
└────────────────────────────────────────────────────────────────────────────┘
```

### Dependency Matrix

| Component | Depends On | Depended By |
|-----------|------------|-------------|
| `ChannelInitiatorObject` | `ChannelHandlerObject`, `ObjectBase`, `SyscallBuffer` | Syscall handler, Object table |
| `ChannelHandlerObject` | `ObjectBase`, `SyscallBuffer`, `Mutex`, `Transaction` | `ChannelInitiatorObject`, Syscall handler |
| `SyscallBuffer` | `MemoryRegionType`, Process memory validation | Both channel objects |
| `ObjectBase` | `SpinLock`, `Event`, `Signals` | All kernel objects |
| `system_generator` | `system_config.rs`, Jinja templates | Target `codegen.rs` |
| `syscall_user` | `syscall_defs` | User-space `syscall.rs` |

---

## 9. File Reference Map

### Kernel Implementation

| File | Purpose |
|------|---------|
| `pw_kernel/kernel/object/channel.rs` | `ChannelInitiatorObject`, `ChannelHandlerObject`, `Transaction` |
| `pw_kernel/kernel/object/buffer.rs` | `SyscallBuffer` - cross-process buffer abstraction |
| `pw_kernel/kernel/object.rs` | `KernelObject` trait, `ObjectBase`, `ObjectTable` |
| `pw_kernel/kernel/syscall.rs` | Syscall dispatch and handlers |

### Syscall Definitions

| File | Purpose |
|------|---------|
| `pw_kernel/syscall/syscall_defs.rs` | `SysCallId`, `Signals`, syscall documentation |
| `pw_kernel/syscall/syscall_user/arm_cortex_m.rs` | ARM syscall veneer (SVC instruction) |
| `pw_kernel/syscall/syscall_user/riscv.rs` | RISC-V syscall veneer (ECALL instruction) |

### User-Space API

| File | Purpose |
|------|---------|
| `pw_kernel/userspace/syscall.rs` | High-level syscall wrappers |
| `pw_kernel/userspace/time.rs` | `Instant` type for deadlines |

### Code Generation

| File | Purpose |
|------|---------|
| `pw_kernel/tooling/system_generator/lib.rs` | Generator main logic |
| `pw_kernel/tooling/system_generator/system_config.rs` | Config schema (`ChannelInitiatorConfig`, etc.) |
| `pw_kernel/tooling/system_generator/templates/system.rs.jinja` | Target codegen template |
| `pw_kernel/tooling/system_generator/templates/objects/channel_*.rs.jinja` | Object creation templates |
| `pw_kernel/tooling/system_generator/templates/app.rs.jinja` | App handle generation |

### Test Code

| File | Purpose |
|------|---------|
| `pw_kernel/tests/ipc/user/initiator.rs` | Example initiator implementation |
| `pw_kernel/tests/ipc/user/handler.rs` | Example handler implementation |
| `pw_kernel/target/ast1030/ipc/user/system.json5` | AST1030 IPC test configuration |

---

## 10. Known Issues

### 10.1 AST1030 IPC Test Failure

**Symptom**: Handler receives 0 bytes from `channel_read()` instead of expected data.

**Test Output**:
```
[INF] Initiator: sending char 97
[INF] IPC service starting
[INF] Handler: waiting for READABLE
[INF] Initiator: transact returned 0 bytes
[ERR] Received 0 bytes, 8 expected
```

**Analysis**:
- Both processes start correctly
- Handler is signaled (READABLE fires)
- `channel_read()` returns 0 bytes

**Possible Causes**:
1. **Cross-process buffer access**: `SyscallBuffer.copy_into()` may fail if the initiator's send buffer address is not accessible from the handler's context
2. **MPU configuration**: PMSAv7 MPU may block cross-process memory access
3. **Signal race**: Handler may be woken before transaction is fully stored

**Investigation Points**:
- Check `SyscallBuffer::copy_into()` return value
- Verify MPU regions allow kernel to access both process memory spaces
- Add debug logging to `channel_read()` implementation

### 10.2 Architecture Limitations

1. **Static Allocation Only**: Cannot create channels at runtime
2. **Single Transaction**: Only one active transaction per channel
3. **No Async Handler API**: Handler must use blocking `object_wait()`
4. **No Error Channel**: Handler cannot signal errors to initiator through the channel itself

---

## Appendix A: Glossary

| Term | Definition |
|------|------------|
| **Initiator** | The process endpoint that starts transactions |
| **Handler** | The process endpoint that receives and responds to transactions |
| **Transaction** | A single request-response exchange over a channel |
| **Signal** | A kernel notification bit (READABLE, WRITEABLE, ERROR, USER) |
| **Handle** | A u32 index into a process's object table |
| **Object Table** | Per-process array mapping handles to kernel objects |
| **SyscallBuffer** | Memory buffer validated for cross-process access |

---

## Appendix B: Quick Reference

### Starting a Transaction (Initiator)
```rust
use app_initiator::handle;
use userspace::syscall;
use userspace::time::Instant;

let mut send_buf = [1u8, 2, 3, 4];
let mut recv_buf = [0u8; 8];

let len = syscall::channel_transact(
    handle::IPC, 
    &send_buf, 
    &mut recv_buf, 
    Instant::MAX
)?;
// recv_buf[0..len] contains response
```

### Handling a Transaction (Handler)
```rust
use app_handler::handle;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

loop {
    // Wait for incoming transaction
    syscall::object_wait(handle::IPC, Signals::READABLE, Instant::MAX)?;
    
    // Read the request
    let mut buffer = [0u8; 64];
    let len = syscall::channel_read(handle::IPC, 0, &mut buffer)?;
    
    // Process and respond
    let response = process(&buffer[..len]);
    syscall::channel_respond(handle::IPC, &response)?;
}
```
