# ARMv7-M User Mode Debug Session Results

## Date: 2026-01-17

## Summary

**User mode execution works correctly on ARMv7-M (AST1030/Cortex-M4).**

The initial hypothesis that "user code never executes" was incorrect. GDB debugging confirmed that:
1. Exception frames are set up correctly
2. Exception return to user mode succeeds
3. User code entry points are reached with correct privilege level

## Debug Session Findings

### Phase 1: KernelExceptionFrame Verification

At PendSV return (`0x000005e0`), the KernelExceptionFrame at `0x60ab0` contained:

| Offset | Field | Value | Status |
|--------|-------|-------|--------|
| +0x00-0x1C | r4-r11 | `0x00000000` | Zeroed for new thread |
| +0x20 | PSP | `0x00087fe0` | Valid user stack pointer |
| +0x24 | CONTROL | `0x00000003` | nPRIV=1, SPSEL=1 (correct) |
| +0x28 | EXC_RETURN | `0xfffffffd` | Thread mode, PSP (correct) |

### Phase 2: User ExceptionFrame Verification

The hardware exception frame at PSP (`0x87fe0`) contained:

| Offset | Field | Value | Status |
|--------|-------|-------|--------|
| +0x00-0x14 | r0-r3, r12, lr | `0x00000000` | Zeroed |
| +0x18 | PC | `0x00020001` | User entry with Thumb bit |
| +0x1C | xPSR | `0x01000000` | Thumb bit (bit 24) set |

### Phase 3: Exception Return Trace

Just before `pop {pc}` at `0x000005f2`:

| Register | Value | Notes |
|----------|-------|-------|
| `r0` | `0x87fe0` | PSP value to restore |
| `r1` | `0x3` | CONTROL value to restore |
| `psp` | `0x87fe0` | Already set |
| `control` | `0x1` | SPSEL not yet visible (normal - needs ISB or exception return) |
| Stack top | `0xfffffffd` | EXC_RETURN value |

### Phase 4: User Mode Entry Confirmed

Breakpoint hit at `0x00020000` (`_start_initiator_0`):

| Register | Value | Interpretation |
|----------|-------|----------------|
| `pc` | `0x20000` | User entry point |
| `control` | `0x3` | Unprivileged + using PSP |
| `psp` | `0x88000` | User stack |
| `xpsr` | `0x1000000` | Thread mode (IPSR=0), Thumb bit set |
| `msp` | `0x60adc` | Kernel stack (preserved) |

## Observations

### SysTick Preemption

During debugging, SysTick occasionally preempted the PendSV exception return sequence. This is because `cpsie i` enables interrupts before `pop {pc}`. However, this does not prevent user mode execution - it just delays it by one more exception cycle.

### CONTROL Register Behavior

The CONTROL register showed `0x1` (nPRIV=1, SPSEL=0) just before exception return, even though `0x3` was written. This is expected ARMv7-M behavior:
- SPSEL changes don't take effect until ISB or exception entry/return
- The exception return uses EXC_RETURN to determine which stack to use, not CONTROL.SPSEL
- After exception return, CONTROL correctly shows `0x3`

## Conclusions

1. **ARMv7-M user mode transition works correctly** - The kernel properly sets up exception frames and executes exception return to user mode.

2. **The original test timeout is NOT caused by user mode execution failure** - User code entry points are reached.

3. **Possible remaining issues to investigate:**
   - Syscall/SVC handling from user mode
   - IPC message passing between user processes
   - Semihosting/console output from unprivileged mode
   - Potential infinite loops or deadlocks in user code or IPC

## Commands Used

```gdb
# Connect to QEMU
target remote :3333

# Set breakpoints
break HardFault
break MemoryManagement
break UsageFault
break BusFault
break PendSV
break pendsv_swap_sp
break *0x00020000  # initiator entry
break *0x00040000  # handler entry

# Examine KernelExceptionFrame
p/x $r0
x/11xw $r0

# Examine user ExceptionFrame
x/8xw 0x00087fe0

# Check state at exception return
break *0x000005f2  # pop {pc}
info registers
p/x $psp
p/x $control
x/1xw $sp

# Single step through exception return
stepi
info registers
```

## Files

- [debug_usermode.gdb](debug_usermode.gdb) - GDB script with helper functions
- [../../../docs/armv7m_usermode_debug_plan.md](../../../docs/armv7m_usermode_debug_plan.md) - Full debug plan with ARMv7-M background
