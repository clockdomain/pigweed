# ARMv7-M GDB Debugging Guide for Stack Corruption Investigation

## Target: AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7 MPU)

This guide provides comprehensive GDB commands for debugging stack corruption during syscall/exception handling on ARMv7-M.

---

## 1. Core Register Inspection

### 1.1 Stack Pointers (MSP and PSP)

```gdb
# Read Main Stack Pointer (kernel stack)
p/x $msp

# Read Process Stack Pointer (user stack)
p/x $psp

# Read both with context
printf "MSP (kernel): 0x%08x\nPSP (user):   0x%08x\n", $msp, $psp

# Read the stack pointer currently in use
p/x $sp
```

### 1.2 CONTROL Register

```gdb
# Read CONTROL register directly
p/x $control

# Decode CONTROL register bits:
#   Bit 0 (nPRIV): 0=Privileged, 1=Unprivileged
#   Bit 1 (SPSEL): 0=MSP, 1=PSP
#   Bit 2 (FPCA):  0=No FP context, 1=FP context active
printf "CONTROL: 0x%08x\n  nPRIV=%d (0=kernel,1=user)\n  SPSEL=%d (0=MSP,1=PSP)\n  FPCA=%d\n", \
    $control, $control & 1, ($control >> 1) & 1, ($control >> 2) & 1
```

### 1.3 Program Status Register (xPSR)

```gdb
# Read xPSR
p/x $xpsr

# Decode xPSR:
#   Bits 0-8: Exception number (0=Thread mode, >0=Handler mode)
#   Bit 24 (T): Thumb state (must be 1 for Cortex-M)
printf "xPSR: 0x%08x\n  Exception#=%d (0=Thread mode)\n  T=%d (Thumb state)\n", \
    $xpsr, $xpsr & 0x1ff, ($xpsr >> 24) & 1
```

### 1.4 Link Register and EXC_RETURN

```gdb
# Read LR - critical during exception handling
p/x $lr

# Decode EXC_RETURN values:
define decode_exc_return
    set $exc = $arg0
    printf "EXC_RETURN: 0x%08x\n", $exc
    if ($exc & 0xFFFFFFF0) == 0xFFFFFFF0
        printf "  Valid EXC_RETURN detected\n"
        printf "  SPSEL=%d (0=MSP, 1=PSP)\n", ($exc >> 2) & 1
        printf "  MODE=%d (0=Handler, 1=Thread)\n", ($exc >> 3) & 1
        printf "  FType=%d (0=Extended, 1=Standard frame)\n", ($exc >> 4) & 1
    else
        printf "  WARNING: Not a valid EXC_RETURN value!\n"
    end
end

# Usage:
decode_exc_return $lr
```

**Common EXC_RETURN values:**

| Value | Meaning |
|-------|---------|
| `0xFFFFFFF1` | Return to Handler mode, MSP |
| `0xFFFFFFF9` | Return to Thread mode, MSP (kernel thread) |
| `0xFFFFFFFD` | Return to Thread mode, PSP (user thread) |

---

## 2. Exception Frame Inspection

### 2.1 Hardware-Pushed Exception Frame (on PSP for user threads)

The hardware automatically pushes this frame on exception entry:

```
+--------+--------+
| Offset | Field  |
+--------+--------+
|  0x00  | r0     |
|  0x04  | r1     |
|  0x08  | r2     |
|  0x0C  | r3     |
|  0x10  | r12    |
|  0x14  | lr     |
|  0x18  | pc     |
|  0x1C  | xPSR   |
+--------+--------+
```

```gdb
# Dump hardware exception frame at PSP
define dump_exception_frame
    set $frame = $arg0
    printf "Exception frame at 0x%08x:\n", $frame
    printf "  r0  = 0x%08x  r1  = 0x%08x\n", *(uint32_t*)($frame+0x00), *(uint32_t*)($frame+0x04)
    printf "  r2  = 0x%08x  r3  = 0x%08x\n", *(uint32_t*)($frame+0x08), *(uint32_t*)($frame+0x0C)
    printf "  r12 = 0x%08x  lr  = 0x%08x\n", *(uint32_t*)($frame+0x10), *(uint32_t*)($frame+0x14)
    printf "  pc  = 0x%08x  psr = 0x%08x\n", *(uint32_t*)($frame+0x18), *(uint32_t*)($frame+0x1C)
end

# Dump user exception frame
dump_exception_frame $psp
```

### 2.2 KernelExceptionFrame (Software-Pushed on MSP)

Your kernel pushes additional context:

```
+--------+--------------+
| Offset | Field        |
+--------+--------------+
|  0x00  | r4           |
|  0x04  | r5           |
|  0x08  | r6           |
|  0x0C  | r7           |
|  0x10  | r8           |
|  0x14  | r9           |
|  0x18  | r10          |
|  0x1C  | r11          |
|  0x20  | psp          |
|  0x24  | control      |
|  0x28  | return_addr  |
+--------+--------------+
```

```gdb
# Dump kernel exception frame
define dump_kernel_frame
    set $kf = $arg0
    printf "KernelExceptionFrame at 0x%08x:\n", $kf
    printf "  r4  = 0x%08x  r5  = 0x%08x  r6  = 0x%08x  r7  = 0x%08x\n", \
        *(uint32_t*)($kf+0x00), *(uint32_t*)($kf+0x04), \
        *(uint32_t*)($kf+0x08), *(uint32_t*)($kf+0x0C)
    printf "  r8  = 0x%08x  r9  = 0x%08x  r10 = 0x%08x  r11 = 0x%08x\n", \
        *(uint32_t*)($kf+0x10), *(uint32_t*)($kf+0x14), \
        *(uint32_t*)($kf+0x18), *(uint32_t*)($kf+0x1C)
    printf "  psp = 0x%08x  control = 0x%08x  return_addr = 0x%08x\n", \
        *(uint32_t*)($kf+0x20), *(uint32_t*)($kf+0x24), *(uint32_t*)($kf+0x28)
end

# Dump from current MSP
dump_kernel_frame $msp
```

### 2.3 Combined Frame Dump

```gdb
define dump_all_frames
    printf "=== STACK STATE ===\n"
    printf "MSP: 0x%08x  PSP: 0x%08x  CONTROL: 0x%08x\n", $msp, $psp, $control
    printf "\n=== KERNEL FRAME (MSP) ===\n"
    dump_kernel_frame $msp
    printf "\n=== USER FRAME (PSP) ===\n"
    dump_exception_frame $psp
end
```

---

## 3. Critical Breakpoints

### 3.1 SVCall Entry (Syscall Start)

```gdb
# Break at SVCall handler entry
break SVCall
commands
    silent
    printf "\n>>> SVCall ENTRY <<<\n"
    printf "LR (EXC_RETURN): 0x%08x\n", $lr
    printf "PSP: 0x%08x  MSP: 0x%08x\n", $psp, $msp
    printf "CONTROL: 0x%08x\n", $control
    dump_exception_frame $psp
    continue
end

# Or set specific address if symbol not found
# break *0x<address_of_SVCall>
```

### 3.2 svc_return (Syscall Return to User)

```gdb
# Break at syscall return trampoline
break svc_return
commands
    silent
    printf "\n>>> svc_return ENTRY <<<\n"
    printf "r0 (frame ptr): 0x%08x\n", $r0
    dump_kernel_frame $r0
    continue
end

# Critical: break AFTER psp/control restoration
break *svc_return+0x14
commands
    silent
    printf "\n>>> svc_return: AFTER control write <<<\n"
    printf "PSP: 0x%08x  CONTROL: 0x%08x\n", $psp, $control
    dump_exception_frame $psp
    continue
end
```

### 3.3 PendSV Handler (Context Switch)

```gdb
# Break at PendSV entry
break pendsv_swap_sp
commands
    silent
    printf "\n>>> PendSV ENTRY <<<\n"
    printf "Frame arg (r0): 0x%08x\n", $r0
    printf "ACTIVE_THREAD ptr: 0x%08x\n", *(uint32_t*)&ACTIVE_THREAD
    dump_kernel_frame $r0
    continue
end

# Break at PendSV exit (returning new frame)
# Find the 'bx lr' instruction at end of pendsv
tbreak pendsv_swap_sp
commands
    silent
    finish
    printf "\n>>> PendSV RETURNING <<<\n"
    printf "Returned frame: 0x%08x\n", $r0
    dump_kernel_frame $r0
end
```

### 3.4 handle_svc (Syscall Processing)

```gdb
# Break at syscall dispatch
break handle_svc
commands
    silent
    printf "\n>>> handle_svc <<<\n"
    printf "KernelFrame: 0x%08x\n", $r0
    # r4 contains syscall number (from frame)
    set $kf = (uint32_t*)$r0
    printf "Syscall number (r4): 0x%08x\n", $kf[0]
    continue
end
```

---

## 4. Detecting Stack Corruption

### 4.1 Validate Exception Frame Integrity

```gdb
define validate_user_frame
    set $f = $arg0
    set $pc = *(uint32_t*)($f + 0x18)
    set $lr = *(uint32_t*)($f + 0x14)
    set $psr = *(uint32_t*)($f + 0x1C)
    
    printf "Validating frame at 0x%08x:\n", $f
    
    # Check PC is in valid user flash range (0x40000-0x60000 for handler process)
    if $pc < 0x40000 || $pc >= 0x80000
        printf "  ERROR: PC 0x%08x outside user range!\n", $pc
    else
        printf "  OK: PC 0x%08x in valid range\n", $pc
    end
    
    # Check LR looks valid (should be user code address or valid return)
    if ($lr & 0xFFFF0000) == 0x00000000 && $lr < 0x1000
        printf "  WARNING: LR 0x%08x looks suspicious (small value)\n", $lr
    end
    
    # Check Thumb bit in PSR
    if (($psr >> 24) & 1) == 0
        printf "  ERROR: Thumb bit not set in PSR!\n"
    else
        printf "  OK: Thumb bit set\n"
    end
    
    # Check for kernel addresses in user registers
    set $r0 = *(uint32_t*)($f + 0x00)
    set $r1 = *(uint32_t*)($f + 0x04)
    set $r2 = *(uint32_t*)($f + 0x08)
    set $r3 = *(uint32_t*)($f + 0x0C)
    
    if $r0 >= 0x60000 && $r0 < 0x80000
        printf "  LEAK: r0 = 0x%08x (kernel RAM)\n", $r0
    end
    if $r1 >= 0x60000 && $r1 < 0x80000
        printf "  LEAK: r1 = 0x%08x (kernel RAM)\n", $r1
    end
    if $r2 >= 0x60000 && $r2 < 0x80000
        printf "  LEAK: r2 = 0x%08x (kernel RAM)\n", $r2
    end
    if $r3 >= 0x60000 && $r3 < 0x80000
        printf "  LEAK: r3 = 0x%08x (kernel RAM)\n", $r3
    end
end

# Usage:
validate_user_frame $psp
```

### 4.2 Monitor for Kernel Address Leaks

```gdb
define check_kernel_leak_in_regs
    printf "Checking user registers for kernel addresses:\n"
    set $kernel_ram_start = 0x60000
    set $kernel_ram_end = 0x80000
    
    if $r0 >= $kernel_ram_start && $r0 < $kernel_ram_end
        printf "  LEAK: r0 = 0x%08x\n", $r0
    end
    if $r1 >= $kernel_ram_start && $r1 < $kernel_ram_end
        printf "  LEAK: r1 = 0x%08x\n", $r1
    end
    if $r2 >= $kernel_ram_start && $r2 < $kernel_ram_end
        printf "  LEAK: r2 = 0x%08x\n", $r2
    end
    if $r3 >= $kernel_ram_start && $r3 < $kernel_ram_end
        printf "  LEAK: r3 = 0x%08x\n", $r3
    end
    if $r4 >= $kernel_ram_start && $r4 < $kernel_ram_end
        printf "  LEAK: r4 = 0x%08x\n", $r4
    end
    if $r5 >= $kernel_ram_start && $r5 < $kernel_ram_end
        printf "  LEAK: r5 = 0x%08x\n", $r5
    end
    if $r6 >= $kernel_ram_start && $r6 < $kernel_ram_end
        printf "  LEAK: r6 = 0x%08x\n", $r6
    end
    if $r7 >= $kernel_ram_start && $r7 < $kernel_ram_end
        printf "  LEAK: r7 = 0x%08x\n", $r7
    end
    if $r8 >= $kernel_ram_start && $r8 < $kernel_ram_end
        printf "  LEAK: r8 = 0x%08x\n", $r8
    end
    if $r9 >= $kernel_ram_start && $r9 < $kernel_ram_end
        printf "  LEAK: r9 = 0x%08x\n", $r9
    end
    if $r10 >= $kernel_ram_start && $r10 < $kernel_ram_end
        printf "  LEAK: r10 = 0x%08x\n", $r10
    end
    if $r11 >= $kernel_ram_start && $r11 < $kernel_ram_end
        printf "  LEAK: r11 = 0x%08x\n", $r11
    end
end
```

---

## 5. Memory Watchpoints

### 5.1 Watch User Frame for Corruption

```gdb
# Get user PSP and set watchpoints on critical fields

# Watch LR in exception frame (offset 0x14)
set $user_lr_addr = $psp + 0x14
watch *(uint32_t*)$user_lr_addr
commands
    printf "User LR modified! New value: 0x%08x\n", *(uint32_t*)$user_lr_addr
    bt
end

# Watch PC in exception frame (offset 0x18)
set $user_pc_addr = $psp + 0x18
watch *(uint32_t*)$user_pc_addr
commands
    printf "User PC modified! New value: 0x%08x\n", *(uint32_t*)$user_pc_addr
    bt
end
```

### 5.2 Watch Kernel Frame PSP/Control

```gdb
# Assuming kernel frame is at known address (get from breakpoint)
# Watch the control field for unexpected modification
set $kframe_control_addr = 0x000616cc + 0x24
watch *(uint32_t*)$kframe_control_addr
commands
    printf "KernelFrame CONTROL modified! New: 0x%08x\n", *(uint32_t*)$kframe_control_addr
    bt
end
```

### 5.3 Catch Writes to Specific Register Slots

```gdb
# Watch r8 in kernel frame (where you see 0x00061778 leak)
# KernelFrame.r8 is at offset 0x10
define watch_kernel_frame_r8
    set $addr = $arg0 + 0x10
    watch *(uint32_t*)$addr
    commands
        printf "KernelFrame.r8 written! Value: 0x%08x\n", *(uint32_t*)$addr
        bt
        continue
    end
end

# Usage: watch_kernel_frame_r8 0x000616cc
```

---

## 6. Context Switch Tracing

### 6.1 Complete Context Switch Trace

```gdb
# Trace entire context switch sequence
define trace_context_switch
    printf "=== Starting Context Switch Trace ===\n"
    
    # Before SVCall
    break SVCall
    commands
        silent
        printf "[SVCall Entry] PSP=0x%08x MSP=0x%08x LR=0x%08x\n", $psp, $msp, $lr
        continue
    end
    
    # Before PendSV  
    break pendsv_swap_sp
    commands
        silent
        printf "[PendSV Entry] frame=0x%08x\n", $r0
        dump_kernel_frame $r0
        continue
    end
    
    # After context switch
    tbreak pendsv_swap_sp
    commands
        finish
        printf "[PendSV Exit] new_frame=0x%08x\n", $r0
        dump_kernel_frame $r0
    end
    
    # Return to user
    break svc_return
    commands
        silent
        printf "[svc_return] frame=0x%08x\n", $r0
        continue
    end
end
```

### 6.2 Track Thread Transitions

```gdb
# Track which thread is active
define track_active_thread
    # Need address of ACTIVE_THREAD static
    set $active_thread_addr = &ACTIVE_THREAD
    printf "ACTIVE_THREAD address: 0x%08x\n", $active_thread_addr
    printf "Current value: 0x%08x\n", *(uint32_t*)$active_thread_addr
end
```

### 6.3 Dump Thread State

```gdb
# Dump ArchThreadState structure
define dump_thread_state
    set $ts = $arg0
    printf "ArchThreadState at 0x%08x:\n", $ts
    printf "  frame ptr:            0x%08x\n", *(uint32_t*)($ts + 0)
    printf "  memory_config ptr:    0x%08x\n", *(uint32_t*)($ts + 4)
    # ThreadLocalState at offset 8
    printf "  canonical_control:    0x%08x\n", *(uint32_t*)($ts + 16)  # Adjust offset
    printf "  canonical_return:     0x%08x\n", *(uint32_t*)($ts + 20)  # Adjust offset
end
```

---

## 7. ARMv7-M Specific System Registers

### 7.1 Fault Status Registers

```gdb
# Configurable Fault Status Register (CFSR) - 0xE000ED28
define dump_cfsr
    set $cfsr = *(uint32_t*)0xE000ED28
    printf "CFSR: 0x%08x\n", $cfsr
    
    # MemManage Fault Status (bits 0-7)
    set $mmfsr = $cfsr & 0xFF
    printf "  MMFSR: 0x%02x\n", $mmfsr
    if $mmfsr & (1 << 0)
        printf "    IACCVIOL: Instruction access violation\n"
    end
    if $mmfsr & (1 << 1)
        printf "    DACCVIOL: Data access violation\n"
    end
    if $mmfsr & (1 << 3)
        printf "    MUNSTKERR: MemManage fault on unstacking\n"
    end
    if $mmfsr & (1 << 4)
        printf "    MSTKERR: MemManage fault on stacking\n"
    end
    if $mmfsr & (1 << 5)
        printf "    MLSPERR: MemManage fault during FP lazy stacking\n"
    end
    if $mmfsr & (1 << 7)
        printf "    MMARVALID: MMFAR contains valid address\n"
    end
    
    # BusFault Status (bits 8-15)
    set $bfsr = ($cfsr >> 8) & 0xFF
    printf "  BFSR: 0x%02x\n", $bfsr
    
    # UsageFault Status (bits 16-31)
    set $ufsr = ($cfsr >> 16) & 0xFFFF
    printf "  UFSR: 0x%04x\n", $ufsr
end

# MemManage Fault Address Register - 0xE000ED34
define dump_mmfar
    printf "MMFAR: 0x%08x\n", *(uint32_t*)0xE000ED34
end

# BusFault Address Register - 0xE000ED38
define dump_bfar
    printf "BFAR: 0x%08x\n", *(uint32_t*)0xE000ED38
end

# Combined fault dump
define dump_fault_info
    dump_cfsr
    dump_mmfar
    dump_bfar
end
```

### 7.2 System Handler Control and State (SHCSR)

```gdb
# SHCSR - 0xE000ED24
define dump_shcsr
    set $shcsr = *(uint32_t*)0xE000ED24
    printf "SHCSR: 0x%08x\n", $shcsr
    printf "  MEMFAULTACT:   %d\n", ($shcsr >> 0) & 1
    printf "  BUSFAULTACT:   %d\n", ($shcsr >> 1) & 1
    printf "  USGFAULTACT:   %d\n", ($shcsr >> 3) & 1
    printf "  SVCALLACT:     %d\n", ($shcsr >> 7) & 1
    printf "  MONITORACT:    %d\n", ($shcsr >> 8) & 1
    printf "  PENDSVACT:     %d\n", ($shcsr >> 10) & 1
    printf "  SYSTICKACT:    %d\n", ($shcsr >> 11) & 1
    printf "  USGFAULTPENDED:%d\n", ($shcsr >> 12) & 1
    printf "  MEMFAULTPENDED:%d\n", ($shcsr >> 13) & 1
    printf "  BUSFAULTPENDED:%d\n", ($shcsr >> 14) & 1
    printf "  SVCALLPENDED:  %d\n", ($shcsr >> 15) & 1
    printf "  MEMFAULTENA:   %d\n", ($shcsr >> 16) & 1
    printf "  BUSFAULTENA:   %d\n", ($shcsr >> 17) & 1
    printf "  USGFAULTENA:   %d\n", ($shcsr >> 18) & 1
end
```

### 7.3 Interrupt Control State Register (ICSR)

```gdb
# ICSR - 0xE000ED04
define dump_icsr
    set $icsr = *(uint32_t*)0xE000ED04
    printf "ICSR: 0x%08x\n", $icsr
    printf "  VECTACTIVE:  %d (0=Thread, 11=SVCall, 14=PendSV, 15=SysTick)\n", $icsr & 0x1FF
    printf "  VECTPENDING: %d\n", ($icsr >> 12) & 0x1FF
    printf "  ISRPENDING:  %d\n", ($icsr >> 22) & 1
    printf "  PENDSTCLR:   %d\n", ($icsr >> 25) & 1
    printf "  PENDSTSET:   %d\n", ($icsr >> 26) & 1
    printf "  PENDSVCLR:   %d\n", ($icsr >> 27) & 1
    printf "  PENDSVSET:   %d\n", ($icsr >> 28) & 1
end
```

---

## 8. QEMU-Specific Debugging

### 8.1 QEMU Monitor Commands

```gdb
# In QEMU, use monitor commands via GDB
monitor info registers
monitor info cpus
monitor info mtree  # Memory map
```

### 8.2 Break on MemoryManagement Fault

```gdb
# Set breakpoint on your MemoryManagement handler
break MemoryManagement
commands
    printf "=== MEMORY MANAGEMENT FAULT ===\n"
    dump_fault_info
    dump_all_frames
    check_kernel_leak_in_regs
end
```

### 8.3 QEMU Tracing

```bash
# Run QEMU with tracing (outside GDB)
qemu-system-arm -d exec,int,cpu -D qemu_trace.log ...

# Or for specific exception tracing:
qemu-system-arm -d int -D qemu_int.log ...
```

---

## 9. Complete Debugging Session Example

### 9.1 Initial Setup Script

Save as `debug_setup.gdb`:

```gdb
# Connect to QEMU
target remote :1234

# Load symbols
# symbol-file your_kernel.elf

# Memory ranges for validation
set $kernel_ram_start = 0x60000
set $kernel_ram_end   = 0x80000
set $user_flash_start = 0x40000
set $user_flash_end   = 0x60000
set $user_ram_start   = 0x88000
set $user_ram_end     = 0x90000

# Load helper functions
source gdb_helpers.gdb

# Set critical breakpoints
break SVCall
break svc_return
break pendsv_swap_sp
break MemoryManagement

printf "Debug setup complete. Use 'continue' to start.\n"
```

### 9.2 Investigating Your Specific Bug

Based on your observations (r1=0x00061778, r8=0x00061778, lr=0x00000003):

```gdb
# 1. First, find where 0x00061778 comes from
break *svc_return
commands
    set $kf = $r0
    printf "Checking kernel frame before restore...\n"
    
    # Check r8 in saved kernel frame (offset 0x10)
    set $saved_r8 = *(uint32_t*)($kf + 0x10)
    if $saved_r8 >= 0x60000 && $saved_r8 < 0x80000
        printf "CORRUPTION: r8 = 0x%08x (kernel addr in saved frame!)\n", $saved_r8
        bt
    end
    continue
end

# 2. Watch for when the corruption happens
# Set watchpoint on a known handler thread's kernel frame r8 slot
# First get the frame address from PendSV or SVCall
watch *(uint32_t*)(KNOWN_FRAME_ADDR + 0x10)
commands
    printf "r8 slot written: 0x%08x\n", *(uint32_t*)(KNOWN_FRAME_ADDR + 0x10)
    bt
end

# 3. Check if wrong thread's frame is being restored
break pendsv_swap_sp
commands
    # Verify the frame being returned matches the intended thread
    printf "Returning frame: 0x%08x\n", $r0
    # Compare with expected handler thread frame address
end
```

### 9.3 Tracing the Corruption Point

```gdb
# The corruption likely happens when:
# 1. Handler is in syscall (object_wait)
# 2. Context switch to Initiator
# 3. Initiator runs, does something that corrupts Handler's saved state
# 4. Switch back to Handler restores corrupted state

# Set conditional breakpoint to catch the moment of corruption
break pendsv_swap_sp
commands
    # Print both old and new thread info
    set $old = *(uint32_t*)&ACTIVE_THREAD
    printf "Switching FROM thread at: 0x%08x\n", $old
    if $old != 0
        set $old_frame = *(uint32_t*)$old
        printf "  Old frame: 0x%08x\n", $old_frame
        printf "  Old frame r8: 0x%08x\n", *(uint32_t*)($old_frame + 0x10)
    end
    continue
end
```

---

## 10. Quick Reference Card

| Register | GDB Access | Address |
|----------|-----------|---------|
| MSP | `$msp` | N/A |
| PSP | `$psp` | N/A |
| CONTROL | `$control` | N/A |
| PRIMASK | `$primask` | N/A |
| BASEPRI | `$basepri` | N/A |
| xPSR | `$xpsr` | N/A |
| CFSR | `*(uint32_t*)0xE000ED28` | 0xE000ED28 |
| MMFAR | `*(uint32_t*)0xE000ED34` | 0xE000ED34 |
| BFAR | `*(uint32_t*)0xE000ED38` | 0xE000ED38 |
| SHCSR | `*(uint32_t*)0xE000ED24` | 0xE000ED24 |
| ICSR | `*(uint32_t*)0xE000ED04` | 0xE000ED04 |

| EXC_RETURN | Meaning |
|------------|---------|
| 0xFFFFFFF1 | Handler mode, MSP |
| 0xFFFFFFF9 | Thread mode, MSP (kernel) |
| 0xFFFFFFFD | Thread mode, PSP (user) |

| Exception # | Name |
|-------------|------|
| 0 | Thread mode |
| 11 | SVCall |
| 14 | PendSV |
| 15 | SysTick |
| ≥16 | External IRQ |

---

## 11. Object_wait Hang Investigation Commands

The `object_wait` syscall hangs after ~39/40 calls with PRIMASK=1 (interrupts disabled).
Key insight: Handle 0 doesn't exist, so syscall should return `Error::OutOfRange` immediately.

### 11.1 Track Syscall Flow

```gdb
# Count syscalls and watch for the hang
set $syscall_count = 0

break SVCall
commands
    silent
    set $syscall_count = $syscall_count + 1
    printf "SVCall #%d: LR=%08x CONTROL=%08x PRIMASK=%08x\n", $syscall_count, $lr, $control, $primask
    continue
end

break svc_return
commands
    silent
    printf "svc_return #%d: returning to user\n", $syscall_count
    continue
end
```

### 11.2 Check PRIMASK State

```gdb
# Monitor when PRIMASK gets stuck at 1
define check_primask
    if $primask == 1
        printf "WARNING: Interrupts DISABLED (PRIMASK=1)\n"
        printf "  Location: "
        where 1
    end
end

# Add to all breakpoints
break handle_svc
commands
    silent
    check_primask
    continue
end
```

### 11.3 Catch the Hang

```gdb
# When system hangs, Ctrl+C and run:
define diagnose_hang
    printf "=== HANG DIAGNOSTICS ===\n"
    printf "PC: 0x%08x  LR: 0x%08x\n", $pc, $lr
    printf "MSP: 0x%08x  PSP: 0x%08x\n", $msp, $psp
    printf "CONTROL: 0x%08x  PRIMASK: 0x%08x\n", $control, $primask
    printf "xPSR: 0x%08x (exception#=%d)\n", $xpsr, $xpsr & 0x1ff
    
    # Check if in handler mode
    if ($xpsr & 0x1ff) != 0
        printf "STATUS: In Handler Mode (exception %d)\n", $xpsr & 0x1ff
    else
        printf "STATUS: In Thread Mode\n"
    end
    
    # Check interrupt state
    if $primask == 1
        printf "STATUS: Interrupts DISABLED\n"
    else
        printf "STATUS: Interrupts enabled\n"
    end
    
    # Show backtrace
    printf "\n=== BACKTRACE ===\n"
    bt
    
    # Show kernel frame if on MSP
    printf "\n=== KERNEL FRAME ===\n"
    dump_kernel_frame $msp
    
    # Show user frame
    printf "\n=== USER FRAME ===\n"
    dump_exception_frame $psp
end
```

### 11.4 Check SpinLock State

```gdb
# Watch spinlock acquisition
break InterruptGuard::new
commands
    silent
    printf "SpinLock: acquiring (PRIMASK was %d)\n", $primask
    continue
end

# Watch spinlock release
break InterruptGuard::drop
commands
    silent
    printf "SpinLock: releasing (saved_primask=%d)\n", *(uint32_t*)($r0)
    continue
end
```

### 11.5 Check PendSV Scheduling

```gdb
# Verify PendSV is properly scheduled
define check_pendsv_pending
    set $icsr = *(uint32_t*)0xE000ED04
    if ($icsr & (1 << 28)) != 0
        printf "PendSV is PENDING\n"
    else
        printf "PendSV is NOT pending\n"
    end
end

# Check when context_switch drops the lock
break context_switch
commands
    silent
    printf "context_switch: about to drop scheduler lock\n"
    check_pendsv_pending
    continue
end
```

---

## 12. Hypothesis Testing Commands

For your specific issue where kernel addresses leak to user registers:

```gdb
# Hypothesis: PendSV saves kernel-mode state to handler's frame
# during syscall processing

# Test: Check CONTROL value when PendSV fires
break pendsv_swap_sp
commands
    set $ctrl = *(uint32_t*)($r0 + 0x24)  # control in frame
    printf "Saved CONTROL: 0x%08x\n", $ctrl
    if ($ctrl & 1) == 0
        printf "WARNING: nPRIV=0, this is KERNEL mode being saved!\n"
        printf "This frame should NOT be used for user thread return!\n"
    end
    continue
end

# Hypothesis: Wrong frame pointer in ArchThreadState
# Test: Compare canonical_return_address with saved return_address
break pendsv_swap_sp
commands
    set $thread = *(uint32_t*)&ACTIVE_THREAD
    if $thread != 0
        # Get saved frame
        set $frame = *(uint32_t*)$thread
        set $saved_ret = *(uint32_t*)($frame + 0x28)
        # Get canonical (need to find offset in ArchThreadState)
        set $canonical = *(uint32_t*)($thread + 20)  # adjust offset
        if $saved_ret != $canonical
            printf "MISMATCH: saved=0x%08x canonical=0x%08x\n", $saved_ret, $canonical
        end
    end
    continue
end
```
