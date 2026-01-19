# ARMv7-M User Mode Execution Debugging Plan

## ARMv7-M Architecture Background

Before diving into debugging, it's essential to understand how ARMv7-M handles
the transition from kernel (privileged) mode to user (unprivileged) mode. This
context explains why we're checking specific registers and memory locations.

### Execution Modes and Privilege Levels

ARMv7-M (Cortex-M3/M4/M7) has two execution modes and two privilege levels:

```
┌─────────────────────────────────────────────────────────────┐
│                    Execution Modes                          │
├─────────────────────────────┬───────────────────────────────┤
│        Handler Mode         │         Thread Mode           │
│   (Exception/Interrupt)     │      (Normal execution)       │
│                             │                               │
│   - Always privileged       │   - Can be privileged or      │
│   - Uses MSP (Main Stack)   │     unprivileged              │
│   - Entered via exception   │   - Uses MSP or PSP           │
│                             │   - Where user code runs      │
└─────────────────────────────┴───────────────────────────────┘
```

**Key insight:** The kernel runs in Thread Mode (privileged), and user code
runs in Thread Mode (unprivileged). The transition happens via an exception
return, not a direct jump.

### The CONTROL Register

The CONTROL register determines privilege level and stack pointer selection
in Thread Mode:

```
CONTROL Register (ARMv7-M):
┌────────┬────────┬────────┐
│ Bit 2  │ Bit 1  │ Bit 0  │
│ FPCA   │ SPSEL  │ nPRIV  │
└────────┴────────┴────────┘

nPRIV (bit 0):
  0 = Thread mode is privileged (kernel)
  1 = Thread mode is unprivileged (user)

SPSEL (bit 1):
  0 = Use MSP (Main Stack Pointer) in Thread mode
  1 = Use PSP (Process Stack Pointer) in Thread mode

FPCA (bit 2):
  Floating-point context active (not relevant here)
```

For user mode, we need CONTROL = 0x3 (nPRIV=1, SPSEL=1).

### Exception Entry and Return

When an exception occurs (like PendSV for context switching):

**Entry (hardware automatic):**
1. Push exception frame to current stack: {r0-r3, r12, lr, pc, xpsr}
2. Switch to Handler mode
3. Switch to MSP
4. Load exception vector into PC

**Return (triggered by special LR value):**
1. Recognize EXC_RETURN value in PC/LR
2. Pop exception frame from appropriate stack
3. Restore processor state
4. Continue execution at restored PC

```
Exception Frame (pushed/popped by hardware):
┌─────────┐ ← Higher address
│  xPSR   │ +0x1C  (includes Thumb bit)
│   PC    │ +0x18  (return address)
│   LR    │ +0x14
│   R12   │ +0x10
│   R3    │ +0x0C
│   R2    │ +0x08
│   R1    │ +0x04
│   R0    │ +0x00  ← Stack pointer points here
└─────────┘
```

### EXC_RETURN: The Magic Value

EXC_RETURN is a special value loaded into LR on exception entry. When this
value is loaded into PC (via `bx lr` or `pop {pc}`), it triggers exception
return instead of a normal branch.

```
EXC_RETURN format (ARMv7-M):
0xFFFFFFF_
         │
         └─ Low 4 bits determine return behavior:

0xFFFFFFF1 = Return to Handler mode, use MSP
0xFFFFFFF9 = Return to Thread mode, use MSP
0xFFFFFFFD = Return to Thread mode, use PSP  ← User mode needs this
```

### How User Mode Transition Works

To switch from kernel to user mode, we:

1. **Prepare the user exception frame** on the user's PSP:
   - Set PC to user entry point
   - Set xPSR with Thumb bit (bit 24) set
   - Set up arguments in r0-r3

2. **Prepare the kernel exception frame** (KernelExceptionFrame):
   - Save callee-saved registers (r4-r11)
   - Save PSP value (pointing to user exception frame)
   - Save CONTROL value (0x3 for user mode)
   - Save EXC_RETURN value (0xFFFFFFFD)

3. **Trigger exception return**:
   - Restore r4-r11 from kernel frame
   - Write PSP from kernel frame
   - Write CONTROL from kernel frame
   - Load EXC_RETURN into PC (triggers hardware unstacking)

4. **Hardware does the rest**:
   - Recognizes EXC_RETURN value
   - Pops exception frame from PSP (because SPSEL bit in EXC_RETURN)
   - Switches to Thread mode
   - Jumps to PC from exception frame

```
Before Exception Return:          After Exception Return:
┌──────────────────────┐          ┌──────────────────────┐
│ Handler Mode         │          │ Thread Mode          │
│ Privileged           │          │ Unprivileged         │
│ Using MSP            │    →     │ Using PSP            │
│ PC in kernel code    │          │ PC in user code      │
│ CONTROL = 0x0        │          │ CONTROL = 0x3        │
└──────────────────────┘          └──────────────────────┘
```

### Why This Debug Plan?

The debugging phases follow the data flow:

1. **Phase 1-2**: Verify we reach the context switch point
2. **Phase 3**: Check the KernelExceptionFrame has correct values
3. **Phase 4**: Check the user ExceptionFrame has correct values
4. **Phase 5**: Watch the actual exception return happen
5. **Phase 6**: Catch any faults that occur immediately after
6. **Phase 7**: Verify MPU allows user code execution
7. **Phase 8**: Compare with working ARMv8-M to spot differences

The bug must be in one of these stages - either we're setting up the frames
wrong, or the exception return mechanism isn't working as expected on ARMv7-M.

### Key Differences: ARMv7-M vs ARMv8-M

| Aspect | ARMv7-M | ARMv8-M |
|--------|---------|---------|
| CONTROL bits | 3 bits (0-2) | 8 bits (0-7) |
| TrustZone | No | Yes (S bit in EXC_RETURN) |
| MPU | PMSAv7 (power-of-2 regions) | PMSAv8 (arbitrary regions) |
| EXC_RETURN | 4-bit encoding | 8-bit encoding |

The code currently uses ARMv8-M style EXC_RETURN construction, but calculates
to the same value (0xFFFFFFFD) for both architectures. However, there may be
subtle differences in how the hardware interprets these values.

---

## Problem Summary

| Aspect | Observation |
|--------|-------------|
| **Symptom** | User code never executes on ARMv7-M targets |
| **Context switches** | 30,000+ switches observed, kernel works correctly |
| **User thread setup** | CONTROL=0x3, EXC_RETURN=0xFFFFFFFD (correct values) |
| **ARMv8-M (MPS2-AN505)** | Works correctly |
| **ARMv7-M (AST1030, LM3S6965)** | Fails - no user output |

## Hypothesis

The exception return to user mode is failing silently. Possible causes:
1. Exception return goes to wrong address
2. Immediate fault after return (silent or mishandled)
3. Stuck in exception handler (never returns to user mode)
4. CONTROL register not taking effect properly
5. MPU misconfiguration blocking code execution

## Debugging Environment Setup

### Prerequisites

```bash
# Ensure you have multiarch GDB
gdb-multiarch --version

# Build the IPC test (ipc_test depends on ipc, which produces the ELF)
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc --config=k_qemu_ast1030
```

### Option A: Using tmux (Single Terminal)

tmux lets you run QEMU and GDB side-by-side in one terminal:

```bash
# Start a new tmux session
tmux new-session -s debug

# Split the window vertically (left/right panes)
# Press: Ctrl+b %

# In left pane: Start QEMU
qemu-system-arm \
    -cpu cortex-m4 \
    -machine ast1030-evb \
    -nographic \
    -semihosting-config enable=on,target=native \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
    -S -gdb tcp::3333

# Switch to right pane: Ctrl+b <arrow-right>

# In right pane: Start GDB
gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -ex "target remote :3333"
```

**tmux quick reference:**
| Key | Action |
|-----|--------|
| `Ctrl+b %` | Split vertically (left/right) |
| `Ctrl+b "` | Split horizontally (top/bottom) |
| `Ctrl+b <arrow>` | Move between panes |
| `Ctrl+b z` | Zoom current pane (toggle fullscreen) |
| `Ctrl+b d` | Detach from session |
| `tmux attach -t debug` | Reattach to session |

### Option B: Two Separate Terminals

#### Terminal 1: Start QEMU with GDB Server

```bash
qemu-system-arm \
    -cpu cortex-m4 \
    -machine ast1030-evb \
    -nographic \
    -semihosting-config enable=on,target=native \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
    -S -gdb tcp::3333
```

**Flags explained:**
- `-S`: Start paused (don't execute until GDB continues)
- `-gdb tcp::3333`: Listen for GDB on port 3333
- `-semihosting-config enable=on,target=native`: Enable semihosting for printf

#### Terminal 2: Connect GDB

```bash
gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
```

```gdb
(gdb) target remote :3333
```

## Phase 1: Set Up Breakpoints

### 1.1 Exception Handler Breakpoints

These catch any faults that might be occurring silently:

```gdb
break HardFault
break MemoryManagement
break BusFault
break UsageFault
break DefaultHandler
```

### 1.2 Context Switch Breakpoints

```gdb
# PendSV naked wrapper (exception entry point)
break PendSV

# Rust handler that decides which thread to switch to
break pendsv_swap_sp
```

### 1.3 Find User Entry Points

```gdb
# List symbols to find user app entry points
info functions _start
info functions main
info functions initiator
info functions handler

# Example output might show:
#   0x00020001  initiator::_start
#   0x00040001  handler::_start

# Set breakpoints on user entry
break *0x00020001
break *0x00040001
```

## Phase 2: Initial Run to Context Switch

```gdb
# Continue to first PendSV (context switch)
continue

# When PendSV hits, examine entry state
info registers

# Check special registers
p/x $control
p/x $psp
p/x $msp
p/x $lr
p/x $xpsr

# Check IPSR (exception number in low 9 bits of xPSR)
# IPSR = 14 means we're in PendSV
p ($xpsr & 0x1FF)
```

## Phase 3: Trace Through pendsv_swap_sp

```gdb
# Continue to pendsv_swap_sp
continue

# Step through the function
next
next
# ... until we reach the return

# Check what frame pointer is being returned
finish
p/x $r0

# This r0 value is the KernelExceptionFrame pointer
# Examine it (11 words for user_space build):
# r4, r5, r6, r7, r8, r9, r10, r11, psp, control, return_address
x/11xw $r0
```

**Expected KernelExceptionFrame layout:**
```
Offset  Field           Expected for User Thread
------  -----           ------------------------
+0x00   r4              0x00000000 (zeroed for new thread)
+0x04   r5              0x00000000
+0x08   r6              0x00000000
+0x0C   r7              0x00000000
+0x10   r8              0x00000000
+0x14   r9              0x00000000
+0x18   r10             0x00000000
+0x1C   r11             0x00000000
+0x20   psp             0x2000xxxx (user stack pointer)
+0x24   control         0x00000003 (nPRIV=1, SPSEL=1)
+0x28   return_address  0xFFFFFFFD (EXC_RETURN)
```

## Phase 4: Examine User Exception Frame

The hardware unstacks the ExceptionFrame from PSP on exception return.

```gdb
# Get the PSP value from KernelExceptionFrame
set $user_psp = *(unsigned int*)($r0 + 0x20)
p/x $user_psp

# Examine the hardware exception frame at PSP
# Layout: r0, r1, r2, r3, r12, lr, pc, xpsr
x/8xw $user_psp
```

**Expected ExceptionFrame layout:**
```
Offset  Field   Expected for User Thread
------  -----   ------------------------
+0x00   r0      Arg 0 to user main
+0x04   r1      Arg 1 to user main
+0x08   r2      Arg 2 to user main
+0x0C   r3      Arg 3 to user main
+0x10   r12     0x00000000
+0x14   lr      Return address (or 0)
+0x18   pc      User entry point (0x00020001)
+0x1C   xpsr    0x01000000 (Thumb bit set)
```

**Critical check:** The PC value must be the user entry point, and xPSR must have bit 24 (Thumb) set.

## Phase 5: Trace Exception Return

```gdb
# Disassemble PendSV to find the exception return instruction
disassemble PendSV

# Look for something like:
#   pop {r4-r11}
#   pop {r0-r1}       ; psp, control
#   msr psp, r0
#   msr control, r1
#   pop {pc}          ; loads EXC_RETURN, triggers exception return

# Set breakpoint just before 'pop {pc}' or 'bx lr'
# (get the address from disassemble output)
break *0x<address_before_exception_return>

continue

# Now we're about to do exception return
# Check all the registers one more time
info registers
p/x $psp
p/x $control
p/x $lr

# Examine what's on the stack (what pop {pc} will load)
x/1xw $sp

# Single step through exception return
stepi

# WHERE ARE WE NOW?
info registers
p/x $pc
p/x $control
p/x $xpsr
```

## Phase 6: Check for Immediate Fault

If we hit a fault immediately after exception return:

```gdb
# If we're in a fault handler, check fault status registers

# CFSR (Configurable Fault Status Register)
x/1xw 0xE000ED28
# Bits [7:0]   = MMFSR (MemManage)
# Bits [15:8]  = BFSR (BusFault)
# Bits [31:16] = UFSR (UsageFault)

# HFSR (HardFault Status Register)
x/1xw 0xE000ED2C

# MMFAR (MemManage Fault Address)
x/1xw 0xE000ED34

# BFAR (BusFault Address)
x/1xw 0xE000ED38

# Decode CFSR:
set $cfsr = *(unsigned int*)0xE000ED28
printf "MMFSR=0x%02x BFSR=0x%02x UFSR=0x%04x\n", $cfsr & 0xFF, ($cfsr >> 8) & 0xFF, ($cfsr >> 16) & 0xFFFF
```

**UFSR bit meanings (UsageFault):**
| Bit | Name | Meaning |
|-----|------|---------|
| 0 | UNDEFINSTR | Undefined instruction |
| 1 | INVSTATE | Invalid state (e.g., Thumb bit not set) |
| 2 | INVPC | Invalid PC load |
| 3 | NOCP | No coprocessor |
| 8 | UNALIGNED | Unaligned access |
| 9 | DIVBYZERO | Divide by zero |

## Phase 7: Check MPU Configuration

```gdb
# MPU registers base: 0xE000ED90
# TYPE, CTRL, RNR, RBAR, RASR

# MPU_TYPE - check if MPU exists
x/1xw 0xE000ED90

# MPU_CTRL
x/1xw 0xE000ED94
# Bit 0: ENABLE
# Bit 1: HFNMIENA (MPU enabled during HardFault/NMI)
# Bit 2: PRIVDEFENA (privileged default memory map)

# Dump all 8 MPU regions
set $i = 0
while $i < 8
  set *(unsigned int*)0xE000ED98 = $i
  set $rbar = *(unsigned int*)0xE000ED9C
  set $rasr = *(unsigned int*)0xE000EDA0
  printf "Region %d: RBAR=0x%08x RASR=0x%08x", $i, $rbar, $rasr
  if $rasr & 1
    printf " [ENABLED]"
  end
  printf "\n"
  set $i = $i + 1
end
```

**Check that user code region:**
1. Is enabled (RASR bit 0 = 1)
2. Has XN=0 (execute allowed, RASR bit 28 = 0)
3. Has correct AP bits for unprivileged access (RASR bits 26:24)
4. Covers the user code address range

## Phase 8: Compare with ARMv8-M

Run the same debugging on the working MPS2-AN505 target:

```bash
# Terminal 1
bazel build //pw_kernel/target/mps2_an505/ipc/user:ipc --config=k_qemu_mps2_an505

qemu-system-arm \
    -cpu cortex-m33 \
    -machine mps2-an505 \
    -nographic \
    -semihosting-config enable=on,target=native \
    -kernel bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf \
    -S -gdb tcp::3333

# Terminal 2
gdb-multiarch bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf
(gdb) target remote :3333
```

Compare these values at the same breakpoints:
- KernelExceptionFrame contents
- User ExceptionFrame contents
- Register values before/after exception return
- Where PC ends up after exception return

## Automated Debug Script

Save as `debug_usermode.gdb`:

```gdb
# ARMv7-M User Mode Debug Script
target remote :3333

# Helper to dump CPU state
define dump_cpu
  printf "=== CPU State ===\n"
  printf "PC=0x%08x  LR=0x%08x  SP=0x%08x\n", $pc, $lr, $sp
  printf "PSP=0x%08x MSP=0x%08x CONTROL=0x%08x\n", $psp, $msp, $control
  printf "xPSR=0x%08x (IPSR=%d)\n", $xpsr, $xpsr & 0x1FF
end

# Helper to dump fault status
define dump_faults
  set $cfsr = *(unsigned int*)0xE000ED28
  set $hfsr = *(unsigned int*)0xE000ED2C
  printf "=== Fault Status ===\n"
  printf "CFSR=0x%08x HFSR=0x%08x\n", $cfsr, $hfsr
  printf "MMFSR=0x%02x BFSR=0x%02x UFSR=0x%04x\n", $cfsr & 0xFF, ($cfsr >> 8) & 0xFF, ($cfsr >> 16) & 0xFFFF
  if $cfsr & 0x80
    printf "MMFAR=0x%08x\n", *(unsigned int*)0xE000ED34
  end
  if $cfsr & 0x8000
    printf "BFAR=0x%08x\n", *(unsigned int*)0xE000ED38
  end
end

# Helper to dump MPU
define dump_mpu
  printf "=== MPU Configuration ===\n"
  printf "MPU_TYPE=0x%08x MPU_CTRL=0x%08x\n", *(unsigned int*)0xE000ED90, *(unsigned int*)0xE000ED94
  set $i = 0
  while $i < 8
    set *(unsigned int*)0xE000ED98 = $i
    set $rbar = *(unsigned int*)0xE000ED9C
    set $rasr = *(unsigned int*)0xE000EDA0
    if $rasr & 1
      printf "Region %d: RBAR=0x%08x RASR=0x%08x [ENABLED]\n", $i, $rbar, $rasr
    end
    set $i = $i + 1
  end
end

# Break on faults
break HardFault
commands
  silent
  printf "\n!!! HardFault !!!\n"
  dump_cpu
  dump_faults
end

break MemoryManagement
commands
  silent
  printf "\n!!! MemoryManagement !!!\n"
  dump_cpu
  dump_faults
end

break UsageFault
commands
  silent
  printf "\n!!! UsageFault !!!\n"
  dump_cpu
  dump_faults
end

# Break on PendSV
break PendSV
commands
  silent
  printf "\n=== PendSV Entry ===\n"
  dump_cpu
  continue
end

# Instructions
printf "\n"
printf "=== ARMv7-M User Mode Debugger ===\n"
printf "Commands:\n"
printf "  dump_cpu    - Show CPU registers\n"
printf "  dump_faults - Show fault status registers\n"
printf "  dump_mpu    - Show MPU configuration\n"
printf "\n"
printf "Breakpoints set on: HardFault, MemoryManagement, UsageFault, PendSV\n"
printf "\n"
printf "Type 'continue' to start execution\n"
printf "\n"
```

The script above is available at [pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb](../pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb).

**Run with:**
```bash
gdb-multiarch -x pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
```

## Expected Findings

Based on the symptoms, we expect to find one of:

| Finding | Likely Cause | Next Step |
|---------|--------------|-----------|
| PC at wrong address after exception return | Exception frame PC field corrupted or wrong | Check frame initialization in `initialize_user_frame` |
| UsageFault with INVSTATE | Thumb bit not set in xPSR | Check `RetPsrVal` initialization |
| MemManage fault | MPU not allowing user code execution | Check MPU region for user flash |
| Stuck in PendSV | Exception return not happening | Check assembly restore sequence |
| PC in user code but no output | Syscall/semihosting broken for user mode | Check syscall path |

## Quick Reference: Key Addresses

| Register/Address | Description |
|------------------|-------------|
| `0xE000ED04` | ICSR (Interrupt Control State) |
| `0xE000ED28` | CFSR (Configurable Fault Status) |
| `0xE000ED2C` | HFSR (HardFault Status) |
| `0xE000ED34` | MMFAR (MemManage Fault Address) |
| `0xE000ED38` | BFAR (BusFault Address) |
| `0xE000ED90` | MPU_TYPE |
| `0xE000ED94` | MPU_CTRL |
| `0xE000ED98` | MPU_RNR (Region Number) |
| `0xE000ED9C` | MPU_RBAR (Region Base Address) |
| `0xE000EDA0` | MPU_RASR (Region Attribute and Size) |

## Quick Reference: EXC_RETURN Values

| Value | Mode | Stack | Security |
|-------|------|-------|----------|
| `0xFFFFFFF1` | Handler | MSP | - |
| `0xFFFFFFF9` | Thread | MSP | - |
| `0xFFFFFFFD` | Thread | PSP | - |
| `0xFFFFFFE1` | Handler | MSP | Non-secure |
| `0xFFFFFFE9` | Thread | MSP | Non-secure |
| `0xFFFFFFED` | Thread | PSP | Non-secure |

For ARMv7-M user mode, we expect `0xFFFFFFFD` (Thread mode, PSP).
