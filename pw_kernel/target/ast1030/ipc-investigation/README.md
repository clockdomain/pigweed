# ARMv7-M IPC Investigation Strategy

## Problem Statement

The IPC test on AST1030 (ARMv7-M with PMSAv7 MPU) fails with:
```
[INF] Initiator: transact returned 397204 bytes
[ERR] Received 397204 bytes, 8 expected
[INF] MemoryManagement exception triggered: address=0x00060238
```

Both processes start correctly, but `channel_transact()` returns garbage (397204 = 0x60F94).

---

## 🔴 ROOT CAUSE UPDATE (2026-01-24 Session 2)

### Hypothesis Tested: Spurious Wakeup

We tested whether the handler's `object_wait()` was returning spuriously by **commenting out the initiator's IPC calls entirely**. If the handler had a spurious wakeup bug, it would still return early. If it was working correctly, it would block forever (timeout).

### Test Setup
```rust
// initiator.rs - IPC loop COMMENTED OUT
fn test_uppercase_ipcs() -> Result<()> {
    pw_log::info!("Initiator: NOT sending any IPC - waiting 5 seconds");
    for i in 0..50 {
        pw_log::info!("Initiator: tick {}", i as u32);
        // spin loop delay
    }
    Ok(())
}
```

### Test Result: NOT a Spurious Wakeup! 

The handler **correctly blocks** on `object_wait()`. The crash happens **inside the kernel** while processing the syscall:

```
[INF] Handler: waiting for READABLE
[DBG] syscall: 0x0000                              <- object_wait syscall starts
[DBG] syscall: handling object_wait 0 1 ffffffffffffffff
[INF] object_wait: about to lookup_handle          <- kernel starts processing
[INF] Initiator: tick 0                            <- CONTEXT SWITCH TO INITIATOR!
[INF] MemoryManagement exception triggered: address=0x00061778
```

### Key Observations

1. **Handler blocks correctly** - `object_wait` doesn't return early
2. **Crash during syscall handling** - Fault happens between `lookup_handle` and `object.object_wait()`
3. **Context switch during syscall** - Initiator runs while handler's syscall is in-progress
4. **Kernel RAM accessed from user mode** - Fault address `0x00061778` is in kernel RAM (0x60000-0x80000)
5. **Faulting PC is in user space** - `pc = 0x000408d4` is in handler's flash region (0x40000-0x60000)

### Exception Frame Analysis

```
[INF] Kernel exception frame 0x0616cc:
[INF] r4  0x0008ff48 r5  0x00000000 r6  0x00000004 r7  0x0008ffb0
[INF] r8  0x00061778 r9  0x00061788 r10 0x00000004 r11 0x0008fed8
[INF] psp 0x0008fea8 control 0x00000001 return_address 0xfffffffd

[INF] Exception frame 0x08fea8:
[INF] r0  0x00000000 r1  0x00061778 r2  0x0008fed8 r3  0x0008fedc
[INF] r12 0x0008fed8 lr  0x00000003 pc  0x000408d4 psr 0x61000000
```

**Critical findings:**
- `r1 = 0x00061778` and `r8 = 0x00061778` - **Kernel addresses leaked to user registers!**
- `control = 0x00000001` - nPRIV=1 means running in **unprivileged (user) mode**
- `lr = 0x00000003` - **Invalid!** Should be a return address, not a small constant
- `psp = 0x0008fea8` - User stack in handler RAM (0x88000-0x90000) ✓ correct

### Likely Root Cause: Context Switch Stack Corruption

The evidence suggests corruption during context switch:

1. **Kernel addresses in user registers** - The `svc_return` code restores r4-r11 from kernel stack; these should be the user's saved registers, but contain kernel addresses
2. **Invalid LR value** - `lr = 0x00000003` is not a valid return address
3. **Interleaving during syscall** - The `pw_log::info!` calls within the kernel trigger DebugLog syscalls (0xf002), which may cause scheduler to preempt

### Suspected Code Path

In `svc_return()` (pw_kernel/arch/arm_cortex_m/syscall.rs):
```asm
pop     {{ r4 - r11 }}      // Restore from kernel stack
pop     {{ r0 - r1, lr }}   // Get PSP, CONTROL, EXC_RETURN
msr     psp, r0             // Set user stack
msr     control, r1         // Switch to user mode
// ... then pop user exception frame from PSP
```

If the kernel stack frame was corrupted or the wrong frame is being restored (e.g., from a different thread), user code would receive garbage in its registers.

---

## Previous Analysis (Outdated)

~~The handler's `object_wait()` returns BEFORE the initiator's `channel_transact()` has stored the transaction.~~

**UPDATE**: This was a misinterpretation. The handler doesn't return early - it crashes while the syscall is being processed.

---

## Investigation Phases

### Phase 1: Signal & Wait Validation
**Goal**: Verify signal mechanism works correctly between processes
**Status**: 🔴 **ISSUE FOUND** - Handler wakes before transaction stored

### Phase 2: Transaction Storage Validation  
**Goal**: Verify transaction is properly stored in handler object
**Status**: 🔴 **CONFIRMED** - Transaction NOT present when handler reads

### Phase 3: SyscallBuffer Cross-Process Access
**Goal**: Verify kernel can read initiator's buffer from handler context
**Status**: ⏸️ Blocked (Phase 1-2 issue must be fixed first)

### Phase 4: MPU Configuration Analysis
**Goal**: Verify PMSAv7 MPU allows kernel access to user memory
**Status**: ⏸️ Blocked

### Phase 5: End-to-End Fix Validation
**Goal**: Full IPC test passes reliably
**Status**: 🔲 Not Started

---

## Debug Instrumentation Added

### Files Modified

1. **`pw_kernel/kernel/syscall.rs`**
   - `SYSCALL_DEBUG = true` (was false)
   - Added `pw_log::info!` in `handle_channel_read`

2. **`pw_kernel/kernel/object/channel.rs`**
   - Added logging in `channel_read()` for transaction state
   - Added logging in `channel_transact()` for transaction storage

3. **`pw_kernel/kernel/object/buffer.rs`**
   - Added logging in `copy_into()` for addresses and sizes

---

## Quick Links

- [**GDB Debug Strategy**](gdb-debug-strategy.md) - Step-by-step GDB debugging guide
- [Phase 1: Signal Validation](phase1-signal-validation.md)
- [Phase 2: Transaction Storage](phase2-transaction-storage.md)
- [Phase 3: SyscallBuffer Access](phase3-syscallbuffer-access.md)
- [Phase 4: MPU Configuration](phase4-mpu-configuration.md)
- [Phase 5: End-to-End Fix](phase5-end-to-end.md)

---

## Next Steps

### Immediate Priority: Investigate Context Switch / Stack Corruption

1. **Enable `LOG_CONTEXT_SWITCH` in threads.rs**
   - File: `pw_kernel/arch/arm_cortex_m/threads.rs` line 54
   - Set `const LOG_CONTEXT_SWITCH: bool = true;`
   - This will show when/why context switches happen

2. **Audit `svc_return()` stack frame restoration**
   - File: `pw_kernel/arch/arm_cortex_m/syscall.rs`
   - Verify the kernel exception frame being popped matches the current thread
   - Check if MSP (kernel stack pointer) is correct before `pop {{ r4-r11 }}`

3. **Check PendSV interaction with SVCall**
   - PendSV is used for context switching
   - SVCall is used for syscalls
   - Verify PendSV cannot preempt while syscall frame is being set up
   - Check `scb.set_priority(PendSV, 0xFF)` is correctly configuring priorities

4. **Investigate `pw_log::info!` within syscall handlers**
   - Kernel logs trigger DebugLog syscalls (0xf002)
   - These may cause unintended scheduler activity
   - Consider disabling kernel logging during syscall handling

### Secondary Investigation

5. **Compare kernel stack state before/after context switch**
   - Dump MSP and kernel stack contents at key points
   - Verify thread's saved `ArchThreadState` matches actual stack

6. **Check `active_thread` global variable**
   - Used by PendSV to know which thread to switch from
   - Verify it's correctly set/cleared during syscall entry/exit

---

## Test Logs

- [ipc_test_output.log](ipc_test_output.log) - Initial run (no debug)
- [ipc_test_output2.log](ipc_test_output2.log) - With SYSCALL_DEBUG enabled
- **Session 2 (2026-01-24)**: Spurious wakeup test - handler blocks correctly, crashes during syscall
