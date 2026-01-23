# AST1030 IPC MemoryManagement Fault Investigation

**Date:** January 22-23, 2026  
**Target:** AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7)  
**Status:** HardFault FIXED - IPC issue still under investigation

## Executive Summary

The IPC test fails on AST1030 with a MemoryManagement exception when the initiator process attempts to start its first IPC transaction. The fault occurs immediately after both processes are initialized and the initiator begins communication.

**Update (Jan 23):** A separate HardFault issue in the hello_user test was identified and fixed. The root cause was **incorrect exception priority ordering** - PendSV had higher priority than SVCall, allowing context switches to corrupt syscall state.

## Key Fix: Exception Priority Ordering

### Problem

The original priority configuration was:
```rust
// WRONG - PendSV could preempt SVCall mid-setup
scb.set_priority(scb::SystemHandler::SVCall, 0b1111_1111);   // 255 (lowest)
scb.set_priority(scb::SystemHandler::PendSV, 0b1011_1111);   // 191 (higher than SVCall!)
```

When SVCall enabled interrupts before returning to `handle_svc`, a pending PendSV would immediately preempt, corrupting the syscall's fake exception frame setup.

### Solution

Swap priorities so PendSV cannot preempt SVCall:
```rust
// CORRECT - PendSV cannot preempt SVCall
scb.set_priority(scb::SystemHandler::PendSV, 0b1111_1111);   // 255 (lowest)
scb.set_priority(scb::SystemHandler::SVCall, 0b1011_1111);   // 191 (higher than PendSV)
```

### Results

| Test | Before Fix | After Fix |
|------|-----------|-----------|
| hello_user (10 runs) | 10% pass (1/10), 90% HardFault | **80% pass (16/20), 0% HardFault** |

The remaining failures are timeout issues (test completes but shutdown doesn't terminate QEMU), not crashes.

## Reproduction

### Build Commands

```bash
# Build AST1030 IPC test
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc --config=k_qemu_ast1030

# Build MPS2-AN505 IPC test (for comparison)
bazelisk build //pw_kernel/target/mps2_an505/ipc/user:ipc --config=k_qemu_mps2_an505
```

### Test Commands

```bash
# Run AST1030 IPC test via bazelisk
bazelisk test //pw_kernel/target/ast1030/ipc/user:ipc_test \
    --config=k_qemu_ast1030 --test_output=streamed

# Run AST1030 IPC test directly via QEMU
qemu-system-arm -machine ast1030-evb \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
    -nographic

# Run MPS2-AN505 IPC test (for comparison)
bazelisk test //pw_kernel/target/mps2_an505/ipc/user:ipc_test \
    --config=k_qemu_mps2_an505 --test_output=streamed
```

### Results

| Target | Architecture | IPC Test Result |
|--------|--------------|-----------------|
| AST1030 | ARMv7-M (Cortex-M4) | **FAIL** - MemoryManagement |
| MPS2-AN505 | ARMv8-M (Cortex-M33) | PASS |

## Fault Details

### Exception Output

```
[INF] MemoryManagement exception triggered: address=0x00060850
[INF] Kernel exception frame 0x06177c:
[INF] r4  0x0008ff48 r5 0x00000000 r6  0x00000004 r7  0x0008ffb0
[INF] r8  0x00060850 r9 0x00060860 r10 0x00000004 r11 0x0008fed8
[INF] psp 0x0008fea8 control 0x00000001 return_address 0xfffffffd
[INF] Exception frame 0x08fea8:
[INF] r0  0x00000000 r1 0x00060850 r2  0x0008fed8 r3  0x0008fedc
[INF] r12 0x0008fed8 lr 0x00000003 pc  0x000408d4 psr 0x61000000
```

### Key Observations

| Register | Value | Interpretation |
|----------|-------|----------------|
| `address` | `0x00060850` | Faulting address - **kernel memory region** |
| `control` | `0x00000001` | Unprivileged mode, using MSP (unusual!) |
| `return_address` | `0xfffffffd` | EXC_RETURN: Thread mode, PSP |
| `pc` | `0x000408d4` | Faulting instruction |
| `psp` | `0x0008fea8` | User stack pointer (valid) |
| `r1` | `0x00060850` | Same as faulting address - likely source operand |

### Timeline Before Fault

```
[INF] Starting thread 'initiator thread' (0x000610a8)
[INF] Starting thread 'handler thread' (0x00061990)
[INF] 🔄 RUNNING
[INF] Handler: waiting for READABLE      ← Handler ready
[INF] Ipc test starting                   ← Initiator starts
[INF] MemoryManagement exception          ← FAULT!
```

The fault occurs immediately when the initiator tries to start the IPC test, likely during the first `channel_transact()` call.

## Memory Map Analysis

### Address Regions

| Address Range | Region | Accessible from User? |
|---------------|--------|----------------------|
| `0x00000000 - 0x0005FFFF` | Kernel code/data | NO |
| `0x00060000 - 0x0006FFFF` | Kernel heap/threads | NO |
| `0x00080000 - 0x0008FFFF` | User memory | YES |

The faulting address `0x00060850` is in kernel memory, which is correctly protected by the MPU.

### Question: Why is user code accessing kernel memory?

Possible causes:
1. **IPC buffer misconfiguration** - Buffer addresses not properly mapped
2. **Channel handle corruption** - Handle points to kernel object directly
3. **Syscall argument validation** - User passing kernel pointer
4. **Memory layout difference** - AST1030 has different memory layout than AN505

## Comparison: AST1030 vs MPS2-AN505

| Aspect | AST1030 | MPS2-AN505 |
|--------|---------|------------|
| Architecture | ARMv7-M | ARMv8-M |
| MPU | PMSAv7 | PMSAv8 |
| RAM Base | 0x00000000? | 0x10000000? |
| IPC Test | FAIL | PASS |

### Hypothesis: Memory Layout Mismatch

The AST1030 may have a different memory layout that causes IPC buffers or channel objects to be placed in regions inaccessible to user processes.

## Files to Investigate

| File | Purpose |
|------|---------|
| `pw_kernel/target/ast1030/ipc/user/config.rs` | IPC configuration for AST1030 |
| `pw_kernel/target/ast1030/memory.x` | Linker script, memory regions |
| `pw_kernel/syscall/src/channel.rs` | Channel syscall implementation |
| `pw_kernel/kernel/src/channel.rs` | Channel kernel object |
| `pw_kernel/arch/arm_cortex_m/protection_v7.rs` | ARMv7-M MPU configuration |

## Next Steps

1. **Disassemble `0x000408d4`** - Identify the faulting instruction
2. **Check MPU regions** - Verify what regions are configured for the initiator process
3. **Compare memory configs** - Diff AST1030 vs AN505 memory configurations
4. **Trace IPC buffer allocation** - Where are send/receive buffers allocated?
5. **Check channel handle resolution** - How does syscall resolve user handle to kernel object?

## Debug Commands

### Get MPU Configuration

```bash
# In GDB after connecting to QEMU
(gdb) x/8wx 0xE000ED90  # MPU_TYPE, CTRL, RNR, RBAR
(gdb) dump_mpu          # If custom command available
```

### Disassemble Faulting Location

```bash
# Find what instruction is at 0x000408d4
arm-none-eabi-objdump -d bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -A5 "408d4:"
```

### Check Memory Regions

```bash
# Get section info
arm-none-eabi-readelf -S bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
```

## Working Hypothesis

The IPC test accesses a channel object or buffer that is allocated in kernel memory (around `0x00060xxx`). On MPS2-AN505, either:
1. The memory layout places these objects in user-accessible regions, OR
2. The MPU is configured differently

On AST1030, the user process correctly cannot access kernel memory, causing the MPU fault.

**This suggests the bug may be in the IPC configuration for AST1030, not in the kernel itself.**

## Related Issues

- **Resolved:** Syscall context switch crash (DSB+ISB fix) - separate from this issue
- **hello_user test:** PASSES on AST1030 - basic syscalls work
- **IPC test on AN505:** PASSES - IPC works on ARMv8-M

---

## Investigation Log

### January 22, 2026 - Memory Layout Analysis

#### Memory Layout Comparison

**AST1030 (Failing - ARMv7-M / PMSAv7):**
```
Address Range         | Region
----------------------|------------------
0x00000000-0x00000420 | Vector table (1056 bytes)
0x00000420-0x00020000 | Kernel code (~126KB)
0x00020000-0x00040000 | Initiator app code (128KB)
0x00040000-0x00060000 | Handler app code (128KB)
0x00060000-0x00080000 | Kernel RAM (128KB) ← FAULT HERE!
0x00080000-0x00088000 | Initiator app RAM (32KB)
0x00088000-0x00090000 | Handler app RAM (32KB)
```

**MPS2-AN505 (Working - ARMv8-M / PMSAv8):**
```
Address Range         | Region
----------------------|------------------
0x10000000-0x10000800 | Vector table (2KB)
0x10000800-0x10040400 | Kernel code (~255KB)
0x10040400-0x10080000 | Initiator app code
0x10080000-0x100C0000 | Handler app code
0x38000000-0x38010000 | Kernel RAM (64KB)
0x38010000-0x38020000 | Initiator app RAM (64KB)
0x38020000-0x38030000 | Handler app RAM (64KB)
```

**Key Difference:** On MPS2-AN505, RAM is in a completely separate address region (`0x38xxxxxx`) from code (`0x10xxxxxx`). On AST1030, everything is in one contiguous SRAM block starting at `0x00000000`.

#### Faulting Instruction Analysis

The fault occurs at PC=`0x000408d4` which is in the **handler process's code** (handler code is at `0x00040000-0x00060000`).

Disassembly at fault location:
```asm
408cc:  f04f 3401   mov.w   r4, #16843009   ; 0x1010101 (memset pattern)
408d0:  4362        muls    r2, r4
408d2:  f845 2b04   str.w   r2, [r5], #4    ← FAULT (r5=0x00060850)
408d6:  4565        cmp     r5, ip
```

This is a **memset operation** in `pw_assert_HandleFailure_handler_1`. The handler is trying to write to address `0x00060850` which is in **kernel RAM**.

#### Symbol Analysis

The address `0x00060850` falls within kernel static storage:
```
000602f0 b ___init_non_priv_thread8___STATIC   (2048 bytes)
00060af0 b ___init_non_priv_threads_8___STATIC
```

Offset: `0x60850 - 0x602f0 = 1376 bytes` into `___init_non_priv_thread8___STATIC`

This structure contains the **non-privileged thread state**, likely including channel objects or thread control blocks.

#### Root Cause Hypothesis

The handler process is calling `pw_assert_HandleFailure()` which performs a memset on a data structure. The address being written (`0x00060850`) is in kernel RAM, which is correctly protected by the MPU.

**Why is the handler process asserting?**

Looking at the log output:
```
[INF] Handler: waiting for READABLE
[INF] Ipc test starting
[INF] MemoryManagement exception triggered
```

The handler successfully starts and calls `object_wait()`. The initiator starts the IPC test. Then the fault occurs - but in the **handler's** code at `pw_assert_HandleFailure`.

**Possible causes:**
1. **Assert triggered in handler** - Something failed before the IPC transaction started
2. **Stack overflow** - Handler's 2KB stack may be insufficient  
3. **Uninitialized pointer** - Handler is writing to a kernel address instead of its own buffer

#### Section Addresses from ELF

```
Handler Code:    0x00040000 - 0x00040CC0  (.code.handler_1)
Handler Stack:   0x00088000 - 0x00090000  (.stack.handler_1, 32KB)
Handler Heap:    0x00088000               (.heap.handler_1, 0 bytes)

Kernel RAM:      0x00060000 - 0x00080000  (contains thread state, channels)
```

**Problem:** The handler has NO static data section (`.static_init_ram.handler_1` and `.zero_init_ram.handler_1` are 0 bytes). All handler data would need to come from stack or kernel-managed objects.

### January 22, 2026 - Initial Discovery

- IPC test fails immediately after both threads start
- MemoryManagement fault at address `0x00060850`
- Handler is waiting, initiator just started
- Fault is during handler's assert failure path (memset in kernel RAM)

### Next Steps

1. **Check why handler is asserting** - Add debug output before `object_wait()` returns
2. **Verify channel handle** - Is the handler receiving a valid channel handle?
3. **Check stack size** - 2KB may be too small for handler with logging enabled
4. **Compare with AN505 handler** - Does it have static data sections?
5. **Check MPU region configuration** - What regions are configured for handler process?

---

## January 23, 2026 - HardFault Root Cause Found

### The Flaky hello_user Test

Initial testing showed hello_user was extremely flaky - only 10% pass rate with HardFault crashes.

### Investigation Steps

1. **Discovered cached test results were stale** - The initial "PASSED" was from cache, actual runs showed HardFault
2. **Fresh 10-run test**: 1/10 passed (10%), 9/10 HardFault at addresses 0x041ffc or 0x041fac
3. **Analyzed crash dump**: PSP=0x00000000, CONTROL=0x00000000 during syscall handling
4. **Found faulting PC**: 0x0000124c = `handle_svc` - crash during syscall processing

### Root Cause Analysis

The SVCall handler has this sequence:
```asm
// Reenable interrupts now that the exception stack state is coherent.
cpsie i

// Return from exception into the syscall handler
ldr lr, ={exc_return}
bx lr
```

The problem: **PendSV had higher priority than SVCall**. If a context switch was pending when `cpsie i` executed, PendSV would immediately preempt and try to context switch, corrupting the syscall's exception frame setup.

### The Fix

In `pw_kernel/arch/arm_cortex_m/threads.rs`, swap the priorities:

```rust
// Before (WRONG):
scb.set_priority(scb::SystemHandler::SVCall, 0b1111_1111);  // lowest
scb.set_priority(scb::SystemHandler::PendSV, 0b1011_1111);  // higher!

// After (CORRECT):
scb.set_priority(scb::SystemHandler::PendSV, 0b1111_1111);  // lowest
scb.set_priority(scb::SystemHandler::SVCall, 0b1011_1111);  // higher than PendSV
```

### Test Results After Fix

```
=== Summary: 16 passed, 0 hardfaults, 4 timeouts ===
```

- **0 HardFaults** - The crash is completely fixed
- **80% pass rate** - Remaining failures are timeouts (test completes but QEMU shutdown fails)
- The timeout issue is separate and likely related to the shutdown syscall implementation
