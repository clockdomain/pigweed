# GDB Debug Strategy: Stack Corruption During Syscall/Context Switch

## Target Information

- **SoC**: AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7 MPU)
- **Emulator**: QEMU (`qemu-system-arm -machine ast1030-evb`)
- **Debugger**: GDB with ARM support

## Observed Symptoms

| Symptom | Value | Interpretation |
|---------|-------|----------------|
| Fault address | `0x00061778` | Kernel RAM (0x60000-0x80000) |
| Faulting PC | `0x000408d4` | Handler user-space flash (0x40000-0x60000) |
| r1, r8 | `0x00061778` | **Kernel addresses leaked to user registers!** |
| LR | `0x00000003` | **Invalid!** Looks like CONTROL value, not return address |
| CONTROL | `0x00000001` | nPRIV=1 (user mode) ✓ |
| PSP | `0x0008fea8` | Handler RAM (0x88000-0x90000) ✓ |

## Root Cause Hypothesis

The `svc_return()` code restores r4-r11 from the KernelExceptionFrame on the kernel stack. The corruption suggests:
1. **Wrong thread's frame** is being restored, OR
2. **Frame was overwritten** after saving, OR
3. **PendSV preempted SVCall** during frame setup

---

## Phase 1: QEMU GDB Connection


### Quick Start (tmux one-liner)

```bash
# Build first
cd /home/rusty1968/workstreams/rtos/pigweed-rusty
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc

# Start tmux with QEMU+detokenizer (left pane) and GDB (right pane)
tmux new-session -s ast1030-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -semihosting-config enable=on,target=native -S -s 2>&1 | python3 -m pw_tokenizer.detokenize base64 bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -x pw_kernel/target/ast1030/ipc-investigation/debug.gdb -ex "target remote :1234"' C-m
```

### object_wait Hang Investigation (tmux one-liner)

```bash
# Build the hello_user test first (contains object_wait calls)
cd /home/rusty1968/workstreams/rtos/pigweed-rusty
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/hello_user:hello_user

# Option A: With detokenized logs (requires Pigweed environment)
source activate.sh && tmux new-session -s object-wait-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf -semihosting-config enable=on,target=native -S -s 2>&1 | python3 -m pw_tokenizer.detokenize base64 bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf -x pw_kernel/target/ast1030/ipc-investigation/debug.gdb -ex "target remote :1234"' C-m

# Option B: Without detokenization (raw base64 logs, but GDB works fine)
tmux new-session -s object-wait-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf -semihosting-config enable=on,target=native -S -s' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf -x pw_kernel/target/ast1030/ipc-investigation/debug.gdb -ex "target remote :1234"' C-m

# In GDB, run these commands to track the hang:
#   (gdb) enable_syscall_counting
#   (gdb) continue
#   ... wait for hang (~39 syscalls) ...
#   Ctrl+C
#   (gdb) diagnose_hang
#   (gdb) check_pendsv_state
```

**Note**: The detokenizer pipe decodes tokenized log messages. If you don't need readable logs (just GDB), you can omit the `| python -m pw_tokenizer.detokenize ...` part.

### GDB Script File

The [debug.gdb](debug.gdb) script in this folder contains:
- **Logging**: Auto-enabled to `ast1030-debug.log`
- **Helper commands**: `dump_context`, `dump_frame`, `validate_frame`, `check_priorities`, etc.
- **Pre-configured breakpoints**: SVCall, handle_svc, svc_return, PendSV, MemoryManagement
- **Watchpoint helpers**: `watch_r8 <addr>`, `watch_frame <addr>`

Run `start_debug` in GDB after connecting to see quick-start info.

**tmux tips:**
- `Ctrl-b o` - Switch between panes
- `Ctrl-b d` - Detach from session
- `tmux attach -t ast1030-debug` - Reattach to session
- `tmux kill-session -t ast1030-debug` - Kill session

### Manual Setup (alternative)

```bash
# In one terminal - run the test with GDB server
cd /home/rusty1968/workstreams/rtos/pigweed-rusty
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc_test

# Run QEMU with GDB stub (port 3333)
qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none \
    -nographic -serial mon:stdio \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
    -semihosting-config enable=on,target=native \
    -S -gdb tcp::3333
```

### Connecting GDB

```bash
# In another terminal
gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

# In GDB
(gdb) target remote :3333
```

---

## Phase 1b: GDB Logging to File

Enable logging to capture the entire debug session for later analysis:

```gdb
# Set the log file name
(gdb) set logging file ast1030-debug.log

# Overwrite existing log (vs append)
(gdb) set logging overwrite on

# Enable logging (output goes to both terminal and file)
(gdb) set logging enabled on

# Alternative: redirect output ONLY to file (no terminal output)
(gdb) set logging redirect on
(gdb) set logging enabled on

# Disable logging when done
(gdb) set logging enabled off
```

### Logging with Timestamps

For timing analysis, create a GDB command that adds timestamps:

```gdb
# Define a command that prints timestamp before each continue
define tc
    shell date "+[%H:%M:%S.%3N]" | tr -d '\n'
    printf " continuing...\n"
    continue
end

# Use 'tc' instead of 'continue' to get timestamps in log
```

### Quick Start with Logging

Add to your tmux command to auto-enable logging:

```bash
tmux new-session -s ast1030-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -semihosting-config enable=on,target=native -S -gdb tcp::3333' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -ex "target remote :3333" -ex "set logging file ast1030-debug.log" -ex "set logging overwrite on" -ex "set logging enabled on"' C-m
```

---

## Phase 2: ARMv7-M Register Inspection

### Core Registers

```gdb
# View all registers
(gdb) info registers

# Specific registers
(gdb) print/x $msp        # Main Stack Pointer (kernel)
(gdb) print/x $psp        # Process Stack Pointer (user)
(gdb) print/x $control    # CONTROL register
(gdb) print/x $lr         # Link Register (EXC_RETURN during exception)
(gdb) print/x $pc         # Program Counter

# Special: read CONTROL manually (may need this)
(gdb) x/wx 0xE000ED14     # Read CONTROL via debug access
```

### Understanding CONTROL Register

```
CONTROL[0] = nPRIV  (0=privileged, 1=unprivileged)
CONTROL[1] = SPSEL  (0=MSP, 1=PSP)
CONTROL[2] = FPCA   (floating-point context active)

Expected values:
  Kernel mode:  CONTROL = 0x00 (privileged, MSP)
  User mode:    CONTROL = 0x03 (unprivileged, PSP)
```

### Understanding EXC_RETURN

```
EXC_RETURN Values (in LR during exception):
  0xFFFFFFF1 = Return to Handler mode, MSP
  0xFFFFFFF9 = Return to Thread mode, MSP
  0xFFFFFFFD = Return to Thread mode, PSP  ← Correct for user return
  0xFFFFFFE1 = Return to Handler mode, MSP, FP frame
  0xFFFFFFE9 = Return to Thread mode, MSP, FP frame
  0xFFFFFFED = Return to Thread mode, PSP, FP frame
```

---

## Phase 3: Critical Breakpoints

### SVCall Entry/Exit

```gdb
# Break at SVCall handler entry
(gdb) break SVCall
(gdb) commands
    silent
    printf "=== SVCall Entry ===\n"
    printf "  MSP: 0x%08x\n", $msp
    printf "  PSP: 0x%08x\n", $psp
    printf "  LR:  0x%08x (EXC_RETURN)\n", $lr
    printf "  r11: 0x%08x (syscall ID)\n", $r11
    continue
end

# Break at handle_svc (Rust function)
(gdb) break handle_svc
(gdb) commands
    silent
    printf "=== handle_svc ===\n"
    printf "  frame_ptr: 0x%08x\n", $r0
    # Dump the KernelExceptionFrame
    printf "  frame.r4:  0x%08x\n", *(uint32_t*)($r0 + 0)
    printf "  frame.r5:  0x%08x\n", *(uint32_t*)($r0 + 4)
    printf "  frame.r6:  0x%08x\n", *(uint32_t*)($r0 + 8)
    printf "  frame.r7:  0x%08x\n", *(uint32_t*)($r0 + 12)
    printf "  frame.r8:  0x%08x\n", *(uint32_t*)($r0 + 16)
    printf "  frame.r9:  0x%08x\n", *(uint32_t*)($r0 + 20)
    printf "  frame.r10: 0x%08x\n", *(uint32_t*)($r0 + 24)
    printf "  frame.r11: 0x%08x\n", *(uint32_t*)($r0 + 28)
    printf "  frame.psp: 0x%08x\n", *(uint32_t*)($r0 + 32)
    printf "  frame.ctl: 0x%08x\n", *(uint32_t*)($r0 + 36)
    printf "  frame.ret: 0x%08x\n", *(uint32_t*)($r0 + 40)
    continue
end

# Break at svc_return
(gdb) break svc_return
(gdb) commands
    silent
    printf "=== svc_return ===\n"
    printf "  r0 (frame_ptr): 0x%08x\n", $r0
    # Check frame contents BEFORE restore
    printf "  frame.r8: 0x%08x\n", *(uint32_t*)($r0 + 16)
    if (*(uint32_t*)($r0 + 16) >= 0x60000 && *(uint32_t*)($r0 + 16) < 0x80000)
        printf "  *** WARNING: r8 contains kernel address! ***\n"
    end
    continue
end
```

### PendSV Context Switch

```gdb
# Break at PendSV handler
(gdb) break PendSV
(gdb) commands
    silent
    printf "=== PendSV Entry ===\n"
    printf "  MSP: 0x%08x\n", $msp
    printf "  LR:  0x%08x\n", $lr
    continue
end

# If there's a pendsv_swap_sp or similar function
(gdb) break pendsv_swap_sp
(gdb) commands
    silent
    printf "=== pendsv_swap_sp ===\n"
    printf "  old_sp (r0): 0x%08x\n", $r0
    printf "  new_sp (r1): 0x%08x\n", $r1
    continue
end
```

### MemoryManagement Fault Handler

```gdb
# Break at the fault handler
(gdb) break MemoryManagement
(gdb) commands
    printf "=== MemoryManagement Fault! ===\n"
    # Read MMFSR (MemManage Fault Status Register)
    printf "  MMFSR: 0x%02x\n", *(uint8_t*)0xE000ED28
    # Read MMFAR (MemManage Fault Address Register)
    printf "  MMFAR: 0x%08x\n", *(uint32_t*)0xE000ED34
    # Dump registers
    info registers
end
```

---

## Phase 4: Memory Watchpoints

### Watch for Kernel Address Being Written to Frame

```gdb
# First, find the handler thread's kernel stack frame address
# Look at trace output for "Kernel exception frame 0x0616cc"
# Then set watchpoint on r8 field (offset 16 from frame start)

(gdb) watch *(uint32_t*)0x0616dc
(gdb) commands
    printf "=== r8 field modified! ===\n"
    printf "  New value: 0x%08x\n", *(uint32_t*)0x0616dc
    printf "  PC: 0x%08x\n", $pc
    backtrace
end

# Watch for the specific kernel address being written anywhere
(gdb) watch *(uint32_t*)0x00061778
(gdb) commands
    printf "=== Address 0x61778 written! ===\n"
    backtrace
end
```

### Watch MSP Changes

```gdb
# Hardware watchpoint on MSP changing
# This is tricky - MSP changes during every exception
# Better to check at specific points

(gdb) break handle_svc
(gdb) commands
    set $saved_msp = $msp
    continue
end

(gdb) break svc_return  
(gdb) commands
    if $msp != $saved_msp
        printf "*** MSP changed during syscall! ***\n"
        printf "  Before: 0x%08x\n", $saved_msp
        printf "  After:  0x%08x\n", $msp
    end
    continue
end
```

---

## Phase 5: Exception Priority Verification

### Check SHPR Registers

```gdb
# System Handler Priority Registers
(gdb) x/wx 0xE000ED18    # SHPR1 (MemManage, BusFault, UsageFault)
(gdb) x/wx 0xE000ED1C    # SHPR2 (SVCall priority in bits 31:24)
(gdb) x/wx 0xE000ED20    # SHPR3 (PendSV in bits 23:16, SysTick in bits 31:24)

# Parse the priorities
(gdb) set $shpr2 = *(uint32_t*)0xE000ED1C
(gdb) set $shpr3 = *(uint32_t*)0xE000ED20
(gdb) printf "SVCall priority: %d\n", ($shpr2 >> 24) & 0xFF
(gdb) printf "PendSV priority: %d\n", ($shpr3 >> 16) & 0xFF
(gdb) printf "SysTick priority: %d\n", ($shpr3 >> 24) & 0xFF

# CRITICAL: PendSV priority must be > SVCall priority (higher number = lower priority)
# If PendSV can preempt SVCall, stack corruption can occur!
```

### Check Active/Pending Exceptions

```gdb
# ICSR - Interrupt Control and State Register
(gdb) x/wx 0xE000ED04

# Parse active exceptions
(gdb) set $icsr = *(uint32_t*)0xE000ED04
(gdb) printf "VECTACTIVE: %d\n", $icsr & 0x1FF       # Currently active exception
(gdb) printf "VECTPENDING: %d\n", ($icsr >> 12) & 0x1FF  # Highest pending
(gdb) printf "PENDSVSET: %d\n", ($icsr >> 28) & 1   # PendSV pending
(gdb) printf "PENDSVCLR: %d\n", ($icsr >> 27) & 1

# Exception numbers:
#   2 = NMI
#   3 = HardFault
#   4 = MemManage
#   11 = SVCall
#   14 = PendSV
#   15 = SysTick
```

---

## Phase 6: Thread State Inspection

### Find Thread Structures

```gdb
# Search for the scheduler and thread structures
# These names may vary - check your codebase

(gdb) info variables scheduler
(gdb) info variables current_thread
(gdb) info variables active_thread

# Once you find the scheduler address, dump thread info
# Example (addresses will vary):
(gdb) set $sched = *(uint32_t*)0x60000
(gdb) p/x *((Thread*)$sched)
```

### Verify active_thread Consistency

```gdb
# The active_thread global is used by PendSV
# Check it at key points

(gdb) break context_switch
(gdb) commands
    printf "context_switch: active_thread = 0x%08x\n", active_thread
    continue
end
```

---

## Phase 7: Stack Frame Validation Script

### GDB Python Script for Frame Validation

Save as `validate_frame.py`:

```python
import gdb

class ValidateKernelFrame(gdb.Command):
    """Validate a KernelExceptionFrame for corruption"""
    
    def __init__(self):
        super().__init__("validate-frame", gdb.COMMAND_USER)
    
    def invoke(self, arg, from_tty):
        frame_addr = int(arg, 16) if arg else int(gdb.parse_and_eval("$r0"))
        
        # Read frame fields (adjust offsets for your struct)
        r4 = int(gdb.parse_and_eval(f"*(uint32_t*){frame_addr}"))
        r5 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+4)"))
        r6 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+8)"))
        r7 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+12)"))
        r8 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+16)"))
        r9 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+20)"))
        r10 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+24)"))
        r11 = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+28)"))
        psp = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+32)"))
        control = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+36)"))
        ret_addr = int(gdb.parse_and_eval(f"*(uint32_t*)({frame_addr}+40)"))
        
        KERNEL_RAM_START = 0x60000
        KERNEL_RAM_END = 0x80000
        
        print(f"KernelExceptionFrame @ 0x{frame_addr:08x}")
        print(f"  r4:  0x{r4:08x}  r5:  0x{r5:08x}  r6:  0x{r6:08x}  r7:  0x{r7:08x}")
        print(f"  r8:  0x{r8:08x}  r9:  0x{r9:08x}  r10: 0x{r10:08x}  r11: 0x{r11:08x}")
        print(f"  psp: 0x{psp:08x}  control: 0x{control:08x}  ret: 0x{ret_addr:08x}")
        
        # Check for kernel addresses in user registers
        for name, val in [("r4", r4), ("r5", r5), ("r6", r6), ("r7", r7),
                          ("r8", r8), ("r9", r9), ("r10", r10), ("r11", r11)]:
            if KERNEL_RAM_START <= val < KERNEL_RAM_END:
                print(f"  *** WARNING: {name} = 0x{val:08x} is KERNEL address! ***")
        
        # Check control value
        if control not in [0x00, 0x01, 0x02, 0x03]:
            print(f"  *** WARNING: control = 0x{control:08x} is invalid! ***")
        
        # Check return address (EXC_RETURN)
        valid_exc_return = [0xFFFFFFF1, 0xFFFFFFF9, 0xFFFFFFFD, 
                           0xFFFFFFE1, 0xFFFFFFE9, 0xFFFFFFED]
        if ret_addr not in valid_exc_return:
            print(f"  *** WARNING: return_address = 0x{ret_addr:08x} is not valid EXC_RETURN! ***")

ValidateKernelFrame()
```

Load in GDB:
```gdb
(gdb) source validate_frame.py
(gdb) validate-frame 0x0616cc
```

---

## Phase 8: Specific Investigation for This Bug

### Step-by-Step Debug Session

```gdb
# 1. Connect and set up
target remote :1234
break handle_svc
break svc_return
break MemoryManagement

# 2. Run until handler's object_wait syscall
continue
# Wait for multiple syscalls (DebugLog, etc.)
# When you see r11 = 0x0000 (ObjectWait), stop

# 3. At handle_svc, record the frame address
(gdb) set $handler_frame = $r0
(gdb) printf "Handler syscall frame: 0x%08x\n", $handler_frame

# 4. Dump the frame contents
(gdb) validate-frame $handler_frame

# 5. Set watchpoint on the r8 field
(gdb) watch *(uint32_t*)($handler_frame + 16)

# 6. Continue - should trigger on corruption
continue

# 7. When watchpoint triggers, check:
#    - What code modified the frame?
#    - Was this a different thread's context switch?
#    - Did PendSV fire unexpectedly?
```

### Check for PendSV Preempting SVCall

```gdb
# Set breakpoint at PendSV entry
break PendSV
commands
    # Check if we're inside SVCall
    set $icsr = *(uint32_t*)0xE000ED04
    set $vectactive = $icsr & 0x1FF
    if $vectactive == 11
        printf "*** BUG: PendSV preempted SVCall! ***\n"
    end
    continue
end
```

---

## Phase 9: Quick Diagnostic Commands

### One-Liner Diagnostic

```gdb
# At any breakpoint, run this to get full context
define dump_context
    printf "=== Full Context ===\n"
    printf "PC:  0x%08x  LR:  0x%08x\n", $pc, $lr
    printf "MSP: 0x%08x  PSP: 0x%08x\n", $msp, $psp
    printf "CONTROL: 0x%08x\n", $control
    set $icsr = *(uint32_t*)0xE000ED04
    printf "ICSR: 0x%08x (active=%d, pending=%d)\n", $icsr, $icsr & 0x1FF, ($icsr >> 12) & 0x1FF
    printf "r0:  0x%08x  r1:  0x%08x  r2:  0x%08x  r3:  0x%08x\n", $r0, $r1, $r2, $r3
    printf "r4:  0x%08x  r5:  0x%08x  r6:  0x%08x  r7:  0x%08x\n", $r4, $r5, $r6, $r7
    printf "r8:  0x%08x  r9:  0x%08x  r10: 0x%08x  r11: 0x%08x\n", $r8, $r9, $r10, $r11
    printf "r12: 0x%08x\n", $r12
end

# Use it
(gdb) dump_context
```

---

## Phase 10: Common Pitfalls Checklist

| Issue | How to Detect | GDB Check |
|-------|--------------|-----------|
| PendSV preempts SVCall | VECTACTIVE=11 when PendSV runs | `break PendSV` + check ICSR |
| Wrong thread's frame | frame.thread_id mismatch | Add thread_id to frame |
| MSP changed unexpectedly | MSP differs at save vs restore | Compare at handle_svc vs svc_return |
| EXC_RETURN in user LR | LR = 0xFFFFFFF? in user mode | Check hardware exception frame |
| Stack overflow | Stack painting consumed | Check 0xCDCDCDCD pattern |
| active_thread race | Points to wrong thread | Check at PendSV entry |
| PRIMASK stuck at 1 | Interrupts never re-enable | Check $primask after lock drops |

---

## Phase 11: object_wait Hang Investigation

### Problem Summary
The `object_wait` syscall hangs after ~39/40 calls with PRIMASK=1 (interrupts disabled).
**Key insight**: Handle 0 is invalid, so syscall should return `Error::OutOfRange` immediately without blocking.

### Quick Diagnosis Commands

```gdb
# Count syscalls to find the hanging one
set $syscall_count = 0

break SVCall
commands
    silent
    set $syscall_count = $syscall_count + 1
    printf "SVCall #%d\n", $syscall_count
    continue
end

break svc_return  
commands
    silent
    printf "svc_return #%d\n", $syscall_count
    continue
end
```

### When System Hangs (Ctrl+C)

```gdb
define diagnose_hang
    printf "=== HANG DIAGNOSTICS ===\n"
    printf "Syscall count: %d\n", $syscall_count
    printf "PC: 0x%08x  LR: 0x%08x\n", $pc, $lr
    printf "MSP: 0x%08x  PSP: 0x%08x\n", $msp, $psp
    printf "CONTROL: 0x%08x  PRIMASK: 0x%08x\n", $control, $primask
    printf "xPSR: 0x%08x (exception#=%d)\n", $xpsr, $xpsr & 0x1ff
    
    if $primask == 1
        printf "!! INTERRUPTS DISABLED !!\n"
    end
    
    if ($xpsr & 0x1ff) != 0
        printf "In Handler Mode (exception %d)\n", $xpsr & 0x1ff
    else
        printf "In Thread Mode\n"
    end
    
    bt
end
```

### Check InterruptGuard (SpinLock)

```gdb
# The spinlock saves PRIMASK on entry, restores on exit
# If nested locks corrupt saved_primask, interrupts stay disabled

break InterruptGuard::new
commands
    silent
    printf "SpinLock acquire: PRIMASK was %d\n", $primask
    continue
end

break InterruptGuard::drop  
commands
    silent
    # saved_primask is at offset 0 in InterruptGuard struct
    printf "SpinLock release: saved=%d, will %s interrupts\n", \
        *(uint32_t*)$r0, (*(uint32_t*)$r0 & 1) ? "keep disabled" : "re-enable"
    continue
end
```

### Check PendSV Scheduling

```gdb
define check_pendsv
    set $icsr = *(uint32_t*)0xE000ED04
    printf "ICSR: 0x%08x\n", $icsr
    if ($icsr & (1 << 28)) != 0
        printf "  PendSV is PENDING\n"
    else
        printf "  PendSV is NOT pending\n"
    end
    if ($icsr & (1 << 27)) != 0
        printf "  PendSV is ACTIVE\n"
    end
end
```

### Hypothesis: Where Does PRIMASK Get Stuck?

1. **SVCall entry**: `cpsid i` (expected)
2. **Fake frame push**: Still disabled (expected)  
3. **Return to handle_svc**: `cpsie i` should happen
4. **SpinLock acquisition**: `cpsid i` again
5. **SpinLock release**: `cpsie i` if saved_primask was 0
6. **svc_return**: Should return with interrupts enabled

```gdb
# Track PRIMASK through syscall
break SVCall
commands
    printf "SVCall: PRIMASK=%d\n", $primask
    continue
end

break handle_svc
commands
    printf "handle_svc: PRIMASK=%d\n", $primask
    continue  
end

break svc_return
commands
    printf "svc_return: PRIMASK=%d\n", $primask
    continue
end
```

---

## Files to Examine

- `pw_kernel/arch/arm_cortex_m/syscall.rs` - SVCall handler, svc_return
- `pw_kernel/arch/arm_cortex_m/threads.rs` - PendSV, context_switch
- `pw_kernel/arch/arm_cortex_m/spinlock.rs` - InterruptGuard (PRIMASK handling)
- `pw_kernel/arch/arm_cortex_m/exceptions.rs` - KernelExceptionFrame definition
- `pw_kernel/kernel/scheduler.rs` - Thread scheduling logic
- `pw_kernel/kernel/sync/spinlock.rs` - SpinLock with PreemptDisableGuard

---

## Next Steps

1. Run GDB session with syscall counting breakpoints
2. Let it run until hang (~syscall #40)
3. Ctrl+C and run `diagnose_hang`
4. Check PRIMASK - if 1, trace back where it got stuck
5. Check if PendSV pending but not firing (interrupts disabled)
6. Compare syscall #39 (works) vs #40 (hangs) - what's different?
