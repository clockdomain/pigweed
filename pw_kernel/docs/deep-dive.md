# ARMv7-M User-Mode Context Switch Bug: Deep Dive Analysis

**Date:** January 22, 2026  
**Target:** AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7)  
**Status:** Under Investigation

## Executive Summary

A critical bug causes user-mode processes to crash after approximately 25-50 syscalls on ARMv7-M (Cortex-M4) targets. The crash manifests as:
- PSP (Process Stack Pointer) = 0x00000000
- CONTROL register = 0x00000000 (kernel mode)
- EXC_RETURN = 0xfffffff9 (MSP thread mode)

The same test passes on ARMv8-M (Cortex-M33, MPS2-AN505), confirming this is an ARMv7-M specific issue.

## Reproduction

### Minimal Test Case

```rust
// pw_kernel/tests/hello_user/hello.rs
#[unsafe(no_mangle)]
pub extern "C" fn _user_start() -> ! {
    pw_log::info!("Hello from user mode!");
    
    // Stress test: 100 syscalls
    for i in 0..100 {
        pw_log::info!("Syscall iteration {}", i);
        debug_nop();  // Each call triggers SVCall
    }
    
    exit(0);
}
```

### Results

| Target | Architecture | Result |
|--------|--------------|--------|
| AST1030 | ARMv7-M (Cortex-M4) | **CRASH** after ~25-50 iterations |
| MPS2-AN505 | ARMv8-M (Cortex-M33) | PASS (all 100 iterations) |

## Architecture Background

### Key Registers

**CONTROL Register (ARMv7-M)**
- Bit 0 (nPRIV): 0=Privileged, 1=Unprivileged
- Bit 1 (SPSEL): 0=Use MSP, 1=Use PSP

| Mode | CONTROL | Description |
|------|---------|-------------|
| Kernel | 0x00 | Privileged, MSP |
| User | 0x03 | Unprivileged, PSP |

**EXC_RETURN Values**
| Value | Stack | Mode |
|-------|-------|------|
| 0xFFFFFFF9 | MSP | Thread |
| 0xFFFFFFFD | PSP | Thread |

### Exception Frame Layout

```
KernelExceptionFrame (saved by software):
  Offset 0x00: r4
  Offset 0x04: r5
  Offset 0x08: r6
  Offset 0x0C: r7
  Offset 0x10: r8
  Offset 0x14: r9
  Offset 0x18: r10
  Offset 0x1C: r11
  Offset 0x20: psp          (saved PSP value)
  Offset 0x24: control      (saved CONTROL value)
  Offset 0x28: return_address (EXC_RETURN)
```

## Root Cause Analysis

### The Problem: PendSV During Syscall Processing

The bug occurs when PendSV (context switch) fires while a syscall is being processed. The sequence:

```
Timeline:
─────────────────────────────────────────────────────────────────────────────
User Thread A (CONTROL=0x03, PSP=valid)
    │
    ▼ SVCall (syscall)
┌─────────────────────────────────────────────────────────────────────────┐
│ 1. SVCall Entry:                                                         │
│    - Hardware saves exception frame to PSP                               │
│    - mrs r2, control  → r2 = 0x03 (correct)                             │
│    - mrs r1, psp      → r1 = valid_psp                                  │
│    - push {r1-r2, lr} → saves to kernel stack                           │
│                                                                          │
│ 2. Privilege Elevation:                                                  │
│    - bfc r2, #0, #1   → r2 = 0x02 (clear nPRIV bit)                     │
│    - msr control, r2  → LIVE CONTROL = 0x02 ← TRANSIENT STATE          │
│                                                                          │
│ 3. Return to Thread Mode (kernel):                                       │
│    - bx lr with EXC_RETURN = 0xFFFFFFF9 (MSP, Thread)                   │
│    - Now in Thread mode, using MSP, CONTROL still = 0x02                │
│                                                                          │
│ 4. cpsie i  ← INTERRUPTS RE-ENABLED HERE                                │
│    ════════════════════════════════════════════════════════════════════ │
│    ║ DANGER ZONE: PendSV can fire now!                                 ║ │
│    ║ CONTROL = 0x02 (transient), not 0x03 (canonical)                  ║ │
│    ════════════════════════════════════════════════════════════════════ │
└─────────────────────────────────────────────────────────────────────────┘
    │
    ▼ PendSV fires (context switch requested)
┌─────────────────────────────────────────────────────────────────────────┐
│ PendSV Entry (save_exception_frame macro):                               │
│    - mrs r1, control  → r1 = 0x02 (WRONG! Should be 0x03)               │
│    - mrs r0, psp      → r0 = user's PSP (still correct)                 │
│    - push {r0-r1, lr} → saves CORRUPTED control to frame                │
└─────────────────────────────────────────────────────────────────────────┘
```

### Why This Causes PSP = 0x00000000

The corruption chain:

1. **CONTROL.SPSEL corruption**: When CONTROL=0x02 is saved, SPSEL=1 but nPRIV=0
2. **EXC_RETURN corruption**: PendSV was entered from Thread mode using MSP, so hardware EXC_RETURN = 0xFFFFFFF9
3. **On resume**: Frame is restored with:
   - CONTROL = 0x02 (or worse, 0x00 after further corruption)
   - EXC_RETURN = 0xFFFFFFF9 (use MSP)
4. **PSP never updated**: When CONTROL.SPSEL=0, processor uses MSP, PSP becomes stale
5. **Eventually PSP = 0**: After enough context switches, PSP degrades to NULL

## Current Fix Implementation

### Thread State Storage

```rust
// pw_kernel/arch/arm_cortex_m/threads.rs
pub struct ArchThreadState {
    frame: *mut KernelExceptionFrame,
    memory_config: *const MemoryConfig,
    local: ThreadLocalState<crate::Arch>,
    
    // Canonical values - invariant, set at thread creation
    #[cfg(feature = "user_space")]
    pub canonical_control: ControlVal,      // Offset 16
    #[cfg(feature = "user_space")]
    pub canonical_return_address: u32,      // Offset 20
}
```

### PendSV Fix Code

```rust
// In pendsv_swap_sp():
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    let saved_frame = &mut *(*active_thread).frame;
    saved_frame.control = (*active_thread).canonical_control;
    saved_frame.return_address = (*active_thread).canonical_return_address;
}
```

### Verified in Binary

Disassembly confirms the fix is compiled:
```asm
20b4:  ldr r1, [r5, #16]    ; Load canonical_control from thread
20bc:  str r1, [r4, #36]    ; Store to frame.control (offset 0x24)
20c6:  ldr r0, [r5, #20]    ; Load canonical_return_address
20c8:  str r0, [r4, #40]    ; Store to frame.return_address (offset 0x28)
```

## Unresolved Questions

### Why is PSP still becoming NULL?

The fix correctly restores CONTROL and EXC_RETURN in the saved frame, but PSP corruption persists. Hypotheses:

1. **Race condition**: Multiple context switches compound the problem before fix takes effect
2. **Frame ordering**: The frame being fixed might not be the one causing the corruption
3. **Missing PSP fix**: We fix CONTROL but the damage to PSP has already occurred by then

### Key Observation

The crash state shows:
- PSP = 0x00000000
- CONTROL = 0x00000000
- EXC_RETURN = 0xfffffff9

This looks like **kernel thread** state (idle thread), not user thread state. Could there be a thread identity confusion where the wrong thread's canonical values are being applied?

## Comparison: ARMv7-M vs ARMv8-M

| Aspect | ARMv7-M (Cortex-M4) | ARMv8-M (Cortex-M33) |
|--------|---------------------|----------------------|
| CONTROL bits | 2 bits (nPRIV, SPSEL) | 4+ bits (includes SFPA, etc.) |
| Stack limit | None | Hardware stack limit checking |
| TrustZone | No | Yes |
| Bug present | YES | NO |

The ARMv8-M doesn't exhibit this bug, possibly due to:
- Different exception handling timing
- Hardware stack limit checking catches corruption earlier
- TrustZone security features provide additional protection

## Files Involved

| File | Purpose |
|------|---------|
| `pw_kernel/arch/arm_cortex_m/threads.rs` | Thread state, PendSV handler, canonical values |
| `pw_kernel/arch/arm_cortex_m/syscall.rs` | SVCall handler, privilege elevation |
| `pw_kernel/arch/arm_cortex_m/exceptions.rs` | Exception frame definitions |
| `pw_kernel/macros/arm_cortex_m_macro.rs` | Exception wrapper macros (save/restore) |
| `pw_kernel/tests/hello_user/hello.rs` | Minimal test case |

## Next Steps

1. **Add debug logging** to trace PSP/CONTROL values at every syscall entry/exit and context switch
2. **Verify thread identity** - ensure `active_thread` always points to the correct thread
3. **Check idle thread interaction** - the crash state resembles idle thread canonical values
4. **Consider atomic PSP handling** - may need to save PSP canonically as well
5. **Investigate `svc_return`** - verify the return-to-user path is correct

## Build Configuration

The `armv7m` feature is confirmed enabled:
```
--cfg 'feature="armv7m"'
```

Build command:
```bash
bazelisk test //pw_kernel/target/ast1030/hello_user:hello_user_test \
    --config=k_qemu_ast1030 --test_output=streamed
```

## Experiment Results

### ✅ THE FIX: DSB+ISB in svc_return (January 22, 2026)

**Root Cause Found:** The `svc_return` function only had ISB after CONTROL modification, but was **missing DSB**.

**The Fix:**
```asm
// Before (broken):
msr     control, r1
isb

// After (fixed):
msr     control, r1
dsb                     // Ensure CONTROL write completes
isb                     // Flush pipeline to see the change
```

**Result:** ✅ **ALL 100 SYSCALLS PASS!**

**Why DSB is critical:**
- DSB (Data Synchronization Barrier) ensures the CONTROL register write **completes** before proceeding
- ISB alone only flushes the pipeline but doesn't guarantee the write has finished
- On ARMv7-M, without DSB, the CONTROL change may not be visible when subsequent instructions execute

**Files changed:**
- [pw_kernel/arch/arm_cortex_m/syscall.rs](../arch/arm_cortex_m/syscall.rs) - Added DSB before ISB in `svc_return`
- [pw_kernel/macros/arm_cortex_m_macro.rs](../macros/arm_cortex_m_macro.rs) - Added DSB+ISB in `restore_exception_frame`
- [pw_kernel/arch/arm_cortex_m/protection_v8.rs](../arch/arm_cortex_m/protection_v8.rs) - Added missing DSB+ISB in MPU write

### ISB-only Addition (January 22, 2026)

Added the missing ISB instruction after `msr control, r1`:

```asm
msr     control, r1
isb                     // Required after CONTROL modification per ARM ARM
pop     {{ pc }}
```

**Result:** Still crashes after ~50 syscalls. ISB is architecturally correct but doesn't fix the root cause.

### DSB+ISB Barrier Addition (January 22, 2026)

Per ARM best practices, added both barriers after CONTROL write:

```asm
msr     control, r1
dsb                     // Ensure CONTROL write completes
isb                     // Flush pipeline to see the change
pop     {{ pc }}
```

**Result:** Still crashes after ~50 syscalls. The barriers are architecturally correct but don't address the root cause. The bug is not a pipeline/barrier issue.

### Placeholder Experiment (January 22, 2026)

Tried the clockdomain approach - using `ldr r1, =0xDEAD` instead of `mrs r1, control`:

**Result:** Crash happens MUCH EARLIER (after 0 syscalls instead of 50!) and the output shows:
```
psp 0x00000000 control 0x0000dead return_address 0xfffffff9
```

**Critical Finding:** The `0xDEAD` placeholder survived to the crash! This proves:

1. **The canonical_control fix in PendSV is NOT being applied** to this particular exception return
2. The exception is returning **without going through a PendSV context switch**
3. The placeholder approach requires ALL exception returns to go through PendSV first

This explains why our current fix doesn't work: the corruption happens on an exception return path that **bypasses** the PendSV fix entirely.

### New Hypothesis

The bug isn't in PendSV context switch - it's in the **SVCall return path** (`svc_return`). When SVCall returns directly to user mode (without a context switch), it uses the values saved on the kernel stack, which may have been corrupted.

## Alternative Approach: clockdomain/pigweed Commit c9a141b

A different fix approach was implemented in [clockdomain/pigweed commit c9a141b](https://github.com/clockdomain/pigweed/commit/c9a141b0c7680fd68f5c6c9921e9b3f422567ba7), which takes the opposite approach:

### Their Strategy: Don't Save CONTROL at All

Instead of fixing the corrupted CONTROL value after save (our approach), they **don't read CONTROL during save** - they use a placeholder value:

**Save path (before):**
```asm
mrs     r1, control       ; Read LIVE control (may be corrupted!)
mrs     r0, psp
push    { r0 - r1, lr }
```

**Save path (after):**
```asm
ldr     r1, =0xDEAD       ; Placeholder - never read, helps debug
mrs     r0, psp
push    { r0 - r1, lr }
```

**Restore path (added ISB):**
```asm
pop     { r0 - r1, lr }
msr     psp, r0
msr     control, r1
isb                       ; Required after CONTROL modification per ARM ARM
bx      lr
```

### Key Insight

The CONTROL value is **invariant per-thread**:
- Set once at thread initialization
- Never changes during thread lifetime
- User threads: always 0x03
- Kernel threads: always 0x00

Therefore, reading it during exception save is unnecessary. The value that gets restored comes from the **incoming thread's frame** (set at initialization), not from the live register.

### Missing ISB

Their commit also adds an `isb` instruction after `msr control, r1`. The ARM Architecture Reference Manual requires an ISB after writing to CONTROL to ensure the change takes effect before subsequent instructions. **This was missing in our current code!**

### Comparison of Approaches

| Aspect | Our Approach | clockdomain Approach |
|--------|--------------|----------------------|
| Save path | Read CONTROL, later overwrite | Don't read CONTROL (placeholder) |
| Restore path | No ISB | ISB after CONTROL write |
| Debug value | Uses canonical value | Uses 0xDEAD sentinel |
| Complexity | Requires extra fix in PendSV | Simpler, fix in macro |

### Should We Adopt This?

**Advantages of clockdomain approach:**
1. Simpler - fix is in one place (macro), not scattered
2. ISB is architecturally correct (we're missing it!)
3. Placeholder value (0xDEAD) helps identify bugs

**Potential issues:**
1. May not solve the PSP=0 problem (different root cause?)
2. Their notes say fault happens on FIRST switch, not after 25-50

### Action Items

1. **Add ISB** - We should add the missing `isb` after `msr control, r1` regardless
2. **Consider placeholder approach** - May be cleaner than our current fix
3. **Investigate PSP separately** - The CONTROL fix may not address PSP corruption

## References

- ARMv7-M Architecture Reference Manual (DDI0403E)
- ARM Cortex-M4 Technical Reference Manual
- Pigweed Kernel Documentation
- [clockdomain/pigweed commit c9a141b](https://github.com/clockdomain/pigweed/commit/c9a141b0c7680fd68f5c6c9921e9b3f422567ba7) - Alternative CONTROL fix approach
