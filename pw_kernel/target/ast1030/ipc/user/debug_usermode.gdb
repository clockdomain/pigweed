# ARMv7-M User Mode Debug Script
# Usage: gdb-multiarch -x docs/debug_usermode.gdb bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

#target remote :3333

# Helper to dump CPU state
define dump_cpu
  printf "=== CPU State ===\n"
  printf "PC=0x%08x  LR=0x%08x  SP=0x%08x\n", $pc, $lr, $sp
  printf "PSP=0x%08x MSP=0x%08x CONTROL=0x%08x\n", $psp, $msp, $control
  printf "xPSR=0x%08x (IPSR=%d)\n", $xpsr, $xpsr & 0x1FF
end
document dump_cpu
Dump current CPU state including PC, LR, SP, PSP, MSP, CONTROL, and xPSR.
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
document dump_faults
Dump fault status registers (CFSR, HFSR, MMFAR, BFAR).
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
document dump_mpu
Dump MPU configuration showing all enabled regions.
end

# Helper to dump KernelExceptionFrame at given address
define dump_kef
  if $argc != 1
    printf "Usage: dump_kef <address>\n"
  else
    set $addr = $arg0
    printf "=== KernelExceptionFrame at 0x%08x ===\n", $addr
    printf "r4 =0x%08x  r5 =0x%08x  r6 =0x%08x  r7 =0x%08x\n", *(unsigned int*)($addr+0x00), *(unsigned int*)($addr+0x04), *(unsigned int*)($addr+0x08), *(unsigned int*)($addr+0x0C)
    printf "r8 =0x%08x  r9 =0x%08x  r10=0x%08x  r11=0x%08x\n", *(unsigned int*)($addr+0x10), *(unsigned int*)($addr+0x14), *(unsigned int*)($addr+0x18), *(unsigned int*)($addr+0x1C)
    printf "psp=0x%08x  control=0x%08x  exc_return=0x%08x\n", *(unsigned int*)($addr+0x20), *(unsigned int*)($addr+0x24), *(unsigned int*)($addr+0x28)
  end
end
document dump_kef
Dump KernelExceptionFrame at the given address.
Usage: dump_kef <address>
end

# Helper to dump PendSV frame (FullExceptionFrame passed to pendsv_swap_sp in r0)
define dump_pendsv_frame
  set $addr = $r0
  printf "=== PendSV FullExceptionFrame at 0x%08x ===\n", $addr
  printf "r4 =0x%08x  r5 =0x%08x  r6 =0x%08x  r7 =0x%08x\n", *(unsigned int*)($addr+0x00), *(unsigned int*)($addr+0x04), *(unsigned int*)($addr+0x08), *(unsigned int*)($addr+0x0C)
  printf "r8 =0x%08x  r9 =0x%08x  r10=0x%08x  r11=0x%08x\n", *(unsigned int*)($addr+0x10), *(unsigned int*)($addr+0x14), *(unsigned int*)($addr+0x18), *(unsigned int*)($addr+0x1C)
  set $psp_val = *(unsigned int*)($addr+0x20)
  set $ctrl_val = *(unsigned int*)($addr+0x24)
  set $exc_ret = *(unsigned int*)($addr+0x28)
  printf "psp=0x%08x  control=0x%08x  exc_return=0x%08x\n", $psp_val, $ctrl_val, $exc_ret
  printf "\n"
  # Decode CONTROL
  printf "CONTROL decode: nPRIV=%d SPSEL=%d", $ctrl_val & 1, ($ctrl_val >> 1) & 1
  if $ctrl_val == 0
    printf " [KERNEL MODE]\n"
  end
  if $ctrl_val == 3
    printf " [USER MODE - correct]\n"
  end
  if $ctrl_val == 2
    printf " [PRIVILEGED+PSP - CORRUPTED?]\n"
  end
  if $ctrl_val == 1
    printf " [UNPRIVILEGED+MSP - INVALID!]\n"
  end
  # Decode EXC_RETURN
  printf "EXC_RETURN decode: "
  if $exc_ret == 0xFFFFFFF9
    printf "Return to Handler mode, MSP\n"
  end
  if $exc_ret == 0xFFFFFFFD
    printf "Return to Thread mode, PSP [USER]\n"
  end
  if $exc_ret == 0xFFFFFFE9
    printf "Return to Handler mode, MSP (FPU)\n"
  end
  if $exc_ret == 0xFFFFFFED
    printf "Return to Thread mode, PSP (FPU) [USER]\n"
  end
end
document dump_pendsv_frame
Dump the FullExceptionFrame passed to pendsv_swap_sp (uses r0).
Call this when stopped inside pendsv_swap_sp.
end

# Helper to dump hardware ExceptionFrame at given address (what hardware pushes/pops)
define dump_ef
  if $argc != 1
    printf "Usage: dump_ef <address>\n"
  else
    set $addr = $arg0
    printf "=== ExceptionFrame at 0x%08x ===\n", $addr
    printf "r0 =0x%08x  r1 =0x%08x  r2 =0x%08x  r3 =0x%08x\n", *(unsigned int*)($addr+0x00), *(unsigned int*)($addr+0x04), *(unsigned int*)($addr+0x08), *(unsigned int*)($addr+0x0C)
    printf "r12=0x%08x  lr =0x%08x  pc =0x%08x  xpsr=0x%08x\n", *(unsigned int*)($addr+0x10), *(unsigned int*)($addr+0x14), *(unsigned int*)($addr+0x18), *(unsigned int*)($addr+0x1C)
    set $pc_val = *(unsigned int*)($addr+0x18)
    set $xpsr_val = *(unsigned int*)($addr+0x1C)
    if $xpsr_val & 0x01000000
      printf "Thumb bit: SET (good)\n"
    else
      printf "Thumb bit: NOT SET (BAD - will cause UsageFault!)\n"
    end
  end
end
document dump_ef
Dump hardware ExceptionFrame at the given address.
Usage: dump_ef <address>
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

break BusFault
commands
  silent
  printf "\n!!! BusFault !!!\n"
  dump_cpu
  dump_faults
end

# Break on PendSV (context switch)
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
printf "  dump_cpu       - Show CPU registers\n"
printf "  dump_faults    - Show fault status registers\n"
printf "  dump_mpu       - Show MPU configuration\n"
printf "  dump_kef <addr> - Dump KernelExceptionFrame\n"
printf "  dump_ef <addr>  - Dump hardware ExceptionFrame\n"
printf "\n"
printf "Breakpoints set on: HardFault, MemoryManagement, UsageFault, BusFault, PendSV\n"
printf "\n"
printf "Type 'continue' to start execution\n"
printf "\n"
