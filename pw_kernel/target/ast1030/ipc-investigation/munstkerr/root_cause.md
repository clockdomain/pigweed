# AST1030 object_wait / Syscall Fault Analysis

## Problem Summary

**MemManage fault (MUNSTKERR)** is a Heisenbug. The system crashes after ~34 syscalls with a corrupted kernel exception frame.

## ROOT CAUSE IDENTIFIED (2026-01-24)

### The Bug: Wrong Frame Pointer Passed to svc_return

**The frame pointer (r0) passed to `svc_return` is WRONG.** The actual valid frame is 32 bytes (0x20) below where MSP points.

### Memory Layout at Fault
```
Address    Offset   Content      Interpretation
─────────────────────────────────────────────────
0x60c10    -0x30    0x00020df1   r4 (previous frame)
0x60c14    -0x2c    0x00000010   r5
0x60c18    -0x28    0x0000005b   r6
0x60c1c    -0x24    0x00087ed0   r7
0x60c20    -0x20    0x00000003   r8 ← VALID FRAME STARTS HERE
0x60c24    -0x1c    0x00087fd8   r9
0x60c28    -0x18    0x00000005   r10
0x60c2c    -0x14    0x00087ee8   r11
0x60c30    -0x10    0x00087e68   psp (bad PSP value!)
0x60c34    -0x0c    0x00000003   control (VALID!)
0x60c38    -0x08    0xfffffffd   return (VALID EXC_RETURN!)
─────────────────────────────────────────────────
0x60c40    MSP→     0x00060c64   ← svc_return gets THIS (garbage!)
0x60c44             0x00087f98
0x60c48             0x00000000
...
```

### Proof
- `svc_return` was called with `r0 = 0x60c40` (from MSP)
- The **actual valid frame** with correct `control=0x3` and `return=0xfffffffd` is at **0x60c20**
- The frame is **0x20 bytes (32 bytes) BELOW** where it should be
- This means **the frame pointer calculation is off by the size of the frame itself**

### Why PSP is Invalid
Even the "real" frame at 0x60c20 has `psp = 0x87e68`, which is below the user stack base (`0x88000`). This is a **secondary bug** - the user stack has underflowed OR the PSP was saved incorrectly.

## GDB Evidence

### Memory Dump at MSP-0x40
```
0x60c20: 0x00087ed0  0x00000003  0x00087fd8  0x00000005
0x60c30: 0x00087ee8  0x00087e68  0x00000003  0xfffffffd  ← Valid frame!
0x60c40: 0x00060c64  0x00087f98  0x00000000  0x21000000  ← MSP points here (garbage)
```

### Frame at 0x60c20 (the REAL frame)
| Offset | Field | Value | Status |
|--------|-------|-------|--------|
| +0x00 | r4 | 0x00087ed0 | OK (stack addr) |
| +0x04 | r5/r8? | 0x00000003 | OK |
| +0x08 | r6/r9? | 0x00087fd8 | OK (stack addr) |
| +0x0c | r7/r10? | 0x00000005 | OK |
| +0x10 | r11 | 0x00087ee8 | OK (stack addr) |
| +0x14 | psp | 0x00087e68 | **BAD** (below 0x88000) |
| +0x18 | control | 0x00000003 | ✓ Valid |
| +0x1c | return | 0xfffffffd | ✓ Valid EXC_RETURN |

## Suspect Code: Frame Pointer Calculation

The bug is in how the frame pointer is passed to `svc_return`. Check:

1. **`handle_svc` return value** - Does it return the correct frame pointer?
2. **SVCall handler epilogue** - Does it set r0 correctly before calling svc_return?
3. **Stack adjustment** - Is there an extra push/pop that offsets MSP?

