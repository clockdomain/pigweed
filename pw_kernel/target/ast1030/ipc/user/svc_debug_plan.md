# SVC/Syscall Debug Plan for ARMv7-M (AST1030/Cortex-M4)

## Context

User mode execution has been confirmed working (see [DEBUG_SESSION_RESULTS.md](DEBUG_SESSION_RESULTS.md)). The IPC test still times out, so the next investigation area is syscall/SVC handling from user mode.

**Key Finding:** The same IPC test PASSES on ARMv8-M (MPS2_AN505) but FAILS on ARMv7-M (AST1030). This points to an architecture-specific issue.

## Current Status (Updated)

### What We Know Works:
1. ✅ Kernel boots and initializes correctly
2. ✅ MPU configuration (PMSAv7) completes without errors
3. ✅ User threads are created with correct entry points (0x20001, 0x40001)
4. ✅ Context switch to user mode works (CONTROL=0x3 confirmed)
5. ✅ User threads start executing at `_start` entry point
6. ✅ Semihosting logging guard prevents hangs during PendSV

### What We Know Fails:
1. ❌ No output from user-mode `pw_log::info!` calls
2. ❌ SVCall breakpoints were NEVER hit during debug session
3. ❌ Test times out after ~8 seconds with no progress

### The Mystery:
User threads start running (confirmed via GDB breakpoints at `_start_initiator_0` and `_start_handler_1`), but they never make it to their first syscall (`pw_log::info!` → `debug_log` syscall). The SVCall exception handler is never triggered.

## Hypotheses

### Hypothesis 1: User Thread Crashes Before First Syscall
The user entry point `_start` calls:
1. `memcpy` - to initialize .data section
2. `memset` - to zero .bss section
3. `main` - user entry function

If `memcpy` or `memset` are not properly linked or jump to invalid addresses, the thread would crash. However, we're not seeing fault handler output.

**Test:** Set breakpoint after memcpy/memset calls, verify execution reaches main.

### Hypothesis 2: Silent Fault (MPU or Hard Fault)
A fault may be occurring but not producing output if:
- The fault handler itself faults (double fault → HardFault → lockup)
- MPU blocks access to fault handler code
- Stack corruption prevents fault handler execution

**Test:** Set breakpoints on HardFault, MemManage, BusFault handlers.

### Hypothesis 3: Infinite Loop in User Code
User code might be stuck in an infinite loop before making any syscall. Possible causes:
- Compiler-generated initialization code
- Library initialization (unlikely for no_std)

**Test:** Single-step through user code from `_start` to `main` to first syscall.

### Hypothesis 4: SVC Instruction Encoding Issue
ARMv7-M and ARMv8-M have the same SVC encoding, but there might be a subtle difference in how it's handled.

**Test:** Verify SVC instruction is present in user binary at expected location.

## Key Files

### SVC Handler (ARM Assembly)
- [pw_kernel/arch/arm_cortex_m/syscall.rs](../../../../arch/arm_cortex_m/syscall.rs)
  - `SVCall()` - naked exception handler entry (lines 98-176)
  - `handle_svc()` - kernel-side syscall processor (lines 248-267)
  - `svc_return()` - return trampoline to user mode (lines 189-246)

### Syscall Dispatch
- [pw_kernel/kernel/syscall.rs](../../../../kernel/syscall.rs)
  - `handle_syscall()` - dispatcher (lines 227-272)
  - `handle_channel_transact()` (lines 105-142)
  - `handle_channel_read()` (lines 144-161)
  - `handle_channel_respond()` (lines 163-179)

### User-Side Syscall Wrappers
- [pw_kernel/syscall/syscall_user/arm_cortex_m.rs](../../../../syscall/syscall_user/arm_cortex_m.rs)
  - `syscall_asm!()` macro - generates `svc 0` instructions

### IPC Test Code
- [pw_kernel/tests/ipc/user/initiator.rs](../../../../tests/ipc/user/initiator.rs)
  - Calls `channel_transact()` for each character
- [pw_kernel/tests/ipc/user/handler.rs](../../../../tests/ipc/user/handler.rs)
  - Loop: `object_wait()` → `channel_read()` → `channel_respond()`

## Syscall Flow Overview

```
User Code (unprivileged, Thread mode, PSP)
    │
    ▼
push {r4-r5, r11}
mov r11, <syscall_id>      ← Syscall ID in r11
svc 0                       ← Triggers SVCall exception
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ SVCall Exception Handler (Handler mode, MSP)            │
│                                                         │
│ 1. cpsid i                    Disable interrupts        │
│ 2. Save r4-r11 to MSP         Kernel exception frame    │
│ 3. Save PSP, CONTROL to MSP                             │
│ 4. Clear nPRIV bit            Elevate to privileged     │
│ 5. Push fake frame to PSP     For return to Thread mode │
│ 6. bx 0xFFFFFFFD              Return from exception     │
└─────────────────────────────────────────────────────────┘
    │
    ▼
handle_svc() (privileged, Thread mode, PSP)
    │
    ├── Extract syscall ID from r11
    ├── Extract args from r4-r7 (moved from r0-r3)
    ├── Call handle_syscall() dispatcher
    │       │
    │       ├── ObjectWait (0x0000)
    │       ├── ChannelTransact (0x0001)
    │       ├── ChannelRead (0x0002)
    │       └── ChannelRespond (0x0003)
    │
    └── Store result in r4-r5
    │
    ▼
svc_return() trampoline (privileged, Thread mode)
    │
    ├── Restore r4-r11 from frame
    ├── Pop PSP, CONTROL
    ├── msr PSP, <value>
    ├── msr CONTROL, 0x3        Set nPRIV + SPSEL
    ├── isb                     Instruction barrier
    └── bx <user_return_addr>   Return to user code
    │
    ▼
User Code continues (unprivileged, Thread mode, PSP)
```

## Phase 1: Verify First SVC from User Mode

After hitting the `_start_initiator_0` breakpoint (user mode entry), the initiator will call `channel_transact()` which invokes SVC.

### Setup

```gdb
# Find SVCall handler address
info functions SVCall

# Set breakpoint on SVCall
break SVCall

# Also set on handle_svc (the Rust function)
break handle_svc

# Continue from user entry
continue
```

### At SVCall Entry

```gdb
# Verify we're in SVCall handler
info registers

# Check:
# - xPSR IPSR field = 11 (SVCall exception number)
# - MSP points to exception frame
# - control = 0 (we're in handler mode)

p/x ($xpsr & 0x1FF)   # Should be 11

# Examine hardware exception frame on MSP
x/8xw $msp
# Expected layout:
# +0x00: r0  (arg0)
# +0x04: r1  (arg1)
# +0x08: r2  (arg2)
# +0x0C: r3  (arg3)
# +0x10: r12
# +0x14: lr  (return address in user code)
# +0x18: pc  (instruction after SVC)
# +0x1C: xpsr (bit 24 must be set for Thumb)
```

### Expected Values

| Register | Value | Notes |
|----------|-------|-------|
| `xpsr & 0x1FF` | `0x0B` (11) | SVCall exception |
| `control` | `0x0` | Handler mode, no nPRIV |
| `msp` | Valid kernel stack | Exception frame location |
| `exception_frame.pc` | `0x0002xxxx` | Return to initiator code |
| `exception_frame.xpsr` | `0x01000000` | Thumb bit set |

## Phase 2: Trace SVCall Assembly

```gdb
# Disassemble SVCall handler
disassemble SVCall

# Step through the assembly
stepi
stepi
# ...

# Watch for:
# 1. cpsid i (disable interrupts)
# 2. push {r4-r11} (save callee-saved regs)
# 3. push {r0, r1, lr} (save PSP, CONTROL, EXC_RETURN)
# 4. bic/msr to clear nPRIV
# 5. Push fake exception frame to PSP
# 6. bx 0xFFFFFFFD (exception return)
```

### Critical Check: Fake Exception Frame

The SVCall handler pushes a fake exception frame to the PSP (user stack) that will cause the exception return to land in `handle_svc`:

```gdb
# Just before exception return, check PSP
p/x $psp

# Examine the fake frame
x/8xw $psp
# +0x18: pc should point to handle_svc
# +0x1C: xpsr should be 0x01000000 (Thumb bit)
```

## Phase 3: Examine handle_svc Execution

```gdb
# Continue to handle_svc
continue

# At handle_svc entry:
info registers

# r0 contains pointer to KernelExceptionFrame
p/x $r0

# Examine the frame
x/11xw $r0
# Offset  Field
# +0x00   r4 (arg0, moved from r0)
# +0x04   r5 (arg1, moved from r1)
# +0x08   r6 (arg2, moved from r2)
# +0x0C   r7 (arg3, moved from r3)
# +0x10   r8
# +0x14   r9
# +0x18   r10
# +0x1C   r11 (syscall ID!)
# +0x20   psp
# +0x24   control
# +0x28   return_address (EXC_RETURN)

# Check syscall ID
set $frame = (unsigned int*)$r0
p/x $frame[7]   # r11 = syscall ID

# For ChannelTransact, should be 0x0001
```

### Syscall IDs

| ID | Name | Description |
|----|------|-------------|
| `0x0000` | ObjectWait | Wait for signals on object |
| `0x0001` | ChannelTransact | Send request, wait for response |
| `0x0002` | ChannelRead | Handler reads request |
| `0x0003` | ChannelRespond | Handler sends response |

## Phase 4: Trace Syscall Dispatch

```gdb
# Set breakpoint on dispatcher
break handle_syscall
continue

# Step through to see which handler is called
step
step

# For ChannelTransact:
break handle_channel_transact
continue

# Trace through channel logic
step
```

### Key Checks in handle_channel_transact

1. Object handle lookup succeeds
2. Channel state allows transaction
3. Request data copied correctly
4. Initiator marked as waiting
5. Handler signaled with READABLE

```gdb
# Watch for signal calls
break signal
commands
  silent
  printf "Signal: signals=%x\n", $r1
  continue
end
```

## Phase 5: Verify SVC Return to User Mode

```gdb
# Set breakpoint on svc_return
break svc_return
continue

# Step through the return sequence
disassemble svc_return

stepi   # Restore r4-r11
stepi   # Pop psp, control
stepi   # msr psp, r0
stepi   # orr r1, 0x3 (set nPRIV + SPSEL)
stepi   # msr control, r1
stepi   # isb
stepi   # pop registers
stepi   # bx to user code

# Verify we're back in user mode
info registers
p/x $control   # Should be 0x3
p/x $pc        # Should be 0x0002xxxx (initiator)
p/x ($xpsr & 0x1FF)  # Should be 0 (Thread mode)
```

## Phase 6: Check for Timeout Root Cause

If syscalls work but test still times out:

### Is Handler Thread Blocked?

```gdb
# Handler should be in object_wait() waiting for READABLE
# Check scheduler state

# Find handler thread
info threads

# Check if handler is runnable or blocked
```

### Is Signal Delivery Working?

```gdb
# When initiator calls channel_transact:
# 1. Initiator's WRITEABLE should be cleared
# 2. Handler's READABLE should be set
# 3. Handler should wake up

# Set watchpoint on handler's signals
# (need to find handler object address first)
```

### Is Context Switch Happening?

```gdb
# The transaction requires:
# 1. Initiator makes syscall
# 2. Initiator blocks
# 3. PendSV triggered
# 4. Handler runs
# 5. Handler makes syscall (object_wait or channel_read)
# 6. Handler responds
# 7. Initiator unblocks
# 8. PendSV triggered
# 9. Initiator continues

break PendSV
continue
# Should hit PendSV after initiator blocks
```

## Quick Debug Commands

```gdb
# Source the helper script
source pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb

# Set all SVC-related breakpoints
break SVCall
break handle_svc
break svc_return
break handle_syscall
break handle_channel_transact
break handle_channel_read
break handle_channel_respond
break handle_object_wait

# User entry points
break *0x00020000
break *0x00040000

# Continue and observe
continue
```

## Expected IPC Flow

1. **Initiator** calls `channel_transact("a", ...)`
2. **SVCall** → handle_channel_transact
3. **Initiator** blocks waiting for response
4. **PendSV** → context switch to handler
5. **Handler** `object_wait()` returns (READABLE set)
6. **Handler** calls `channel_read()` → gets "a"
7. **Handler** calls `channel_respond("A")`
8. **Initiator** READABLE signal set
9. **PendSV** → context switch to initiator
10. **Initiator** unblocks, returns with "A"

If any step fails, the test times out.

## Debugging Checklist

- [ ] SVCall exception triggers correctly
- [ ] Syscall ID extracted from r11 correctly
- [ ] Arguments passed in r4-r7 (not r0-r3)
- [ ] Dispatcher routes to correct handler
- [ ] Channel state machine transitions properly
- [ ] Signals delivered between threads
- [ ] Context switch (PendSV) happens when thread blocks
- [ ] svc_return restores CONTROL=0x3 properly
- [ ] Return to user code at correct address
- [ ] User code continues after syscall

## Related Documentation

- [DEBUG_SESSION_RESULTS.md](DEBUG_SESSION_RESULTS.md) - User mode entry verified
- [armv7m_usermode_debug_plan.md](armv7m_usermode_debug_plan.md) - General debug setup
- [debug_usermode.gdb](debug_usermode.gdb) - GDB helper script

---

## NEXT DEBUG SESSION: Trace User Thread Execution

### Goal
Determine exactly where user threads get stuck between `_start` entry and first syscall.

### Setup

```bash
# Terminal 1: Start QEMU with GDB server
bazel run --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc -- -S -s

# Terminal 2: Connect GDB
gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
(gdb) target remote :1234
```

### Phase A: Set Up Comprehensive Breakpoints

```gdb
# Exception handlers (to catch any faults)
break HardFault
break MemoryManagement
break BusFault
break UsageFault

# User entry points
break *0x00020000
break *0x00040000

# Key functions in user binary - find addresses first
info functions memcpy
info functions memset
info functions main
info functions _start_entry

# SVCall (should NOT be hit based on previous session)
info functions SVCall
break SVCall
```

### Phase B: Run to User Entry, Then Single-Step

```gdb
# Continue to first user thread entry
continue

# When at _start (0x20000 or 0x40000):
info registers

# Disassemble to see what code we're about to execute
disassemble $pc,+64

# Single-step through the initialization
stepi
stepi
# ... continue stepping

# Watch for:
# 1. bl memcpy - does it branch to valid address?
# 2. bl memset - does it branch to valid address?
# 3. bl main   - does it reach main?
```

### Phase C: Check memcpy/memset Addresses

```gdb
# At the "bl memcpy" instruction:
x/i $pc

# Step into the branch
stepi

# Verify we're in valid code
info registers
disassemble $pc,+32

# If PC is in an unexpected location, that's the bug!
```

### Phase D: Trace to First Syscall

If execution reaches `main`:

```gdb
# Find the first syscall instruction
# User code calls pw_log::info! which calls debug_log syscall

# Look for SVC instruction in user binary
find /b 0x20000, 0x40000, 0xdf, 0x00  # SVC #0 encoding: 0xDF00

# Set breakpoint just before SVC
# (address found from disassembly)
break *<address_before_svc>
continue

# When hit, verify we're about to execute SVC
x/i $pc
stepi  # Should trigger SVCall exception
```

### Phase E: If Fault Occurs

```gdb
# At fault handler breakpoint:
info registers

# Check fault status registers
# CFSR (Configurable Fault Status Register) at 0xE000ED28
x/w 0xE000ED28

# HFSR (HardFault Status Register) at 0xE000ED2C
x/w 0xE000ED2C

# MMFAR (MemManage Fault Address) at 0xE000ED34
x/w 0xE000ED34

# BFAR (BusFault Address) at 0xE000ED38
x/w 0xE000ED38

# Decode CFSR bits:
# Bits 0-7: MMFSR (MemManage)
# Bits 8-15: BFSR (BusFault)
# Bits 16-31: UFSR (UsageFault)
```

### Expected Outcome

One of these should happen:
1. **Fault breakpoint hit** → Decode fault registers to find cause
2. **Execution stuck in loop** → Identify which instruction/function
3. **SVCall hit** → Previous theory wrong, issue is in syscall handling
4. **User main reached** → Issue is after main, before first syscall

### Quick Reference: User Binary Layout (AST1030)

```
0x00020000 - 0x0003FFFF: Initiator app code (128KB)
0x00040000 - 0x0005FFFF: Handler app code (128KB)
0x00080000 - 0x0008FFFF: Initiator app RAM (32KB)
0x00088000 - 0x0008FFFF: Handler app RAM (32KB)
```

### Quick Reference: ARMv7-M Exception Numbers

| Number | Exception |
|--------|-----------|
| 3 | HardFault |
| 4 | MemManage |
| 5 | BusFault |
| 6 | UsageFault |
| 11 | SVCall |
| 14 | PendSV |
| 15 | SysTick |
