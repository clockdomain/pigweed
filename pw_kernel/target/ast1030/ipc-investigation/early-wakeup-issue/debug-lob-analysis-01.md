# Debug Log Analysis - Session B (2026-01-24)

## Platform Test Results

| Platform | Arch | IPC Test | Notes |
|----------|------|----------|-------|
| **AST1030** | ARMv7-M (Cortex-M4) | ❌ FAILS | "object_wait FAILED", error code 0 |
| **MPS2-AN505** | ARMv8-M (Cortex-M33) | ✅ PASSES | IPC works correctly |
| **LM3S6965** | ARMv7-M (Cortex-M3) | ❌ FAILS | Different failure: handler works, but transact returns garbage (536875176 bytes) |

**Conclusion**: Bug affects **both ARMv7-M platforms** but not ARMv8-M. The failure mode differs:
- AST1030: object_wait fails immediately with corrupted error code
- LM3S6965: IPC proceeds further but transact return value is corrupted

Both show signs of **return value corruption** - likely an ARMv7-M-specific issue in syscall return handling.

## NEW THEORY: Callee-Saved Register Corruption During Context Switch

### The Problem

When `handle_svc` blocks during a syscall:

1. `handle_svc` is running as regular thread code (SVCall has already returned)
2. The compiler may store `frame_ptr` in a callee-saved register (r4-r11)
3. When PendSV fires for context switch:
   - PendSV saves the **current** r4-r11 (kernel's working values, NOT the original userspace values)
   - `active_thread.frame` is set to this **new** PendSV frame
4. When thread resumes:
   - PendSV restores r4-r11 from the saved PendSV frame (kernel's garbage)
   - Execution continues in `handle_svc`
   - `handle_svc` tries to return `frame_ptr` but the register holding it is corrupted!
5. `svc_return` receives wrong `frame_ptr` in r0, pops from wrong location

### Why ARMv7-M but not ARMv8-M?

ARMv8-M (Cortex-M33) has TrustZone and different exception stacking behavior. The context switch
mechanism may work differently, or the compiler generates different code that doesn't trigger this bug.

### Evidence

- Both ARMv7-M platforms show return value corruption at different points
- LM3S6965's "536875176 bytes" = 0x20000A18 looks like a RAM address, suggesting the return value
  register has a pointer instead of a count
- AST1030's "error 0" is impossible (all errors are non-zero), suggesting r0/r1 have wrong values

### Proposed Fix

Option 1: Save `frame_ptr` to the KernelExceptionFrame itself (not just in a register)
Option 2: Ensure PendSV preserves the SVCall frame pointer when context-switching during syscall
Option 3: Use `#[inline(never)]` and explicit memory operations to force frame_ptr to stack

---

## Session Configuration

- **Approach**: Minimal intrusion - hardware watchpoint + single breakpoint
- **Watchpoint**: `*(unsigned int*)0x60850` (didn't trigger - value being read, not written)
- **Breakpoint**: `*MemoryManagement` (triggered)

## Fault Summary

| Field | Value | Notes |
|-------|-------|-------|
| **MMFSR** | 0x82 | DACCVIOL - Data access violation |
| **MMFAR** | 0xffffffff | Invalid address accessed |
| **PSP** | 0x8fea8 | User stack pointer |
| **MSP** | 0x61858 | Kernel stack pointer |

## Faulting Instruction

```asm
0x408d4:  ldr.w  r0, [r1], #16   ← FAULT HERE
```

User code tried to load from `r1 = 0xffffffff` - invalid address.

## User Exception Frame at PSP (0x8fea8)

| Offset | Address | Value | Field | Status |
|--------|---------|-------|-------|--------|
| +0x00 | 0x8fea8 | 0x00000000 | r0 | OK |
| +0x04 | 0x8feac | **0xffffffff** | r1 | ❌ WRONG |
| +0x08 | 0x8feb0 | 0x0008fed8 | r2 | OK |
| +0x0C | 0x8feb4 | 0x0008fedc | r3 | OK |
| +0x10 | 0x8feb8 | 0x0008fed8 | r12 | OK |
| +0x14 | 0x8febc | **0x00000003** | LR | ❌ WRONG (CONTROL value!) |
| +0x18 | 0x8fec0 | 0x000408d4 | PC | OK (return address) |
| +0x1C | 0x8fec4 | 0x61000000 | xPSR | OK |

## Key Corruption Pattern

The user LR contains `0x3` which is the **CONTROL register value** from the KernelExceptionFrame, not a return address.

This confirms the frame offset bug - registers are being restored from wrong offsets.

## Kernel Stack Analysis (MSP region)

### KernelExceptionFrame at ~0x61830

```
0x61830: 0x00000001  0x000011c9  0x000618f0  0xffffffff   (r4, r5, r6, r7)
0x61840: 0x00040ffc  0xffffffff  0x00000000  0x0008ff80   (r8, r9, r10, r11)
0x61850: 0x00000003  0xfffffffd  ...                      (CONTROL, EXC_RETURN)
```

| Offset | Address | Value | Field |
|--------|---------|-------|-------|
| +0x00 | 0x61830 | 0x00000001 | r4 |
| +0x04 | 0x61834 | 0x000011c9 | r5 |
| +0x08 | 0x61838 | 0x000618f0 | r6 |
| +0x0C | 0x6183c | 0xffffffff | r7 |
| +0x10 | 0x61840 | 0x00040ffc | r8 |
| +0x14 | 0x61844 | 0xffffffff | r9 |
| +0x18 | 0x61848 | 0x00000000 | r10 |
| +0x1C | 0x6184c | 0x0008ff80 | r11 |
| +0x20 | 0x61850 | **0x00000003** | CONTROL |
| +0x24 | 0x61854 | 0xfffffffd | EXC_RETURN |

**Note**: CONTROL (0x3) is at offset +0x20 (address 0x61850).

## User Stack Data at 0x8ff80

```
0x8ff80: 0x00000000  0x00000001  0xffffffff  0xffffffff
0x8ff90: 0x0008ff5d  0xffffffff  0x00040ffc  0xffffffff
```

Address 0x8ff84 contains `0x00000001` - this may be what r1 **should** have been.

## Comparison with Previous Sessions

| Field | Previous Session | Session B |
|-------|-----------------|-----------|
| MMFSR | 0x82 (DACCVIOL) | 0x82 (DACCVIOL) ✓ |
| MMFAR | 0x60850 | 0xffffffff |
| User r1 | 0x60850 | 0xffffffff |
| User r8 | 0x60850 | 0xffffffff |
| User LR | 0x3 | 0x3 ✓ **Same!** |

The **LR = 0x3 corruption is consistent** across sessions. The different garbage values in r1/r8 depend on timing, but the offset pattern is the same.

## svc_return Disassembly Analysis

```asm
0x4a4 <+0>:   mov   sp, r0              ; SP = frame pointer (r0)
0x4a6 <+2>:   ldmia sp!, {r4-r11}       ; Pop callee-saved regs (32 bytes)
0x4aa <+6>:   ldmia sp!, {r0, r1, lr}   ; Pop PSP(r0), CONTROL(r1), EXC_RETURN(lr) (12 bytes)
0x4ae <+10>:  msr   PSP, r0             ; Restore user stack pointer
0x4b2 <+14>:  orr   r1, r1, #3          ; Set CONTROL bits
0x4b6 <+18>:  msr   CONTROL, r1         ; Restore CONTROL
0x4ba <+22>:  dsb   sy                  ; Barriers
0x4be <+26>:  isb   sy
0x4c2 <+30>:  ldmia sp!, {r0-r3, r12, lr} ; Pop user exception frame (24 bytes)
0x4c6 <+34>:  pop   {r1}                ; Pop PC into r1
0x4c8 <+36>:  pop   {r0}                ; Pop xPSR into r0
0x4ca <+38>:  ands  r0, r0, #512        ; Check xPSR bit 9 (stack alignment)
0x4ce <+42>:  it    ne
0x4d0 <+44>:  addne sp, #4              ; Adjust SP if aligned
0x4d2 <+46>:  mov   r0, #0              ; Clear r0
0x4d6 <+50>:  orr   r1, r1, #1          ; Set thumb bit in PC
0x4da <+54>:  bx    r1                  ; Return to user code
```

### Stack Layout Expected by svc_return

Starting from frame pointer (r0):
| Offset | Size | Content |
|--------|------|---------|
| +0x00 | 32 bytes | r4-r11 (callee-saved) |
| +0x20 | 12 bytes | PSP, CONTROL, EXC_RETURN |
| +0x2C | 24 bytes | r0-r3, r12, LR (user exception frame) |
| +0x44 | 4 bytes | PC |
| +0x48 | 4 bytes | xPSR |

**Total: 0x4C (76) bytes**

## ROOT CAUSE ANALYSIS - REVISED 🎯

### Initial Hypothesis (Incorrect)

At instruction `0x4c2` (+30):
```asm
ldmia sp!, {r0-r3, r12, lr}   ; Pop user exception frame
```

Initial theory: This loads from kernel stack (MSP) instead of user stack (PSP).

**However**, after ARM architecture analysis, this is NOT the bug. After `msr control, r1` sets SPSEL=1 in Thread mode, `pop` correctly uses PSP.

### Revised Root Cause (Correct)

**The frame pointer passed to `svc_return` is 4 bytes off.**

Evidence from GDB:
- KernelExceptionFrame at ~0x61830
- PSP slot (offset +0x20) contains `0x00000003` ← This is CONTROL, not PSP!
- CONTROL slot (offset +0x24) contains `0xfffffffd` ← This is EXC_RETURN, not CONTROL!

Everything is shifted by 4 bytes. When `svc_return` does:
```asm
pop     {{ r0 - r1, lr }}    // Expects: r0=PSP, r1=CONTROL, lr=EXC_RETURN
```

It actually gets:
- r0 = CONTROL (0x3) instead of PSP
- r1 = EXC_RETURN (0xfffffffd) instead of CONTROL
- lr = garbage instead of EXC_RETURN

Then when it does `msr psp, r0`, it sets PSP to 0x3 (invalid address), and subsequent user stack access fails.

### Why LR = 0x3 in User Frame?

After setting PSP=0x3 (CONTROL value) and CONTROL=0xfffffffd (EXC_RETURN):
```asm
pop     {r0-r3, r12, lr}     // Uses PSP (now 0x3 - invalid!)
```

This reads from address 0x3 which is in vector table region, causing the DACCVIOL fault. The `0x3` appearing in user LR is coincidental - it's garbage from wherever the corrupted PSP points.

---

## BUG FOUND! 🎯

### Location: `pw_kernel/macros/arm_cortex_m_macro.rs`

In `save_exception_frame()` for user_space mode (lines 191-202):
```rust
asm.push_str(
    "
    mrs     r1, control
    mrs     r0, psp
    push    {{ r0 - r1, lr }}

    push    {{ r4 - r11 }}
    mov     r0, sp
    sub     sp, 4   // Align stack to 8 bytes  ← UNCONDITIONAL!
    ",
);
```

In `restore_exception_frame()` (lines 210-225):
```rust
asm.push_str(
    "
    mov     sp, r0            // SP = returned frame pointer
    pop     {{ r4 - r11 }}    // Pop callee-saved

    pop     {{ r0 - r1 }}     // Pop PSP, CONTROL
    ...
```

### The Problem

**Save** (PendSV entry):
```asm
push    {{ r0 - r1, lr }}    // 12 bytes
push    {{ r4 - r11 }}       // 32 bytes
mov     r0, sp               // r0 = frame pointer (correct)
sub     sp, 4                // UNCONDITIONAL alignment
```

**Restore** (PendSV exit):
```asm
mov     sp, r0               // SP = returned frame pointer
                             // NO add sp, 4 to undo alignment!
pop     {{ r4 - r11 }}       // Pops from wrong offset!
```

The `sub sp, 4` is unconditional, but there's no corresponding `add sp, 4` in restore. The frame pointer `r0` returned by `pendsv_swap_sp()` points to the pre-alignment address, but restore doesn't account for the 4-byte gap.

### Why This Causes 4-byte Offset

1. Thread A makes syscall, SVCall saves frame at address X
2. Thread A blocks, PendSV saves frame pointer X to `active_thread.frame`
3. Later, PendSV restores Thread A, returns frame pointer X
4. BUT PendSV restore does `mov sp, r0` then immediately pops
5. The stack has 4 extra bytes from `sub sp, 4` that are never removed
6. All pops are 4 bytes off!

### Comparison with SVCall

SVCall uses **conditional** alignment:
```asm
ands    r3, r0, #0x4
it ne
subne   sp, 4             // Only subtracts IF misaligned
```

PendSV uses **unconditional** alignment:
```asm
sub     sp, 4             // ALWAYS subtracts
```

### The Fix

**Option A**: Remove the unconditional alignment (simplest)
```rust
// In save_exception_frame():
asm.push_str(
    "
    mrs     r1, control
    mrs     r0, psp
    push    {{ r0 - r1, lr }}
    push    {{ r4 - r11 }}
    mov     r0, sp
    // Remove: sub     sp, 4
    ",
);
```

**Option B**: Add `add sp, 4` in restore before popping
```rust
// In restore_exception_frame():
asm.push_str(
    "
    mov     sp, r0
    add     sp, 4           // Undo alignment from save
    pop     {{ r4 - r11 }}
    ...
```

**Option C**: Use conditional alignment like SVCall
```rust
// In save_exception_frame():
asm.push_str(
    "
    ...
    mov     r0, sp
    ands    r3, sp, #0x4
    it ne
    subne   sp, 4           // Conditional alignment
    ",
);
```

### Recommended Fix

**Option A** is simplest and safest. The alignment is only needed for calling C functions that require 8-byte stack alignment. Since `pendsv_swap_sp()` just saves/returns frame pointers, strict alignment may not be necessary.

If alignment IS needed for calling `pendsv_swap_sp()`, use **Option B**.


## Code Authorship

```
git blame: pw_kernel/arch/arm_cortex_m/syscall.rs
```

**Primary Author**: Erik Gilling (`konkers@google.com`)

**Key Commit**:
```
41e6a4517 - pw_kernel: Refactor ARM syscalls to eliminate race
Author: Erik Gilling <konkers@google.com>
Date: Fri Dec 5 15:40:26 2025 -0800
Bug: https://pwbug.dev/465499154
Reviewed-by: Travis Geiselbrecht <travisg@google.com>
```

The commit message mentions "eliminate race" - the refactor was addressing a timing/race condition but may have introduced this stack handling bug.

**Note**: The same `svc_return` code is used for both ARMv7-M and ARMv8-M (no `#[cfg(...)]` conditional compilation). This bug likely affects both architectures but may manifest differently depending on timing and memory layout.

---

## Implementation Plan for Fix

### Evidence from GDB Data

From the kernel stack dump at fault time:
```
0x61830: 0x00000001  0x000011c9  0x000618f0  0xffffffff   (r4, r5, r6, r7)
0x61840: 0x00040ffc  0xffffffff  0x00000000  0x0008ff80   (r8, r9, r10, r11)
0x61850: 0x00000003  0xfffffffd  ...                      (???, ???)
```

**KernelExceptionFrame struct layout:**
| Offset | Field | Expected | Actual at 0x61830 |
|--------|-------|----------|-------------------|
| +0x00 | r4 | - | 0x00000001 |
| +0x04 | r5 | - | 0x000011c9 |
| ... | ... | ... | ... |
| +0x1C | r11 | - | 0x0008ff80 |
| +0x20 | psp | PSP value | **0x00000003** ← WRONG! This is CONTROL |
| +0x24 | control | CONTROL | **0xfffffffd** ← WRONG! This is EXC_RETURN |
| +0x28 | return_address | EXC_RETURN | ??? |

**The frame is shifted by 4 bytes!** The PSP slot contains CONTROL (0x3), and the CONTROL slot contains EXC_RETURN (0xfffffffd).

### Root Cause: Stack Alignment in SVCall

Looking at SVCall entry code:
```asm
push    {{ r1 - r2, lr }}    // PSP, CONTROL, EXC_RETURN (12 bytes)
push    {{ r4 - r11 }}       // 32 bytes
mov     r0, sp               // r0 = frame pointer

// Arm exception frames need to be aligned to 8 bytes
ands    r3, r0, #0x4
it ne
subne   sp, 4                // ← ALIGNMENT ADJUSTMENT
```

**The Bug**: After capturing `r0 = sp`, the code adjusts SP for alignment but **r0 still points to the pre-alignment address**. When `handle_svc` returns this frame pointer to `svc_return`, the pointer is 4 bytes off if alignment was needed.

### The Fix

**Option 1: Capture frame pointer AFTER alignment**
```asm
push    {{ r1 - r2, lr }}
push    {{ r4 - r11 }}

// Align first
ands    r3, sp, #0x4
it ne
subne   sp, 4

// Then capture frame pointer
mov     r0, sp               // Now r0 points to aligned frame
```

**Problem**: This changes the frame pointer, but svc_return expects frame at r0. Need to also store alignment info.

**Option 2: Don't adjust SP, let the fake frame handle alignment**

The stack alignment is for the fake exception frame pushed after. The KernelExceptionFrame doesn't need 8-byte alignment for `handle_svc` - it's just a C function call.

```asm
push    {{ r1 - r2, lr }}
push    {{ r4 - r11 }}
mov     r0, sp               // r0 = frame pointer (don't touch after this)

// Align SP for fake exception frame, but don't change r0
ands    r3, sp, #0x4
it ne
subne   sp, 4

// Continue with fake frame push...
```

Wait - this is already what the code does! Let me re-read...

**Re-analysis**: The code DOES capture r0 before alignment. So the frame pointer is correct. But then why is the frame shifted?

### Alternative Theory: PendSV Interference

The fault happens during IPC, which involves thread switching via PendSV. If PendSV modifies the kernel stack or the frame pointer, that could cause the shift.

Looking at the stack at fault time:
- MSP = 0x61858
- Frame we're analyzing = 0x61830

The frame is 0x28 (40 bytes) below MSP. That's:
- 32 bytes (r4-r11) + 12 bytes (PSP, CONTROL, EXC_RETURN) = 44 bytes
- But we only see 40 bytes... **4 bytes missing!**

### Revised Theory: The Frame Pointer is Wrong

When `svc_return` receives the frame pointer in r0, it's not pointing to the start of KernelExceptionFrame - it's pointing 4 bytes into it!

**Scenario**:
1. SVCall pushes frame, captures `r0 = sp` (correct)
2. Something modifies r0 before/after handle_svc
3. Or PendSV returns a different frame pointer
4. svc_return pops from wrong offset

### Investigation: Check PendSV frame handling

The IPC test involves:
1. Thread A makes syscall (object_wait)
2. Thread A blocks, PendSV switches to Thread B
3. Thread B wakes Thread A
4. PendSV switches back to Thread A
5. Thread A's syscall returns via svc_return

**PendSV may be returning a different frame pointer than what SVCall saved.**

---

## ARM Architecture Analysis

### CONTROL Register Behavior

On both ARMv7-M and ARMv8-M:
- CONTROL.SPSEL bit selects which SP is used in **Thread mode only**
- SPSEL=0: Use MSP
- SPSEL=1: Use PSP
- Handler mode always uses MSP

In `svc_return`:
```asm
msr     psp, r0              // Set PSP register
msr     control, r1          // Set CONTROL (SPSEL=1)
dsb
isb
pop     {r0-r3, r12, lr}     // Should use PSP now (SPSEL=1, Thread mode)
```

After `msr control`, we're in Thread mode with SPSEL=1, so `pop` should use PSP. **This part is correct.**

### Verifying SVCall Push Order

```asm
// In SVCall:
mrs     r2, control
mrs     r1, psp
push    {{ r1 - r2, lr }}    // Push: lr, r2, r1 (descending addresses)
push    {{ r4 - r11 }}       // Push: r11..r4
mov     r0, sp               // r0 = frame pointer
```

`push {r1-r2, lr}` pushes registers in order of **decreasing register number** to **decreasing addresses**:
- First pushed (highest addr): lr
- Then: r2 (CONTROL)
- Last pushed (lowest addr): r1 (PSP)

After both pushes, stack layout from SP:
| Offset | Content |
|--------|---------|
| +0x00 | r4 |
| +0x04 | r5 |
| ... | ... |
| +0x1C | r11 |
| +0x20 | PSP (r1) |
| +0x24 | CONTROL (r2) |
| +0x28 | EXC_RETURN (lr) |

**This matches the KernelExceptionFrame struct!** SVCall's push logic is correct.

### Conclusion

The SVCall entry code correctly saves the frame. The `svc_return` code correctly uses PSP after setting CONTROL.

**The bug is in how the frame pointer is modified between SVCall and svc_return** - specifically in the PendSV context switch that happens when the thread blocks and is later resumed.

---

## Implementation Steps

1. [x] Verify SVCall push order matches KernelExceptionFrame struct ✅
2. [x] Verify svc_return SP handling after CONTROL switch ✅
3. [x] **Examine PendSV exception wrapper** - FOUND THE BUG! ✅
4. [ ] Fix the stack alignment in `pw_kernel/macros/arm_cortex_m_macro.rs`
5. [ ] Test on AST1030 QEMU
6. [ ] Test on MPS2-AN505 (ARMv8-M) to ensure no regression

### Key Files to Modify

- **`pw_kernel/macros/arm_cortex_m_macro.rs`** - `save_exception_frame()` unconditionally does `sub sp, 4` but restore doesn't undo it

## Next Steps

1. ✅ Identified root cause: Frame pointer is 4 bytes off when returned to svc_return
2. ✅ Evidence: PSP slot at +0x20 contains CONTROL (0x3), not actual PSP value
3. ✅ Verified SVCall and svc_return logic are correct
4. ✅ **Found bug in `arm_cortex_m_macro.rs`**: unconditional `sub sp, 4` without matching `add sp, 4` in restore
5. [ ] **Implement fix** - remove `sub sp, 4` or add `add sp, 4` in restore
6. [ ] Test on AST1030 and MPS2-AN505




