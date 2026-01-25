# AST1030 object_wait / Syscall Fault Analysis

## Problem Summary

~~The `object_wait` syscall hangs on AST1030~~ **UPDATE**: GDB debugging revealed this is a **MemManage fault (MUNSTKERR)**, not a hang. The system crashes after ~34 syscalls with a corrupted kernel exception frame.

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

### Files to Check
- `pw_kernel/arch/arm_cortex_m/syscall.rs` - SVCall handler, svc_return call site
- Look for where `r0` is set before jumping to `svc_return`

## Previous Analysis (For Reference)

### Fault Details
| Field | Value | Meaning |
|-------|-------|---------|
| **MMFSR** | `0x08` | **MUNSTKERR** - Unstacking error during exception return |
| **PSP** | `0x00087e68` | **INVALID** - Below user stack base (0x88000) |
| **LR** | `0xfffffffd` | Correct EXC_RETURN for Thread/PSP |
| **MSP** | `0x00060c40` | Kernel stack pointer |
| **Syscalls** | 34 | Crashes on 34th syscall |

### Corrupted Kernel Frame at MSP (0x60c40)
```
Actual values (from GDB):
  r4:  0x00060c64   ← Kernel pointer (WRONG - should be user register)
  r5:  0x00087f98   ← Stack address
  ...
  psp:     0x000004a5   ← This is svc_return+1 code address!
  control: 0x00087fc8   ← Stack address (should be 0/1/2/3)
  return:  0x00000010   ← Invalid EXC_RETURN (should be 0xFFFFFFxx)
```

## Reproduction

### Minimal Test Case
Adding a single `object_wait` call to the context_switch alpha test triggers the fault:

```rust
// In pw_kernel/tests/context_switch/alpha.rs
use userspace::syscall::Signals;
use userspace::time::Instant;

// Inside the iteration loop:
let _ = syscall::object_wait(0, Signals::READABLE, Instant::from_ticks(0));
```

### Test Results
- **Without object_wait**: `hello_user_test` PASSES ✅
- **With object_wait**: MemManage fault after ~34 syscalls ❌

### GDB Trace Output
```
[SVCall #1] Entry LR=0xfffffffd PRIMASK=0
[SVCall #2] Entry LR=0xfffffffd PRIMASK=0
...
[SVCall #34] Entry LR=0xfffffffd PRIMASK=0
MemoryManagement FAULT!
  MMFSR: 0x08 (MUNSTKERR - Unstacking error)
  PSP: 0x00087e68 (below valid user stack 0x88000!)
```

## Key Discovery: Logging Disabled During Hang

The semihosting console backend (`pw_kernel/subsys/console/console_backend_semihosting.rs`) silently drops log messages when interrupts are disabled:

```rust
// Lines 47-52
if interrupts_disabled() {
    return Ok(());  // Silently skip logging
}
```

This means:
1. **The hang occurs with PRIMASK=1** (interrupts disabled)
2. **Logging cannot be used to debug** the exact hang location
3. The system is stuck in a critical section

## Call Path Analysis

### User → Kernel Path
```
User: syscall::object_wait(handle, signals, deadline)
  → SVC exception
  → SVCall handler (cpsid i at entry)
  → handle_svc()
  → handle_object_wait()
  → lookup_handle() - acquires scheduler lock
  → object.object_wait()
  → ObjectBase::wait_until()
```

### wait_until Path (pw_kernel/kernel/object.rs)
```rust
pub fn wait_until(&self, kernel, signal_mask, deadline) -> Result<Signals> {
    let mut state = self.state.lock(kernel);  // SpinLock - disables interrupts
    
    if state.active_signals.contains(signal_mask) {
        return Ok(state.active_signals);  // Early return - no blocking
    }
    
    // Create event and waiter
    let event = Event::new(kernel, EventConfig::ManualReset);
    let waiter = ObjectWaiter { ... };
    state.waiters.push_back(waiter);
    
    drop(state);  // Release spinlock - should re-enable interrupts
    
    event.wait_until(deadline);  // <-- Potential hang location
    
    // Re-acquire lock to clean up
    let mut state = self.state.lock(kernel);
    ...
}
```

### Event::wait_until Path (pw_kernel/kernel/sync/event.rs)
```rust
pub fn wait_until(&self, deadline) -> Result<()> {
    let mut state = self.state.lock();  // WaitQueueLock - may disable interrupts
    if !state.signaled {
        let (_state, result) = state.wait_until(WaitType::Interruptible, deadline);
        return result;
    }
    Ok(())
}
```

### Scheduler wait_until Path (pw_kernel/kernel/scheduler.rs)
```rust
pub fn wait_until(mut self, wait_type, deadline) -> (Self, Result<()>) {
    let mut thread = self.sched_mut().take_current_thread();
    // ... set up timeout callback ...
    // ... add thread to wait queue ...
    self.schedule();  // <-- Context switch happens here
    // ...
}
```

## svc_return Disassembly Analysis

From GDB session:
```asm
svc_return:
   0x4a4:  mov   sp, r0                    # SP = frame pointer (r0)
   0x4a6:  ldmia sp!, {r4-r11}             # Pop r4-r11 (8 words, offset 0-31)
   0x4aa:  ldmia sp!, {r0, r1, lr}         # Pop psp→r0, control→r1, exc_return→lr
   0x4ae:  msr   PSP, r0                   # Set PSP from r0
   0x4b2:  orr   r1, r1, #3                # Force CONTROL bits (nPRIV=1, SPSEL=1)
   0x4b6:  msr   CONTROL, r1               # Set CONTROL
   0x4ba:  dsb   sy
   0x4be:  isb   sy
   0x4c2:  ldmia sp!, {r0-r3, r12, lr}     # Pop HW exception frame regs
   0x4c6:  pop   {r1}                      # Pop xPSR → r1
   0x4c8:  pop   {r0}                      # Pop original xPSR
   0x4ca:  ands  r0, r0, #512              # Check bit 9 (alignment)
   0x4ce:  it    ne
   0x4d0:  addne sp, #4                    # Adjust SP if was aligned
   0x4d2:  mov   r0, #0                    # Clear r0 (syscall return value)
   0x4d6:  orr   r1, r1, #1                # Set Thumb bit in return address
   0x4da:  bx    r1                        # Return to user code
```

### Frame Layout Expected by svc_return
| Offset | Field | Size |
|--------|-------|------|
| 0x00 | r4 | 4 |
| 0x04 | r5 | 4 |
| 0x08 | r6 | 4 |
| 0x0C | r7 | 4 |
| 0x10 | r8 | 4 |
| 0x14 | r9 | 4 |
| 0x18 | r10 | 4 |
| 0x1C | r11 | 4 |
| 0x20 | psp | 4 |
| 0x24 | control | 4 |
| 0x28 | exc_return | 4 |
| 0x2C+ | HW exception frame (r0-r3, r12, lr, pc, xpsr) | 32 |

## Key Finding: Invalid Handle Should Return Immediately

**Critical insight**: The test calls `object_wait(0, ...)` with handle 0, but the alpha process has an **empty object table** (`objects: []` in system.json5). This means:

1. `lookup_handle(kernel, 0)` should fail with `Error::OutOfRange`
2. `handle_object_wait()` should return the error immediately
3. **No blocking should occur** - the syscall should complete quickly

Yet the system hangs. This means the hang occurs **inside the syscall handling machinery**, not in the wait logic itself.

## Suspect Areas

### 1. SVCall Handler Entry/Exit (MOST LIKELY)
Location: `pw_kernel/arch/arm_cortex_m/syscall.rs`

The SVCall handler does complex stack manipulation:
```rust
pub unsafe extern "C" fn SVCall() -> ! {
    // cpsid i - disable interrupts
    // Save kernel exception frame
    // Elevate privilege (clear nPRIV in CONTROL)
    // Push fake exception frame
    // cpsie i - re-enable interrupts
    // Return via EXC_RETURN to handle_svc
}
```

If the fake exception frame or EXC_RETURN value is incorrect, return to user mode may fail.

### 2. svc_return Trampoline
Location: `pw_kernel/arch/arm_cortex_m/syscall.rs`

The return path requires a manual privilege drop:
```rust
pub unsafe extern "C" fn svc_return() -> ! {
    // Restore exception frame
    // Drop to non-privileged mode
    // Return to user
}
```

If CONTROL or PSP is corrupted, this won't work.

### 3. Context Switch Interrupt State
Location: `pw_kernel/arch/arm_cortex_m/threads.rs`

The context_switch has a critical section:
```rust
if !in_interrupt_handler() {
    // Drop scheduler lock to re-enable interrupts
    drop(sched_state);
    // PendSV should fire here
    // ==== TEMPORAL ANOMALY ====
    sched_state = crate::Arch::get_scheduler().lock();
}
```

If PendSV doesn't fire or interrupts aren't properly re-enabled, the system hangs.

### 4. SpinLock Interrupt Handling
Location: `pw_kernel/arch/arm_cortex_m/spinlock.rs`

The `InterruptGuard` saves/restores PRIMASK:
```rust
impl InterruptGuard {
    fn new() -> Self {
        let saved_primask;
        asm!("mrs {}, PRIMASK; cpsid i", out(reg) saved_primask);
        Self { saved_primask }
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if (self.saved_primask & 0x1) == 0x0 {
            asm!("cpsie i");  // Only re-enable if was enabled before
        }
    }
}
```

If nested lock operations corrupt the saved PRIMASK, interrupts may stay disabled.

### 5. PendSV Handler
Location: `pw_kernel/arch/arm_cortex_m/threads.rs`

The `pendsv_swap_sp` function has ARMv7-M specific code:
```rust
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    let saved_frame = &mut *(*active_thread).frame;
    saved_frame.control = (*active_thread).canonical_control;
    saved_frame.return_address = (*active_thread).canonical_return_address;
}
```

This overwrites CONTROL/EXC_RETURN with "canonical" values. If these are wrong, return fails.

## Differences from Working Platforms

### ARMv8-M (MPS2-AN505) vs ARMv7-M (AST1030)

| Feature | ARMv8-M | ARMv7-M |
|---------|---------|---------|
| TrustZone | Yes | No |
| Stack Limit | Hardware | Software |
| Privilege Stack | Separate | Shared MSP |
| Exception Priorities | 8 bits | 3-8 bits impl-defined |
| EXC_RETURN bits | More bits for security | Fewer bits |

The AST1030 uses PMSAv7 (simpler MPU) while MPS2-AN505 uses PMSAv8.

### Code Differences
The kernel has ARMv7-M specific code paths guarded by `#[cfg(feature = "armv7m")]`:
- PendSV handler overwrites canonical CONTROL/EXC_RETURN on ARMv7-M only
- Different EXC_RETURN values for secure/non-secure states

## Debugging Strategy

Since logging doesn't work during the hang, use GDB:

### 1. Set Breakpoint at SVCall Entry
```gdb
break SVCall
commands
  silent
  printf "SVCall entry: LR=%08x CONTROL=%08x\n", $lr, $control
  continue
end
```

### 2. Set Breakpoint at svc_return
```gdb
break svc_return
commands
  silent
  printf "svc_return: r0=%08x (frame) PSP=%08x\n", $r0, $psp
  continue
end
```

### 3. Set Breakpoint at handle_svc
```gdb
break handle_svc
commands
  silent
  printf "handle_svc: syscall processing\n"
  continue
end
```

### 4. Catch the Hang
```gdb
# Run until hang, then Ctrl+C and inspect:
info registers
bt
x/10x $msp
x/10x $psp
p/x $control
p/x $lr
```

### 5. Check Thread State
```gdb
# Check ACTIVE_THREAD
p ACTIVE_THREAD
# Check THREAD_LOCAL_STATE
p THREAD_LOCAL_STATE
```

## Files to Investigate (Priority Order)

1. **`pw_kernel/arch/arm_cortex_m/syscall.rs`** - SVCall handler and svc_return
2. **`pw_kernel/arch/arm_cortex_m/threads.rs`** - context_switch and pendsv_swap_sp
3. **`pw_kernel/arch/arm_cortex_m/spinlock.rs`** - InterruptGuard PRIMASK handling
4. **`pw_kernel/kernel/sync/spinlock.rs`** - SpinLock wrapper with PreemptDisableGuard
5. **`pw_kernel/kernel/scheduler.rs`** - reschedule() function

## Next Steps (UPDATED 2026-01-24)

Based on GDB findings, the frame corruption is the root cause. Next steps:

1. **Track frame pointer through syscall #34**:
   ```gdb
   # Break at svc_return, check r0 (frame pointer)
   break svc_return
   commands
     printf "svc_return frame: 0x%08x\n", $r0
     # Dump frame contents
     x/12wx $r0
     continue
   end
   ```

2. **Watch for frame corruption**:
   ```gdb
   # Set watchpoint on the psp field that gets corrupted
   # Frame is at MSP during syscall, offset 0x20 is psp field
   watch *(uint32_t*)(0x60c40 + 0x20)
   ```

3. **Check kernel stack depth** - Is MSP running into saved frames?
   ```gdb
   # At each syscall entry, log MSP
   break SVCall
   commands
     printf "SVCall MSP: 0x%08x\n", $msp
     continue
   end
   ```

4. **Compare syscall #33 (works) vs #34 (fails)**:
   - Stop at syscall #33, dump full frame
   - Continue to #34, compare frame contents

## Current Hypothesis (UPDATED)

The MemManage fault is caused by **kernel stack exhaustion or frame pointer drift**:

1. Each syscall pushes a KernelExceptionFrame (~44+ bytes) onto MSP
2. After 34 syscalls, MSP has drifted into previously saved frame data
3. The frame at the current MSP position contains stale values from syscall #N-1
4. When svc_return tries to restore, it gets garbage (code addresses, stack pointers)
5. CPU faults trying to unstack from invalid PSP (0x87e68)

**The value `0x000004a5` (svc_return+1) in the psp field is the smoking gun** - this proves the frame was partially overwritten by a previous return sequence.

### Why does debug_nop work but object_wait fail?

Possibilities:
1. **Stack depth**: object_wait calls more nested functions, deeper stack
2. **Lock operations**: object_wait acquires locks, may use more stack
3. **Timing**: Different execution path exposes the bug sooner
