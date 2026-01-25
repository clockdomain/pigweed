# GDB Debug Session Analysis - DACCVIOL Fault

## Session Date: 2026-01-24

## Summary

| Fault | Type | Cause |
|-------|------|-------|
| MMFSR=0x82 | DACCVIOL | User code accessing kernel memory |
| MMFAR | 0x60850 | Kernel static data address |
| Faulting PC | 0x408d4 | `ldr.w r0, [r1], #16` |
| r1 | 0x60850 | **Kernel pointer in user register!** |
| r8 | 0x60850 | Same kernel pointer |
| LR (in frame) | 0x00000003 | **Corrupted** (should be return address) |

## Setup

Ran IPC test under GDB with only MemoryManagement breakpoint (no SVCall breakpoints to preserve timing):

```gdb
delete
break MemoryManagement
continue
```

## Fault Captured

### MemManage Fault Status
```
MMFSR: 0x82
  MMARVALID: MMFAR holds valid address
  DACCVIOL: Data access violation
MMFAR: 0x00060850
```

**This is NOT MUNSTKERR** - it's a data access violation. User code tried to access kernel memory.

### Register State at Fault
```
PC:  0x000005a0  (MemoryManagement handler)
LR:  0xfffffffd  (EXC_RETURN - Thread/PSP)
MSP: 0x000617b8
PSP: 0x0008fea8
CONTROL: 0x00000001
ICSR: 0x00000804 (active=4, pending=0)

r0:  0x00000000  r1:  0x00000001  r2:  0x0008fed8  r3:  0x0008fedc
r4:  0x0008ff48  r5:  0x00000000  r6:  0x00000004  r7:  0x0008ffb0
r8:  0x00060850  ← KERNEL ADDRESS IN USER REGISTER!
r9:  0x00060870  ← KERNEL ADDRESS IN USER REGISTER!
r10: 0x00000004  r11: 0x0008fed8
r12: 0x0008fed8
```

### User Exception Frame (at PSP 0x8fea8)
```
0x8fea8:  0x00000000  ← r0
0x8feac:  0x00060850  ← r1 (KERNEL POINTER!)
0x8feb0:  0x0008fed8  ← r2
0x8feb4:  0x0008fedc  ← r3
0x8feb8:  0x0008fed8  ← r12
0x8febc:  0x00000003  ← LR (CORRUPTED! Should be return address)
0x8fec0:  0x000408d4  ← PC (faulting instruction)
0x8fec4:  0x61000000  ← xPSR
```

### Faulting Instruction
```asm
0x408d4:  ldr.w  r0, [r1], #16    ← FAULT HERE (r1=0x60850)
0x408d8:  str    r1, [sp, #8]
0x408da:  cmp    r0, #1
0x408dc:  bne.n  0x40960
0x408de:  ldrd   r0, r2, [r8, #8]  ← Would also fault (r8=0x60850)
```

## Analysis

### Key Evidence

| Field | Value | Problem |
|-------|-------|---------|
| r1 | 0x00060850 | Kernel address in user register |
| r8 | 0x00060850 | Same kernel address |
| LR (frame) | 0x00000003 | Corrupted - looks like CONTROL value |
| MMFAR | 0x00060850 | Faulting address |

### What 0x60850 Is
```
0x00060850  codegen::start::__init_non_priv_thread::__STATIC
```
This is kernel static data - thread control structures.

### Root Cause

After returning from syscall, user registers contain **kernel addresses** instead of the user's saved register values.

The LR=0x3 in the exception frame is definitive proof of corruption:
- 0x3 = CONTROL register value (nPRIV=1, SPSEL=1)
- This was placed in the LR slot, indicating frame fields are shifted/corrupted

## Conclusion

The `svc_return` trampoline is restoring the **wrong frame** or an **offset within a frame**:
- User gets kernel pointers in registers
- Exception frame fields are shifted (CONTROL → LR slot)
- When user code dereferences these pointers → DACCVIOL

This is the **same underlying bug** as the MUNSTKERR:
- MUNSTKERR: Corrupt PSP causes unstacking failure
- DACCVIOL: Corrupt registers cause user to access kernel memory

Both caused by frame restoration bug in syscall return path.

## Files to Investigate

1. `pw_kernel/arch/arm_cortex_m/syscall.rs` - `svc_return` trampoline
2. Frame pointer calculation after `handle_svc` returns
3. Stack alignment adjustment that doesn't update r0
