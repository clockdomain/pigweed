# AST1030 GDB Debug Session Results

**Date:** January 22, 2026  
**Binary:** `bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf`

---

## Breakpoints Set

| # | Address | Symbol | Purpose |
|---|---------|--------|---------|
| 10 | 0x00000494 | `SVCall+56` | Syscall handler |
| 11 | 0x000005da | `PendSV+18` | Context switch |
| 12 | 0x00001f72 | `pendsv_swap_sp+20` | Fix code location |
| 13 | 0x0000057c | `MemoryManagement+4` | Fault handler |

---

## Execution Trace

### Hit 1: PendSV (Breakpoint 11)

**Location:** `0x000005da` in `PendSV()`

**Register State:**
```
r0             0x7fedc              523996
r1             0x0                  0
r2             0x0                  0
r3             0x0                  0
r4             0x7ff80              524160
r5             0x60238              393784
r6             0x600ac              393388
r7             0x7ff78              524152
r8             0x0                  0
r9             0x0                  0
r10            0x7ffa4              524196
r11            0x60020              393248
r12            0xf                  15
sp             0x7fed8              0x7fed8
lr             0xfffffff9           -7
pc             0x5da                0x5da <PendSV+18>
xpsr           0x4100000e           1090519054
msp            0x7fed8              523992
psp            0x0                  0          ← PSP is zero!
primask        0x0                  0
control        0x0                  0          ← Privileged, MSP
basepri        0x0                  0
faultmask      0x0                  0
```

**Analysis:**
- `lr = 0xFFFFFFF9` → EXC_RETURN: Return to **Handler mode using MSP**
- `psp = 0x0` → No user stack yet (PSP uninitialized)
- `control = 0x0` → Privileged mode, using MSP

**Interpretation:** This is the **kernel bootstrap** - first context switch from kernel idle to first user thread. No user thread has run yet.

---

### Hit 2: pendsv_swap_sp (Breakpoint 12)

**Location:** `0x00001f72` in `pendsv_swap_sp()`

**Key Finding:** ✅ **`pendsv_swap_sp` IS being called!**

This disproves our earlier hypothesis that `pendsv_swap_sp` was never called. The debug logging we added earlier must not have been working due to semihosting issues or output buffering.

---

## Key Discoveries

### 1. PendSV IS Firing ✅
Contrary to earlier assumptions, PendSV exception is being triggered and the handler is executing.

### 2. pendsv_swap_sp IS Being Called ✅
The function containing our CONTROL fix code is being reached.

### 3. Initial Context Switch State
The first PendSV occurs with:
- `CONTROL = 0x0` (kernel mode)
- `PSP = 0x0` (uninitialized)
- `LR = 0xFFFFFFF9` (handler mode return)

This is the kernel bootstrapping the first user thread.

---

## Next Steps

1. **Continue execution** to see if more PendSV/pendsv_swap_sp hits occur
2. **Check if MemoryManagement fault is hit** and capture the state at that point
3. **Verify the fix code is executed** by examining the `canonical_control` value being restored
4. **Check CONTROL value** when returning to user thread

### GDB Commands to Run

```gdb
# Continue to next breakpoint
continue

# When fault hits, examine state:
info registers
x/x 0xE000ED28    # CFSR - Configurable Fault Status Register
x/x 0xE000ED34    # MMFAR - MemManage Fault Address Register

# Check the exception frame on stack
x/8x $sp
```

---

## Updated Understanding

The debug plan summary was incorrect about `pendsv_swap_sp` never being called. The issue is likely:

1. **Semihosting output not working** during exception handlers
2. Or the **fault occurs on a specific context switch** (not the first one)
3. Or the **fix code runs but doesn't prevent the fault** for some other reason

Need to continue the debug session to see when the MemoryManagement fault occurs.
