# GDB Script for AST1030 IPC Stack Corruption Investigation
# Usage: gdb-multiarch <elf> -x debug.gdb -ex "target remote :3333"

# =============================================================================
# Logging Setup
# =============================================================================
set logging file ast1030-debug.log
set logging overwrite on
set logging enabled on
set pagination off
set confirm off

printf "=== AST1030 IPC Debug Session Started ===\n"

# =============================================================================
# Memory Region Constants
# =============================================================================
set $KERNEL_RAM_START = 0x60000
set $KERNEL_RAM_END   = 0x80000
set $HANDLER_FLASH_START = 0x40000
set $HANDLER_FLASH_END   = 0x60000
set $HANDLER_RAM_START   = 0x88000
set $HANDLER_RAM_END     = 0x90000

# System registers
set $ICSR_ADDR  = 0xE000ED04
set $SHPR1_ADDR = 0xE000ED18
set $SHPR2_ADDR = 0xE000ED1C
set $SHPR3_ADDR = 0xE000ED20
set $MMFSR_ADDR = 0xE000ED28
set $MMFAR_ADDR = 0xE000ED34

# KernelExceptionFrame offsets
set $FRAME_R4_OFF  = 0
set $FRAME_R5_OFF  = 4
set $FRAME_R6_OFF  = 8
set $FRAME_R7_OFF  = 12
set $FRAME_R8_OFF  = 16
set $FRAME_R9_OFF  = 20
set $FRAME_R10_OFF = 24
set $FRAME_R11_OFF = 28
set $FRAME_PSP_OFF = 32
set $FRAME_CTL_OFF = 36
set $FRAME_RET_OFF = 40

# =============================================================================
# Helper Commands
# =============================================================================

# Dump full CPU context
define dump_context
    printf "=== Full Context ===\n"
    printf "PC:  0x%08x  LR:  0x%08x\n", $pc, $lr
    printf "MSP: 0x%08x  PSP: 0x%08x\n", $msp, $psp
    printf "CONTROL: 0x%08x\n", $control
    set $icsr = *0xE000ED04
    printf "ICSR: 0x%08x (active=%d, pending=%d, pendSV=%d)\n", $icsr, $icsr & 0x1FF, ($icsr >> 12) & 0x1FF, ($icsr >> 28) & 1
    printf "r0:  0x%08x  r1:  0x%08x  r2:  0x%08x  r3:  0x%08x\n", $r0, $r1, $r2, $r3
    printf "r4:  0x%08x  r5:  0x%08x  r6:  0x%08x  r7:  0x%08x\n", $r4, $r5, $r6, $r7
    printf "r8:  0x%08x  r9:  0x%08x  r10: 0x%08x  r11: 0x%08x\n", $r8, $r9, $r10, $r11
    printf "r12: 0x%08x\n", $r12
end
document dump_context
Dump full CPU context including registers and exception state
end

# Dump KernelExceptionFrame at given address
define dump_frame
    if $argc == 0
        set $frame_addr = $r0
    else
        set $frame_addr = $arg0
    end
    printf "=== KernelExceptionFrame @ 0x%08x ===\n", $frame_addr
    printf "  r4:  0x%08x  r5:  0x%08x  r6:  0x%08x  r7:  0x%08x\n", \
        *($frame_addr + $FRAME_R4_OFF), \
        *($frame_addr + $FRAME_R5_OFF), \
        *($frame_addr + $FRAME_R6_OFF), \
        *($frame_addr + $FRAME_R7_OFF)
    printf "  r8:  0x%08x  r9:  0x%08x  r10: 0x%08x  r11: 0x%08x\n", \
        *($frame_addr + $FRAME_R8_OFF), \
        *($frame_addr + $FRAME_R9_OFF), \
        *($frame_addr + $FRAME_R10_OFF), \
        *($frame_addr + $FRAME_R11_OFF)
    printf "  psp: 0x%08x  control: 0x%08x  return: 0x%08x\n", \
        *($frame_addr + $FRAME_PSP_OFF), \
        *($frame_addr + $FRAME_CTL_OFF), \
        *($frame_addr + $FRAME_RET_OFF)
end
document dump_frame
Dump KernelExceptionFrame. Usage: dump_frame [address]
If no address given, uses $r0
end

# Validate frame for corruption
define validate_frame
    if $argc == 0
        set $vf_addr = $r0
    else
        set $vf_addr = $arg0
    end
    printf "=== Validating frame @ 0x%08x ===\n", $vf_addr
    
    set $vf_r4  = *($vf_addr + $FRAME_R4_OFF)
    set $vf_r5  = *($vf_addr + $FRAME_R5_OFF)
    set $vf_r6  = *($vf_addr + $FRAME_R6_OFF)
    set $vf_r7  = *($vf_addr + $FRAME_R7_OFF)
    set $vf_r8  = *($vf_addr + $FRAME_R8_OFF)
    set $vf_r9  = *($vf_addr + $FRAME_R9_OFF)
    set $vf_r10 = *($vf_addr + $FRAME_R10_OFF)
    set $vf_r11 = *($vf_addr + $FRAME_R11_OFF)
    set $vf_ctl = *($vf_addr + $FRAME_CTL_OFF)
    set $vf_ret = *($vf_addr + $FRAME_RET_OFF)
    
    # Check for kernel addresses in user registers
    set $corrupted = 0
    if $vf_r4 >= $KERNEL_RAM_START && $vf_r4 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r4 = 0x%08x is KERNEL address! ***\n", $vf_r4
        set $corrupted = 1
    end
    if $vf_r5 >= $KERNEL_RAM_START && $vf_r5 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r5 = 0x%08x is KERNEL address! ***\n", $vf_r5
        set $corrupted = 1
    end
    if $vf_r6 >= $KERNEL_RAM_START && $vf_r6 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r6 = 0x%08x is KERNEL address! ***\n", $vf_r6
        set $corrupted = 1
    end
    if $vf_r7 >= $KERNEL_RAM_START && $vf_r7 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r7 = 0x%08x is KERNEL address! ***\n", $vf_r7
        set $corrupted = 1
    end
    if $vf_r8 >= $KERNEL_RAM_START && $vf_r8 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r8 = 0x%08x is KERNEL address! ***\n", $vf_r8
        set $corrupted = 1
    end
    if $vf_r9 >= $KERNEL_RAM_START && $vf_r9 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r9 = 0x%08x is KERNEL address! ***\n", $vf_r9
        set $corrupted = 1
    end
    if $vf_r10 >= $KERNEL_RAM_START && $vf_r10 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r10 = 0x%08x is KERNEL address! ***\n", $vf_r10
        set $corrupted = 1
    end
    if $vf_r11 >= $KERNEL_RAM_START && $vf_r11 < $KERNEL_RAM_END
        printf "  *** CORRUPTION: r11 = 0x%08x is KERNEL address! ***\n", $vf_r11
        set $corrupted = 1
    end
    
    # Check CONTROL value
    if $vf_ctl != 0x00 && $vf_ctl != 0x01 && $vf_ctl != 0x02 && $vf_ctl != 0x03
        printf "  *** CORRUPTION: control = 0x%08x is invalid! ***\n", $vf_ctl
        set $corrupted = 1
    end
    
    # Check EXC_RETURN value
    if $vf_ret != 0xFFFFFFF1 && $vf_ret != 0xFFFFFFF9 && $vf_ret != 0xFFFFFFFD && \
       $vf_ret != 0xFFFFFFE1 && $vf_ret != 0xFFFFFFE9 && $vf_ret != 0xFFFFFFED
        printf "  *** CORRUPTION: return = 0x%08x is not valid EXC_RETURN! ***\n", $vf_ret
        set $corrupted = 1
    end
    
    if $corrupted == 0
        printf "  Frame appears valid.\n"
    end
end
document validate_frame
Validate KernelExceptionFrame for corruption. Usage: validate_frame [address]
end

# Check exception priorities
define check_priorities
    printf "=== Exception Priorities ===\n"
    set $shpr2 = *$SHPR2_ADDR
    set $shpr3 = *$SHPR3_ADDR
    set $svcall_pri = ($shpr2 >> 24) & 0xFF
    set $pendsv_pri = ($shpr3 >> 16) & 0xFF
    set $systick_pri = ($shpr3 >> 24) & 0xFF
    printf "  SVCall priority:  %d\n", $svcall_pri
    printf "  PendSV priority:  %d\n", $pendsv_pri
    printf "  SysTick priority: %d\n", $systick_pri
    if $pendsv_pri <= $svcall_pri
        printf "  *** WARNING: PendSV can preempt SVCall! This may cause corruption! ***\n"
    else
        printf "  OK: PendSV priority > SVCall priority (PendSV cannot preempt SVCall)\n"
    end
end
document check_priorities
Check exception priorities to verify PendSV cannot preempt SVCall
end

# Check active exceptions
define check_exceptions
    printf "=== Active/Pending Exceptions ===\n"
    set $icsr = *$ICSR_ADDR
    set $active = $icsr & 0x1FF
    set $pending = ($icsr >> 12) & 0x1FF
    set $pendsvset = ($icsr >> 28) & 1
    printf "  VECTACTIVE:  %d", $active
    if $active == 0
        printf " (Thread mode)\n"
    else
        if $active == 11
            printf " (SVCall)\n"
        else
            if $active == 14
                printf " (PendSV)\n"
            else
                if $active == 15
                    printf " (SysTick)\n"
                else
                    printf "\n"
                end
            end
        end
    end
    printf "  VECTPENDING: %d\n", $pending
    printf "  PENDSVSET:   %d\n", $pendsvset
end
document check_exceptions
Show active and pending exceptions from ICSR register
end

# Check MemManage fault status
define check_mmfault
    printf "=== MemManage Fault Status ===\n"
    set $mmfsr = *(char*)$MMFSR_ADDR
    set $mmfar = *$MMFAR_ADDR
    printf "  MMFSR: 0x%02x\n", $mmfsr
    if $mmfsr & 0x80
        printf "    MMARVALID: MMFAR holds valid address\n"
    end
    if $mmfsr & 0x10
        printf "    MSTKERR: Stacking error\n"
    end
    if $mmfsr & 0x08
        printf "    MUNSTKERR: Unstacking error\n"
    end
    if $mmfsr & 0x02
        printf "    DACCVIOL: Data access violation\n"
    end
    if $mmfsr & 0x01
        printf "    IACCVIOL: Instruction access violation\n"
    end
    printf "  MMFAR: 0x%08x\n", $mmfar
end
document check_mmfault
Check MemManage fault status registers
end

# Continue with timestamp (for timing analysis)
define tc
    shell date "+[%H:%M:%S.%3N]" | tr -d '\n'
    printf " continuing...\n"
    continue
end
document tc
Continue execution with timestamp prefix (for timing analysis in logs)
end

# =============================================================================
# Breakpoints with Logging
# =============================================================================

# SVCall handler entry - use *SVCall to break at exact entry, not after prologue
break *SVCall
commands
    silent
    printf "\n[SVCall] Entry\n"
    printf "  MSP: 0x%08x  PSP: 0x%08x  LR: 0x%08x\n", $msp, $psp, $lr
    printf "  r11 (syscall ID): 0x%08x\n", $r11
    continue
end

# handle_svc - Rust syscall dispatcher - use *handle_svc for exact entry
break *handle_svc
commands
    silent
    printf "\n[handle_svc] frame_ptr=0x%08x\n", $r0
    set $last_frame = $r0
    dump_frame $r0
    continue
end

# svc_return - before returning to user mode - use * for exact entry
break *svc_return
commands
    silent
    printf "\n[svc_return] frame_ptr=0x%08x\n", $r0
    validate_frame $r0
    continue
end

# PendSV handler - context switch - use * for exact entry
# NOTE: Disabled by default - too noisy! Enable with: enable_pendsv
break *PendSV
commands
    silent
    printf "\n[PendSV] Entry\n"
    printf "  MSP: 0x%08x  LR: 0x%08x\n", $msp, $lr
    check_exceptions
    # Check if we interrupted SVCall
    set $icsr = *$ICSR_ADDR
    if ($icsr & 0x1FF) == 11
        printf "  *** BUG: PendSV preempted SVCall! ***\n"
    end
    continue
end
# Disable PendSV breakpoint by default (it's breakpoint 4)
disable 4

define enable_pendsv
    enable 4
    printf "PendSV breakpoint enabled\n"
end
document enable_pendsv
Enable the noisy PendSV breakpoint (disabled by default)
end

define disable_pendsv
    disable 4
    printf "PendSV breakpoint disabled\n"
end
document disable_pendsv
Disable the noisy PendSV breakpoint
end

# MemoryManagement fault - use * for exact entry
break *MemoryManagement
commands
    printf "\n[MemoryManagement] FAULT!\n"
    check_mmfault
    dump_context
    # Don't continue - stop for analysis
end

# =============================================================================
# Watchpoint Setup (call these manually when you have the frame address)
# =============================================================================

# Set watchpoint on a frame's r8 field
define watch_r8
    if $argc == 0
        printf "Usage: watch_r8 <frame_address>\n"
    else
        set $wr8_addr = $arg0 + $FRAME_R8_OFF
        printf "Setting watchpoint on r8 field at 0x%08x\n", $wr8_addr
        watch *$wr8_addr
        commands
            printf "\n*** r8 field modified! ***\n"
            printf "  New value: 0x%08x\n", *$wr8_addr
            printf "  PC: 0x%08x\n", $pc
            backtrace
        end
    end
end
document watch_r8
Set watchpoint on frame's r8 field. Usage: watch_r8 <frame_address>
end

# Set watchpoint on entire frame
define watch_frame
    if $argc == 0
        printf "Usage: watch_frame <frame_address>\n"
    else
        set $wf_addr = $arg0
        printf "Setting watchpoints on frame at 0x%08x\n", $wf_addr
        # Watch r4-r11 (8 registers = 32 bytes) - use awatch for any access
        awatch *$wf_addr
        commands
            printf "\n*** Frame region accessed! ***\n"
            dump_frame $wf_addr
        end
    end
end
document watch_frame
Set watchpoint on frame's r4-r11 region. Usage: watch_frame <frame_address>
end

# =============================================================================
# object_wait Hang Investigation Commands
# =============================================================================

# Syscall counter for tracking which call hangs
set $syscall_count = 0

# Track PRIMASK through syscall flow
define check_primask
    if $primask == 1
        printf "!! PRIMASK=1 (interrupts DISABLED) !!\n"
    else
        printf "PRIMASK=0 (interrupts enabled)\n"
    end
end
document check_primask
Check if interrupts are disabled (PRIMASK=1)
end

# Diagnose system when hung (after Ctrl+C)
define diagnose_hang
    printf "\n=== HANG DIAGNOSTICS ===\n"
    printf "Syscall count: %d\n", $syscall_count
    printf "PC: 0x%08x  LR: 0x%08x\n", $pc, $lr
    printf "MSP: 0x%08x  PSP: 0x%08x\n", $msp, $psp
    printf "CONTROL: 0x%08x  PRIMASK: 0x%08x\n", $control, $primask
    printf "xPSR: 0x%08x (exception#=%d)\n", $xpsr, $xpsr & 0x1ff
    
    if $primask == 1
        printf "\n!! INTERRUPTS DISABLED - This is likely the hang cause !!\n"
    end
    
    if ($xpsr & 0x1ff) != 0
        printf "Mode: Handler (exception %d)\n", $xpsr & 0x1ff
    else
        printf "Mode: Thread\n"
    end
    
    check_exceptions
    printf "\n=== BACKTRACE ===\n"
    backtrace
end
document diagnose_hang
Run this after Ctrl+C when system hangs to diagnose state
end

# Check if PendSV is pending but not firing (because interrupts disabled)
define check_pendsv_state
    set $icsr = *$ICSR_ADDR
    printf "PendSV state:\n"
    if ($icsr >> 28) & 1
        printf "  PENDSVSET=1 (PendSV is PENDING)\n"
        if $primask == 1
            printf "  !! But PRIMASK=1 so it can't fire !!\n"
        end
    else
        printf "  PENDSVSET=0 (PendSV not pending)\n"
    end
    if ($icsr >> 27) & 1
        printf "  PENDSVCLR active\n"
    end
end
document check_pendsv_state
Check if PendSV is pending but blocked by disabled interrupts
end

# Enable verbose syscall counting (tracks each SVCall entry/exit)
define enable_syscall_counting
    # Reset counter
    set $syscall_count = 0
    set $initial_msp = 0
    
    # Modify SVCall breakpoint to count and track MSP drift
    delete 1
    break *SVCall
    commands
        silent
        set $syscall_count = $syscall_count + 1
        if $initial_msp == 0
            set $initial_msp = $msp
        end
        set $msp_drift = $initial_msp - $msp
        printf "[SVCall #%d] MSP=0x%08x (drift=%d) LR=0x%08x PRIMASK=%d\n", $syscall_count, $msp, $msp_drift, $lr, $primask
        continue
    end
    
    # Add svc_return tracking with frame dump
    break *svc_return
    commands
        silent  
        printf "[svc_return #%d] frame=0x%08x PRIMASK=%d\n", $syscall_count, $r0, $primask
        # Dump first 12 words of frame (r4-r11, psp, control, return, + hw frame start)
        x/12wx $r0
        continue
    end
    
    printf "Syscall counting enabled with MSP drift tracking. Run 'continue' to start.\n"
    printf "When it hangs/faults, check the MSP drift to see stack growth.\n"
    printf "Run 'diagnose_hang' after Ctrl+C.\n"
end
document enable_syscall_counting
Enable syscall counting with MSP drift tracking to find stack corruption
end

# Enable deep frame validation (slower, validates frame at every svc_return)
define enable_frame_validation
    set $syscall_count = 0
    
    delete 1
    break *SVCall
    commands
        silent
        set $syscall_count = $syscall_count + 1
        printf "[SVCall #%d] Entry MSP=0x%08x\n", $syscall_count, $msp
        continue
    end
    
    break *svc_return
    commands
        silent
        printf "\n[svc_return #%d] Validating frame at 0x%08x\n", $syscall_count, $r0
        validate_frame $r0
        continue
    end
    
    printf "Frame validation enabled. Every svc_return will validate the frame.\n"
end
document enable_frame_validation
Enable deep frame validation at every svc_return (slower but catches corruption)
end

# =============================================================================
# Quick Start Commands
# =============================================================================

define start_debug
    printf "\n=== IPC Debug Session Quick Start ===\n"
    printf "\nBreakpoints pre-configured for:\n"
    printf "  SVCall, handle_svc, svc_return, PendSV (disabled), MemoryManagement\n"
    printf "\n=== For STACK CORRUPTION / MSP DRIFT investigation ===\n"
    printf "  1. Run 'enable_syscall_counting' to track MSP drift\n"
    printf "  2. Run 'continue' - watch for MSP drift growing\n"
    printf "  3. Fault at syscall #N means corruption happens before\n"
    printf "  4. Run 'enable_frame_validation' for deep frame checks\n"
    printf "\n=== For object_wait HANG investigation ===\n"
    printf "  1. Run 'enable_syscall_counting' to track syscalls\n"
    printf "  2. Run 'continue' - system will hang after ~39 calls\n"
    printf "  3. Press Ctrl+C when it hangs\n"
    printf "  4. Run 'diagnose_hang' to see the state\n"
    printf "  5. Run 'check_pendsv_state' to see if PendSV blocked\n"
    printf "\n=== For specific frame corruption investigation ===\n"
    printf "  1. Run 'continue' to start execution\n"
    printf "  2. After first syscall hits, run 'check_priorities'\n"
    printf "  3. Watch for [handle_svc] with r11=0x00000000 (ObjectWait)\n"
    printf "  4. Note the frame_ptr address from that log line\n"
    printf "  5. Ctrl+C, then: watch_r8 <frame_address>\n"
    printf "  6. Run 'continue' - watchpoint triggers on corruption\n"
    printf "\nUseful commands:\n"
    printf "  enable_syscall_counting - Track syscalls + MSP drift + frame dump\n"
    printf "  enable_frame_validation - Deep frame validation (slower)\n"
    printf "  diagnose_hang     - Run after Ctrl+C when system hangs\n"
    printf "  check_primask     - Check if interrupts disabled\n"
    printf "  check_pendsv_state - Check if PendSV pending but blocked\n"
    printf "  check_priorities  - Verify SVCall/PendSV priority config\n"
    printf "  check_exceptions  - Show active/pending exceptions\n"
    printf "  dump_context      - Full CPU state dump\n"
    printf "  dump_frame <addr> - Show KernelExceptionFrame contents\n"
    printf "  validate_frame <addr> - Check frame for kernel address leaks\n"
end
document start_debug
Show quick-start info for IPC debug session
end

# =============================================================================
# Auto-run on load
# =============================================================================
printf "\n=== Debug commands loaded ===\n"
printf "Commands available:\n"
printf "  === SYSCALL/STACK TRACKING ===\n"
printf "  enable_syscall_counting - Track syscalls + MSP drift + frame dump\n"
printf "  enable_frame_validation - Deep frame validation at each svc_return\n"
printf "  === HANG INVESTIGATION ===\n"
printf "  diagnose_hang   - Run after Ctrl+C when system hangs\n"
printf "  check_primask   - Check if interrupts disabled (PRIMASK)\n"
printf "  check_pendsv_state - Check if PendSV pending but blocked\n"
printf "  === GENERAL ===\n"
printf "  dump_context    - Show all registers and exception state\n"
printf "  dump_frame [addr] - Dump KernelExceptionFrame (default: $r0)\n"
printf "  validate_frame [addr] - Check frame for corruption\n"
printf "  check_priorities - Verify exception priority config\n"
printf "  check_exceptions - Show active/pending exceptions\n"
printf "  check_mmfault   - Show MemManage fault details\n"
printf "  watch_r8 <addr> - Set watchpoint on frame's r8 field\n"
printf "  watch_frame <addr> - Set watchpoint on frame's registers\n"
printf "  tc              - Continue with timestamp\n"
printf "  start_debug     - Show quick-start info\n"
printf "\nBreakpoints pre-configured:\n"
printf "  SVCall, handle_svc, svc_return, PendSV (disabled), MemoryManagement\n"
printf "\nRun 'start_debug' to begin.\n"
