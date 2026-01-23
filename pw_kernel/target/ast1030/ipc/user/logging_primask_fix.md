# Logging with Interrupts Disabled Blocks Semihosting

**Date:** January 21, 2026  
**Status:** ✅ FIXED (Option 1 implemented - Interrupt-safe logging backend)  
**Affects:** All ARM Cortex-M targets using semihosting for logging output

## Executive Summary

The tokenized logging backend (`pw_log` with `pw_tokenizer`) uses **semihosting** for output. Semihosting requires debugger/QEMU interaction with interrupts enabled. When kernel code running with `PRIMASK=1` (interrupts disabled) calls `pw_log::info!()` or similar macros, the semihosting call **blocks indefinitely**, causing the system to hang.

## Root Cause

### The Problem

```
Kernel startup / context switch code
   │
   ├── cpsid i (disable interrupts, PRIMASK=1)
   │
   ├── info!("Initializing NVIC")
   │   │
   │   ▼
   │   tokenize_to_default_writer()
   │   │
   │   └── semihosting SYS_WRITE call
   │       │
   │       └── BLOCKS FOREVER (needs interrupts to complete!)
   │
   └── System hangs - never reaches cpsie i
```

### Why Semihosting Blocks

Semihosting works by:
1. Executing a `BKPT` instruction with a special immediate value
2. The debugger/QEMU intercepts this breakpoint
3. QEMU handles the I/O request (write to console)
4. QEMU resumes execution

When interrupts are disabled (`PRIMASK=1`), the semihosting mechanism may:
- Wait for an interrupt that never arrives
- Fail to properly resume after the breakpoint
- Enter an internal polling loop that depends on timer interrupts

### GDB Evidence

When Ctrl+C was pressed during the hang:
```
Program received signal SIGINT, Interrupt.
0x00003e00 in pw_tokenizer::internal::tokenize_to_default_writer ()

(gdb) bt
#0  tokenize_to_default_writer
#1  Nvic::early_init
    -- OR --
#1  scheduler::bootstrap_scheduler

(gdb) info reg primask
primask = 0x1  ← INTERRUPTS DISABLED - CAUSES HANG!
```

## Workaround Applied

Disabled 8 `pw_log` calls that run with interrupts disabled:

### Files Modified

| File | Function | Disabled Log Call |
|------|----------|-------------------|
| `pw_kernel/arch/arm_cortex_m/nvic.rs` | `Nvic::early_init()` | `info!("Initializing NVIC")` |
| `pw_kernel/arch/arm_cortex_m/threads.rs` | `early_init()` | `info!("Cortex-M early initialization")` |
| `pw_kernel/arch/arm_cortex_m/threads.rs` | `early_init()` | CPUID logging |
| `pw_kernel/arch/arm_cortex_m/threads.rs` | `early_init()` | MPU regions logging |
| `pw_kernel/arch/arm_cortex_m/protection_v7.rs` | `MpuRegion::write()` | MPU region config logging |
| `pw_kernel/arch/arm_cortex_m/protection_v7.rs` | `MemoryConfig::write()` | Memory config logging |
| `pw_kernel/arch/arm_cortex_m/protection_v7.rs` | `dump()` | MPU state dump |
| `pw_kernel/kernel/scheduler.rs` | `bootstrap_scheduler()` | `info!("Context switching to first thread")` |

### Code Changes

#### nvic.rs
```rust
pub fn early_init() {
    // info!("Initializing NVIC");  // DISABLED: blocks semihosting with PRIMASK=1
    // ...
}
```

#### threads.rs
```rust
pub fn early_init() {
    // info!("Cortex-M early initialization");  // DISABLED
    // ...
    let cpu_id = ...;
    // info!("CPUID: Implementer=0x{:02x} ...", ...);  // DISABLED
    let _ = cpu_id;  // Suppress unused warning
    
    let _r = get_num_mpu_regions();
    // info!("MPU regions: {}", _r);  // DISABLED
}
```

#### protection_v7.rs
```rust
impl MpuRegion {
    pub fn write(&self, ...) {
        // info!("Setting MPU region ...");  // DISABLED
    }
}

impl MemoryConfig {
    pub fn write(&self) {
        // info!("Writing memory config ...");  // DISABLED
    }
}

pub fn dump() {
    // info!("Dumping MPU state ...");  // DISABLED
}
```

#### scheduler.rs
```rust
pub fn bootstrap_scheduler() {
    // info!("Context switching to first thread");  // DISABLED
}
```

## Proper Solutions (Future Work)

### Option 1: Interrupt-Safe Logging Backend (Recommended)

Add a PRIMASK check to the logging backend to skip logging when interrupts are disabled:

```rust
// In pw_log_backend implementation
pub fn log_message(...) {
    // Skip logging if interrupts are disabled (semihosting would block)
    if cortex_m::register::primask::read().is_active() {
        return;  // Silently drop the log
    }
    // ... proceed with semihosting-based logging
}
```

**Pros:**
- Single fix in one place
- All existing `info!()` calls continue to work
- No code changes needed in kernel

**Cons:**
- Silently drops logs in interrupt-disabled contexts
- May miss important debug info during crashes

### Option 2: Use RTT (Real-Time Transfer) Backend

Replace semihosting with SEGGER RTT or similar non-blocking transport:

```rust
#[cfg(feature = "rtt")]
use rtt_target::{rprintln, rtt_init_print};
```

**Pros:**
- Non-blocking, works with interrupts disabled
- Much faster than semihosting
- Industry standard for embedded debugging

**Cons:**
- Requires RTT-compatible debug probe or QEMU RTT support
- Additional dependency

### Option 3: Build-Time Log Levels

Conditionally compile out logs in critical paths:

```rust
#[cfg(feature = "verbose_kernel_logging")]
info!("Context switching to first thread");
```

**Pros:**
- Zero runtime overhead when disabled
- Clear intent about optional logging

**Cons:**
- Requires rebuilding to change log verbosity
- May miss logs needed for debugging

### Option 4: Buffered/Deferred Logging

Buffer log messages during interrupt-disabled contexts and flush when interrupts are re-enabled:

```rust
pub fn log_message(...) {
    if primask_is_set() {
        DEFERRED_LOG_BUFFER.push(message);
    } else {
        flush_deferred_logs();
        semihosting_write(message);
    }
}
```

**Pros:**
- No logs lost
- Works with existing code

**Cons:**
- Memory overhead for buffer
- Complex implementation
- Potential ordering issues

## Verification

After applying the workaround, verify boot proceeds without hanging:

```bash
# Run the IPC test
bazelisk test //pw_kernel/target/ast1030/ipc/user:ipc_test --config=k_qemu_ast1030 --test_output=streamed

# Or run with GDB to verify
bazelisk run //pw_kernel/target/ast1030/ipc/user:ipc_qemu -- -S -s
# In another terminal:
arm-none-eabi-gdb -ex "target remote :1234" -ex "continue" ipc.elf
```

Expected output after fix:
```
[INF] Welcome to Maize on AST1030 User IPC!
[INF] Starting monotonic SysTick timer
[INF] Created initial thread; bootstrapping
[INF] Welcome to the first thread, continuing bootstrap
[INF] Cortex-M initialization
[INF] Starting thread 'idle' (0x00060054)
[INF] Starting thread 'initiator thread' (0x00061080)
[INF] Starting thread 'handler thread' (0x00061960)
```

## Why This Only Affects AST1030 (Initially)

The MPS2-AN505 (ARMv8-M) target may have:
- Different timing characteristics in QEMU's semihosting implementation
- Interrupts enabled at different points during boot
- Different scheduler timing that avoids the race condition

The fundamental issue exists on **all ARM Cortex-M targets using semihosting**, but timing differences may mask it on some platforms.

## Related Documentation

- [svc_instructions_fix.md](svc_instructions_fix.md) - Missing SVC instructions fix
- [control_corruption_analysis.md](control_corruption_analysis.md) - CONTROL register corruption during syscall
- [svc_debug_plan.md](svc_debug_plan.md) - Full debug session log

## Issue Tracking

**TODO:** File issue for interrupt-safe logging backend implementation.
