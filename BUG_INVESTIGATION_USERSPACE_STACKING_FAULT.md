# Bug Investigation: ARMv7-M Userspace Stacking Fault

**Date:** 2026-01-06
**Platform:** AST1030 (Cortex-M4F) on QEMU ast1030-evb
**Test:** IPC userspace test
**Status:** FIXED

## Root Cause Summary

Two bugs were found and fixed:

### Bug 1: Wrong EXC_RETURN in Exception Wrapper (FIXED)

**The `pop {pc}` instruction in `restore_exception_frame` loads EXC_RETURN from the OLD thread's frame instead of the NEW thread's frame.**

The exception wrapper's restore sequence was:
```asm
mov     sp, r0           // Point SP to new frame
pop     { r4 - r11 }     // Restore callee-saved registers
pop     { r0 - r1, lr }  // Pop PSP into r0, CONTROL into r1, EXC_RETURN into LR
msr     psp, r0          // Set PSP to user stack
msr     control, r1      // Set CONTROL for unprivileged mode
pop     { pc }           // BUG: This pops from OLD frame's saved LR!
```

The problem: After `mov sp, r0`, SP points to the NEW thread's frame. But the original code had `push { r4-r11, lr }` and `pop { r4-r11, pc }`, meaning `pop {pc}` tried to read from the location where LR was pushed - but that's the OLD thread's kernel stack, not the new frame.

**Fix:** Change `pop {pc}` to `bx lr`:
```asm
pop     { r0 - r1, lr }  // Pop EXC_RETURN into LR
msr     psp, r0
msr     control, r1
bx      lr               // Use LR directly (no memory access)
```

File: `pw_kernel/macros/arm_cortex_m_macro.rs`

### Bug 2: Thread-Local State Updated Before SpinLock Drop (FIXED)

In `pendsv_swap_sp`, the code was:
```rust
// OLD (buggy):
THREAD_LOCAL_STATE = new_thread.local;  // Update to NEW thread
// ... logging ...
drop(sched_state);  // SpinLockGuard's PreemptDisableGuard decrements preempt_disable_count
                    // But THREAD_LOCAL_STATE points to NEW thread with count=0 -> UNDERFLOW!
// ... MPU write ...
```

The `SpinLockGuard` contains a `PreemptDisableGuard` that decrements `preempt_disable_count` on drop. When `THREAD_LOCAL_STATE` is updated before the drop, the decrement uses the NEW thread's count (which is 0), causing underflow.

**Fix:** Drop `sched_state` BEFORE updating `THREAD_LOCAL_STATE`:
```rust
// NEW (correct):
drop(sched_state);  // Uses OLD thread's preempt_disable_count
THREAD_LOCAL_STATE = new_thread.local;  // Now safe to switch
// ... MPU write (LAST) ...
```

File: `pw_kernel/arch/arm_cortex_m/threads.rs`

## Detailed Bug Analysis

### Bug 1 Mechanism

1. Bootstrap/idle threads run as kernel threads (use MSP, PSP=0)
2. First context switch to userspace thread occurs via PendSV
3. `save_exception_frame` saves PSP (which is 0 or garbage from kernel thread)
4. `restore_exception_frame` restores PSP from user frame (correct PSP ~0xa8400)
5. **BUT `pop {pc}` loads EXC_RETURN from wrong location**
6. If that EXC_RETURN has bit 2 = 0 (use MSP), hardware ignores PSP and tries to stack to MSP
7. MSTKERR at 0x0008235c - attempted stacking to kernel MSP violates MPU

### EXC_RETURN Values

| Value | Meaning |
|-------|---------|
| 0xFFFFFFF1 | Handler mode, MSP (nested exception) |
| 0xFFFFFFF9 | Thread mode, MSP (kernel thread) |
| 0xFFFFFFFD | Thread mode, PSP (userspace thread) |

The userspace thread frame must have EXC_RETURN = 0xFFFFFFFD (bit 2 = 1 for PSP).
Using `bx lr` ensures the correct value from the new frame is used.

## Test Output (Before Fix)

```
[INF] Starting thread 'handler thread' (0x00081478)
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
[DBG] MPU[1]: RBAR=0x000A0000 RASR=0x130BE31F
[DBG] MPU[2]-[7]: RASR=0x00000000 (disabled)
[INF] HardFault exception triggered: HFSR=0x40000000 CFSR=0x00000092
[INF]   MMFSR.DACCVIOL: Data access violation
[INF]   MMFSR.MSTKERR: MemManage fault on stacking
[INF]   MMFSR.MMARVALID: MMFAR=0x0008235c
[INF] Kernel exception frame 0x082324:
[INF] r4  0x00080424 r5 0x00000000 r6  0x00000008 r7  0x00082420
[INF] r8  0x00000001 r9 0x0008157c r10 0x000814a4 r11 0x00000001
[INF] psp 0x00000000 control 0x00000000 return_address 0xfffffff1
[INF] Exception frame 0x082350:
[INF] r0  0x0008157c r1 0x00000000 r2  0x00000044 r3  0x00000000
[INF] r12 0x00000000 lr 0x00000007 pc  0x00082420 psr 0x000025cf
```

### CFSR Decode (0x00000092)

| Bit | Name | Value | Meaning |
|-----|------|-------|---------|
| 1 | DACCVIOL | 1 | Data access violation |
| 4 | MSTKERR | 1 | MemManage fault on stacking |
| 7 | MMARVALID | 1 | MMFAR contains valid address |

### Memory Layout

```
0x00000000 - 0x00000420: Vector table (1056 bytes)
0x00000420 - 0x00040420: Kernel code (256KB)
0x00040420 - 0x00060420: Initiator app code (128KB)
0x00060420 - 0x00080420: Handler app code (128KB)
0x00080420 - 0x000a0420: Kernel RAM (128KB)  <-- MMFAR 0x8235c is HERE
0x000a0420 - 0x000a4420: Initiator app RAM (16KB)
0x000a4420 - 0x000a8420: Handler app RAM (16KB)
```

## Files Modified

1. **pw_kernel/macros/arm_cortex_m_macro.rs**
   - Changed `restore_exception_frame` to use `bx lr` instead of `pop {pc}`
   - Added comments explaining why `bx lr` is necessary

2. **pw_kernel/arch/arm_cortex_m/threads.rs**
   - Reordered operations in `pendsv_swap_sp`:
     1. Get new thread reference
     2. Log (before MPU switch)
     3. `drop(sched_state)` - uses OLD thread's state
     4. Update `THREAD_LOCAL_STATE` to new thread
     5. MPU write (LAST - no kernel memory access after this)
     6. Return frame pointer

3. **pw_kernel/arch/arm_cortex_m/protection_v7.rs**
   - Added explicit DSB/ISB memory barriers after MPU enable
   - Removed logging after MPU enable (would fault)
   - Ensured PRIVDEFENA=true is set

4. **pw_kernel/arch/arm_cortex_m/exceptions.rs**
   - Added CFSR decode to HardFault handler for debugging

---

## Bug 3: PMSAv7 Subregion Overlap with Kernel RAM (IN PROGRESS)

After fixing bugs 1 and 2, the test progresses further but hits a new MSTKERR fault:

```
[INF] PendSV returning frame: psp=0x000a8400 control=0x00000003 ret_addr=0xfffffffd
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
[DBG] MPU[1]: RBAR=0x000A0000 RASR=0x130BE31F
[INF] HardFault exception triggered: HFSR=0x40000000 CFSR=0x00000092
[INF]   MMFSR.MSTKERR: MemManage fault on stacking
[INF]   MMFSR.MMARVALID: MMFAR=0x00081444
```

### Root Cause

The userspace thread started running, but when it took an exception (SVC syscall), the processor tried to stack to the kernel MSP at 0x00081444. The MPU denied this access.

**Why?** The handler app's code MPU region inadvertently covers kernel RAM due to PMSAv7's power-of-2 alignment requirements.

### PMSAv7 Subregion Analysis

Handler code region: [0x60420, 0x80420) = 128KB

PMSAv7 requires power-of-2 regions aligned to their size. The `calculate_aligned_region` function computes:
- Requested range needs to cover 0x60420 to 0x80420
- Smallest power-of-2 that works: 1MB (0x100000) at base 0x0
- Each subregion is 1MB/8 = 128KB

Subregion layout for 1MB region at base 0:
| Subregion | Address Range | Enabled? | Contents |
|-----------|---------------|----------|----------|
| 0 | 0x00000-0x20000 | No | Vector table, kernel code |
| 1 | 0x20000-0x40000 | No | Kernel code |
| 2 | 0x40000-0x60000 | No | Initiator app code |
| 3 | 0x60000-0x80000 | **Yes** | Handler app code (0x60420-0x80000) |
| 4 | 0x80000-0xA0000 | **Yes** | Handler app code (0x80000-0x80420) + **KERNEL RAM** |
| 5 | 0xA0000-0xC0000 | No | App RAM |
| 6 | 0xC0000-0xE0000 | No | Unused |
| 7 | 0xE0000-0x100000 | No | Unused |

**The Problem:** Handler app code ends at 0x80420, which is 0x420 bytes into subregion 4. To cover this, subregion 4 (0x80000-0xA0000) must be enabled. But kernel RAM (0x80420-0xA0420) is ALSO in subregion 4!

The MPU region has AP=6 (RoAny = read-only for all, including privileged). When the processor tries to write (stack) to kernel RAM at 0x81444, the MPU denies it because the region is marked read-only.

### Why PRIVDEFENA Doesn't Help

PRIVDEFENA=1 allows privileged access to **unmapped** regions. But kernel RAM IS mapped - it falls within the enabled subregion 4 of the handler's code MPU region. The MPU region's AP=RoAny applies, blocking the write.

### The Fix

Adjust the memory layout so kernel RAM doesn't fall within any userspace MPU subregion. Options:

**Option A: Move kernel RAM after app RAM**
```
0x00000000 - 0x00000400: Vector table (1KB, aligned)
0x00000400 - 0x00040400: Kernel code (256KB)
0x00040400 - 0x00060400: Initiator app code (128KB)
0x00060400 - 0x00080000: Handler app code (ends at 128KB boundary)
0x00080000 - 0x00084000: Initiator app RAM (16KB)
0x00084000 - 0x00088000: Handler app RAM (16KB)
0x00088000 - 0x000A8000: Kernel RAM (128KB)
```

This ensures:
- Handler code ends at 0x80000, so only subregion 3 is needed (not subregion 4)
- Kernel RAM at 0x88000+ is in subregion 4, but that subregion is NOT enabled for the code region

**Option B: Shrink handler code to fit in subregion 3**
- End handler code at 0x80000 instead of 0x80420
- Lose 0x420 bytes (1KB) of code space

### File to Modify

`pw_kernel/target/ast1030/ipc/user/system.json5` - Update memory layout addresses

### Status: FIXED

Applied memory layout fix to system.json5. Kernel stacking fault resolved.

---

## Bug 4: Userspace Entry Point Branches to NULL (CURRENT)

After fixing the memory layout, the test progresses further but now fails with a MemManage fault at PC=0x00000000:

```
[INF] PendSV returning frame: psp=0x000affe0 control=0x00000003 ret_addr=0xfffffffd
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00060000 RASR=0x060F0021
[DBG] MPU[1]: RBAR=0x000AC000 RASR=0x130B001B
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x0affe0:
[INF] r0  0x00000000 r1 0xbaa98dee r2  0x000ac1e0 r3  0x00000000
[INF] r12 0x00000000 lr 0x00000000 pc  0x00000000 psr 0x40000000
```

### Analysis

1. **Context switch to userspace succeeded** - The handler thread started with correct:
   - PSP = 0x000affe0 (userspace stack)
   - CONTROL = 0x00000003 (NPRIV=1, SPSEL=1)
   - EXC_RETURN = 0xfffffffd (Thread mode + PSP)

2. **MPU regions look correct**:
   - Region 0: RBAR=0x60000, SIZE=128KB, SRD=0x00 (all enabled) - handler code
   - Region 1: RBAR=0xAC000, SIZE=16KB - handler RAM

3. **The fault**: PC=0x00000000, LR=0x00000000
   - The userspace code branched to address 0
   - This is NOT an MPU permission issue - it's a NULL function pointer

### Root Cause Hypothesis

The userspace entry point (`_start` in `arm_cortex_m/entry.s`) calls:
```asm
bl   memcpy    // Copy .data section
bl   memset    // Zero .bss section
bl   main      // Call user's main function
```

If `memcpy` or `memset` symbols are not properly linked (resolve to 0), the `bl` instruction branches to NULL.

### Possible Causes

1. **Missing compiler builtins**: The userspace app may not include `compiler_builtins` crate which provides `memcpy`/`memset` for no_std targets

2. **Linker script issue**: The app linker script may not include these symbols

3. **Library linking order**: The runtime library with these functions may not be linked

### Investigation Needed

1. Check if the working ARMv8-M (mps2_an505) IPC test has the same dependency setup
2. Disassemble the handler binary to see what addresses `memcpy`/`memset` resolve to
3. Check if `compiler_builtins` or equivalent is in the dependency chain

### Files Involved

- `pw_kernel/userspace/arm_cortex_m/entry.s` - Entry point assembly
- `pw_kernel/tests/ipc/user/BUILD.bazel` - App dependencies
- `pw_kernel/userspace/BUILD.bazel` - Userspace library deps

### Status: FIXED

Applied fixes from test-baseline branch:
1. Added `entry_asm_arm_cortex_m` cc_library for entry assembly
2. Added dependency to userspace apps
3. Changed `subs` to `sub` in entry.s

---

## Bug 5: SysTick Race with PreemptDisableGuard (FIXED)

After fixing entry assembly, got assertion:
```
[FTL] scheduler::tick() called with preemption disabled
```

Applied fix from test-baseline: Split SysTick init into early_init (counter only) and init (enable interrupts after PreemptDisableGuard dropped).

---

## Bug 6: Handler RAM Access at 0xB0000 (CURRENT)

```
[INF] MemoryManagement exception triggered: address=0x000b0000
[INF] Exception frame 0x0affe0:
[INF] pc  0x00060402 psr 0x41000000
```

Handler app is running (PC=0x00060402 = entry+1 for Thumb) but accessing 0xB0000 which is just past handler RAM end.

Handler RAM MPU: RBAR=0xAC000, SIZE=16KB, so end is 0xB0000. The access at exactly 0xB0000 is one byte past the allocation.

This appears to be the _start code trying to read linker symbols that point to invalid addresses.

### Analysis Needed

The _start code does:
```asm
ldr  r0, =_pw_static_init_ram_start
ldr  r1, =_pw_static_init_flash_start
ldr  r2, =_pw_static_init_ram_end
```

These linker symbols may not be correctly set for userspace apps.

---

## Test Commands

**Build and run the IPC test on AST1030 QEMU:**
```bash
source activate.sh
bazelisk test --config=k_qemu_ast1030 --test_timeout=180 //pw_kernel/target/ast1030/ipc/user:ipc_test
```

**Clean build (when caching issues suspected):**
```bash
bazelisk clean
# Then run test as above
```

**Build only (no test):**
```bash
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc
```

**Disassemble handler binary:**
```bash
arm-none-eabi-objdump -d bazel-bin/pw_kernel/tests/ipc/user/handler
```

---

## Bug 7: Userspace Branches to NULL (CURRENT)

After all previous fixes (Bugs 1-6), the test now shows the handler thread executes
briefly but then branches to address 0x00000000.

### Test Output (2026-01-07)

```
[INF] Allocating non-privileged thread 'handler thread' (entry: 0x00060421)
[INF] Initializing non-privileged thread 'handler thread'
[INF] initialize_user_frame: user_frame=0x000a8400 kernel_frame=0x00041448 psp=0x000a8400 pc=0x00060421 exc_ret=0xfffffffd
[INF] Starting thread 'handler thread' (0x00041478)
[INF] Allocating non-privileged process 'initiator process'
[INF] Allocating non-privileged thread 'initiator thread' (entry: 0x00040421)
[INF] Initializing non-privileged thread 'initiator thread'
[INF] initialize_user_frame: user_frame=0x000a4400 kernel_frame=0x00041c90 psp=0x000a4400 pc=0x00040421 exc_ret=0xfffffffd
[INF] Starting thread 'initiator thread' (0x00041cbc)
[INF] PendSV returning frame: psp=0x000a8400 control=0x00000003 ret_addr=0xfffffffd
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
[DBG] MPU[1]: RBAR=0x000A0000 RASR=0x130BE31F
[DBG] MPU[2]-[7]: RBAR=0x00000000 RASR=0x00000000
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Kernel exception frame 0x041448:
[INF] r4  0x00000000 r5 0x00000000 r6  0x00000000 r7  0x00000000
[INF] r8  0x00000000 r9 0x00000000 r10 0x00000000 r11 0x00000000
[INF] psp 0x000a8408 control 0x00000001 return_address 0xfffffffd
[INF] Exception frame 0x0a8408:
[INF] r0  0x00000000 r1 0x00000000 r2  0x00000000 r3  0x00000000
[INF] r12 0x00000000 lr 0x00000000 pc  0x00000000 psr 0x40000000
```

### Analysis

1. **Context switch to handler succeeded** - PendSV returns with:
   - PSP = 0x000a8400 (correct userspace stack)
   - CONTROL = 0x00000003 (NPRIV=1, SPSEL=1 - unprivileged, using PSP)
   - EXC_RETURN = 0xfffffffd (Thread mode + PSP)

2. **MPU regions configured**:
   - Region 0: RBAR=0x00000000, RASR=0x060FE727
     - SIZE field = 0x13 (19) = 2^20 = 1MB region
     - SRD = 0xE7 = 0b11100111 - subregions 0,1,2,5,6,7 disabled, 3,4 enabled
     - Each subregion = 1MB/8 = 128KB
     - Enabled range: [0x60000, 0xA0000) = handler code (128KB) + something else?
   - Region 1: RBAR=0x000A0000, RASR=0x130BE31F
     - SIZE field = 0x0F (15) = 2^16 = 64KB region
     - SRD = 0xE3 = 0b11100011 - subregions 0,1,5,6,7 disabled, 2,3,4 enabled
     - Each subregion = 64KB/8 = 8KB
     - Enabled range: [0xA0000+16KB, 0xA0000+40KB) = [0xA4000, 0xC8000)?
     - **But that's wrong for 16KB handler RAM starting at 0xA4000**

3. **The fault**: MemManage at address 0x00000000, PC=0x00000000
   - Userspace code branched to NULL
   - All registers are 0 in the exception frame
   - PSP moved from 0xa8400 to 0xa8408 (+8 bytes = exception frame pushed)

### Root Cause Hypothesis

**The handler entry point 0x00060421 appears to be correctly set, but the code
immediately branches to 0x00000000.**

The userspace _start code calls `memcpy`, `memset`, then `main`. If any of these
symbols resolved to 0, the `bl` instruction would branch to NULL.

Checking the MPU layout:
- Code region [0x60000, 0xA0000) enabled
- Handler code should be at 0x60420 per system.json5
- Entry 0x00060421 = 0x60420 + 1 (Thumb bit)

**Problem identified**: The exception frame at 0xa8408 shows PSP advanced by 8 bytes,
but the normal exception frame is 32 bytes (8 registers). This suggests the frame
wasn't properly initialized or was corrupted.

### Detailed Exception Frame Analysis

Expected user exception frame layout (32 bytes):
```
PSP+0x00: r0
PSP+0x04: r1
PSP+0x08: r2
PSP+0x0C: r3
PSP+0x10: r12
PSP+0x14: lr
PSP+0x18: pc (return address)
PSP+0x1C: psr
```

The frame is at 0xa8408, but initialized PSP was 0xa8400. The +8 offset suggests
the hardware pushed an exception frame ON TOP of an existing stack frame.

**Wait** - This is backwards. PSP was initialized to 0xa8400 (top of user stack).
After exception, PSP = 0xa8408 (8 bytes higher?). That's wrong - PSP should DECREASE
when stacking, not increase.

Actually, looking more carefully:
- initialize_user_frame sets psp=0x000a8400
- Exception frame logged at 0x0a8408

This means the kernel printed the WRONG address, or the PSP was corrupted.

### Next Investigation Steps

1. **Verify the MPU region calculation** for handler code
2. **Check if linker symbols (memcpy, memset, main) are correctly linked**
3. **Verify exception frame initialization** in initialize_user_frame
4. **Examine why PSP appears at 0xa8408 instead of 0xa8400-0x20**

### Files to Investigate

- Handler binary symbols: `arm-none-eabi-nm bazel-bin/pw_kernel/tests/ipc/user/handler`
- Handler disassembly: `arm-none-eabi-objdump -d bazel-bin/pw_kernel/tests/ipc/user/handler`
- System generator: `pw_kernel/tooling/system_generator/src/main.rs`
- Exception frame init: `pw_kernel/arch/arm_cortex_m/threads.rs`

### Investigation Commands to Run

After building with `bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc`:

**1. Find the handler binary location:**
```bash
find bazel-bin -name "handler" -type f 2>/dev/null
# or
find ~/.cache/bazel -path "*pw_kernel/tests/ipc/user*" -name "handler" -type f 2>/dev/null
```

**2. Check handler symbols (memcpy, memset, main, _start):**
```bash
arm-none-eabi-nm <handler_path> | grep -E "memcpy|memset|_start|main|_pw_"
```

**3. Disassemble handler entry point:**
```bash
arm-none-eabi-objdump -d <handler_path> | head -100
```

**4. Check linker symbols in handler:**
```bash
arm-none-eabi-nm <handler_path> | grep _pw_static_init
arm-none-eabi-nm <handler_path> | grep _pw_zero_init
```

**5. Examine generated handler linker script:**
```bash
find ~/.cache/bazel -name "*.ld" -path "*handler*" 2>/dev/null
```

**6. Check system.rs generated code:**
The system generator creates `system.rs` with memory layout. Look for:
```bash
grep -r "handler" ~/.cache/bazel/**/system.rs 2>/dev/null
```

### Key Questions to Answer

1. **Is the handler entry point at 0x60420 (0x60421 with Thumb bit)?**
   - Check if the binary is linked correctly
   - The log shows entry: 0x00060421 which looks correct

2. **What addresses do memcpy/memset resolve to?**
   - If they resolve to 0, that's the bug
   - These should be linked from compiler builtins or libc

3. **What is the initial exception frame content?**
   - The frame should have PC=entry point, PSR=0x01000000 (Thumb bit)
   - Check initialize_user_frame in threads.rs

4. **Why does PSP appear at 0xa8408 instead of 0xa8400?**
   - Initial PSP was 0xa8400 (from initialize_user_frame log)
   - After fault, PSP in kernel frame is 0xa8408 (+8 bytes)
   - **BUT**: This might be correct! The kernel logs the PSP AFTER the exception handler saved it
   - Need to check what save_exception_frame does with PSP

5. **What triggered the MemManage fault at address 0x00000000?**
   - The PC=0 means instruction fetch from address 0
   - This is unprivileged code trying to execute address 0
   - MPU should deny access to address 0 for unprivileged mode

### Likely Root Cause Candidates

**Candidate 1: PSP pointing to wrong location for exception return**

The hardware unstacks the exception frame from PSP when doing exception return.
If PSP was set to 0xa8400 but the actual exception frame was written somewhere else,
the hardware would pop garbage (zeros) into the registers.

Check: Is the exception frame at PSP-32 = 0xa83E0 or at PSP = 0xa8400?

**Candidate 2: Exception frame not initialized correctly**

The `initialize_user_frame` function writes:
- pc = entry point (0x60421)
- psr = 0x01000000 (Thumb mode)
- Other registers = 0

But if the frame was written to the WRONG address, PSP would point to uninitialized memory.

Check: Where does initialize_user_frame actually write the frame?

**Candidate 3: Stack grows down vs up confusion**

ARM stacks grow DOWN (toward lower addresses). The exception return pops from PSP.
If initialize_user_frame put the frame at the TOP of the stack (high address),
but PSP was set to the bottom, the hardware would pop from wrong location.

Check: Is initial_sp set correctly? Does it account for the pre-pushed exception frame?

---

### Memory Layout Verification (2026-01-07)

**From system.json5:**
- Kernel flash: 0x420 - 0x40420 (256KB)
- Kernel RAM: 0x40420 - 0xA0420 (384KB)

**System generator calculates:**
- Initiator flash: 0x40420 (after kernel flash)
- Handler flash: 0x60420 (after initiator, 128KB later)
- Initiator RAM: 0xA0420 (after kernel RAM), 16KB, ends at 0xA4420
- Handler RAM: 0xA4420 (after initiator RAM), 16KB, ends at 0xA8420

**Handler exception frame:**
- initial_sp = 0xA8420 (top of handler RAM)
- user_frame = 0xA8420 - 32 = 0xA8400 (exception frame location)
- PSP set to 0xA8400

**MPU Region Analysis:**

Region 0 (Code): RBAR=0x00000000, RASR=0x060FE727
- SIZE = 19 → 1MB region
- SRD = 0xE7 = 0b11100111 → subregions 3,4 enabled
- Enabled range: [0x60000, 0xA0000)
- Handler code at 0x60420 ✓ within enabled range

Region 1 (RAM): RBAR=0x000A0000, RASR=0x130BE31F
- SIZE = 15 → 64KB region base at 0xA0000
- SRD = 0xE3 = 0b11100011 → subregions 2,3,4 enabled
- Each subregion = 8KB
- Enabled range: [0xA4000, 0xAA000)
- Handler RAM at [0xA4420, 0xA8420) ✓ within enabled range
- Exception frame at 0xA8400 ✓ within enabled range
- AP = 3 (Full access RW) ✓

**Conclusion: MPU configuration is CORRECT**

The exception frame at 0xA8400 should be accessible. The problem is elsewhere.

---

### Deeper Investigation Needed

**Theory: Exception frame was written correctly but contains zeros when read back**

Possible causes:
1. **Write didn't actually happen** - The `mem::zeroed()` + field assignments never executed
2. **Memory was cleared after write** - Something zeroed the memory between init and context switch
3. **Cache coherency issue** - Frame written but not flushed to memory visible to exception return
4. **Wrong PSP restored** - The frame exists but PSP points somewhere else

**New Debug Steps:**

1. Add logging INSIDE initialize_frame to verify values being written
2. Add memory dump before PendSV to verify frame contents at PSP
3. Check if DSB/ISB barriers are needed after writing exception frame
4. Verify that the kernel isn't accidentally clearing app RAM during allocation

---

### PSP Discrepancy Analysis

**Observation:**
- `initialize_user_frame` logs: `psp=0x000a8400`
- Fault dump shows: `psp 0x000a8408` (+8 bytes!)

**Why the PSP changed:**
The fault dump shows the state AFTER the MemManage fault occurred, not before.
When MemManage fault triggers:
1. Hardware pushes new exception frame to PSP
2. `save_exception_frame` in the MemManage handler saves the NEW PSP

So PSP=0xA8408 is the PSP after the fault, not the initial value.

**But why +8 instead of -32?**
Standard exception frame is 32 bytes. After fault, PSP should DECREASE by 32.
0xA8400 - 32 = 0xA83E0, NOT 0xA8408.

The fact that PSP = 0xA8408 (which is 8 bytes HIGHER) suggests the exception
frame was read from the WRONG location entirely.

**Possible explanation:**
The hardware exception return popped from PSP=0xA8400. If those 32 bytes contained
garbage (all zeros), then:
- r0=0, r1=0, r2=0, r3=0, r12=0, lr=0, pc=0, psr=0x40000000

With PC=0, the CPU tried to fetch instruction from address 0, causing MemManage.

The new exception frame was pushed... but wait, where? If PSP was "restored" from
the garbage frame, it might have been set to a weird value.

Actually, let me look at the psr value: 0x40000000
- Bit 24 (T) = 0 → ARM mode!

But Cortex-M only supports Thumb mode. T bit should ALWAYS be 1 in PSR.
If PSR.T = 0, the processor would fault.

**CRITICAL FINDING: The exception frame contains invalid PSR (T bit = 0)**

This means the exception frame was never properly initialized, or was overwritten.
The T bit must be 1 for Cortex-M. A PSR of 0x40000000 has T=0 which is invalid.

When `initialize_frame` runs:
```rust
(*user_frame).psr = RetPsrVal(0).with_t(true);
```

This should set PSR to 0x01000000 (T bit at position 24).

The fault shows PSR = 0x40000000 which has bit 30 set (overflow flag) but NOT bit 24 (T).

**Conclusion: The exception frame at 0xA8400 contains garbage, not the initialized values.**

Likely causes:
1. initialize_frame wrote to wrong address
2. Memory at 0xA8400 was overwritten after initialization
3. Some other code path didn't call initialize_frame
