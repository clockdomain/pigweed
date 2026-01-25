# Next GDB Session Plan - 2026-01-24 Session B

## Lesson from Session A

**Breakpoints change timing too much.** With SVCall/handle_svc/svc_return breakpoints:
- System hangs or shows early wakeup behavior
- Never reaches the MUNSTKERR/DACCVIOL fault

**Need minimal-intrusion approach.**

## Objective

Catch the moment when kernel address 0x60850 gets written to a location where user code will read it.

## Session Setup

```gdb
# Start fresh - delete ALL breakpoints
delete

# Load symbols
file bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

# Connect
target remote :1234
```

## Step 1: Set Hardware Watchpoint

```gdb
# Watch for when kernel address 0x60850 gets written somewhere
# This is the address that ended up in user r1/r8
# Hardware watchpoints don't slow execution as much as breakpoints
watch *(unsigned int*)0x60850
```

## Step 2: Set MemoryManagement Breakpoint (backup)

```gdb
# Catch the fault if watchpoint doesn't trigger first
break *MemoryManagement
```

## Step 3: Run

```gdb
continue
```

## What to Look For

### If Watchpoint Triggers

```gdb
# Where are we?
info registers pc lr
bt

# What was written?
x/wx 0x60850

# What's the context?
info registers msp psp
```

### If MemoryManagement Triggers First

```gdb
# Dump fault info
check_mmfault

# Check user exception frame
x/8wx $psp

# Look for kernel pointers in user regs
info registers r0 r1 r2 r3 r8 r9
```

## Alternative: Watch User Stack Region

If 0x60850 watchpoint doesn't help, try watching where the user exception frame is:

```gdb
# User PSP was around 0x8fea8 at fault
# Watch the r1 slot in exception frame (offset +4)
watch *(unsigned int*)0x8feac
```

## Alternative: Minimal Single Breakpoint

If watchpoints don't work on QEMU, use just ONE breakpoint:

```gdb
delete
break *MemoryManagement
continue
```

Then when fault hits, inspect:
```gdb
# User exception frame at PSP
x/8wx $psp

# Kernel stack
x/32wx ($msp - 0x40)

# Look for the valid frame 0x20 below
```

## Expected Fault Signature

From previous sessions:
- MMFSR = 0x82 (DACCVIOL) or 0x08 (MUNSTKERR)
- MMFAR = 0x60850 (kernel data)
- User r1/r8 = 0x60850 (should be user data)
- User LR = 0x3 (corrupted - CONTROL value)

## Key Question to Answer

**Where does 0x60850 come from?**

Possibilities:
1. Frame restore pulls from wrong offset (0x20 off)
2. PendSV saves wrong frame address
3. Stack corruption overwrites user exception frame

## Notes

- Hardware watchpoints are limited (usually 2-4 on Cortex-M)
- QEMU may not support all watchpoint features
- If watchpoint doesn't work, fall back to single breakpoint approach
