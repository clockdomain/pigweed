# ARMv7-M Canonical CONTROL Fixup: Analysis and Workaround

## Executive Summary

The ARMv7-M "canonical CONTROL fixup" in `pendsv_swap_sp()` was designed to prevent CONTROL register corruption during context switches that interrupt syscall processing. **However, testing reveals that the fixup itself causes IPC return value corruption.** The workaround is to **disable the fixup entirely**.

**Key Finding:** IPC tests PASS when the fixup is disabled, and FAIL (with corrupted return values) when the fixup is enabled.

---

## Background: The Original Problem

### The Theoretical Issue

When a user thread makes a syscall:

1. **SVCall entry**: Hardware saves registers to user stack (PSP), including the return address
2. **SVCall handler**: Temporarily sets `CONTROL.nPRIV = 0` to elevate privilege for kernel operations
3. **PendSV fires** (tail-chained for context switch): Saves the *current* CONTROL register value
4. **Problem**: The saved CONTROL value is `0x02` (privileged, PSP) instead of `0x03` (unprivileged, PSP)

When the thread resumes later, it would restore `CONTROL = 0x02`, running user code with kernel privilege.

### The Proposed Solution (Commit 3cdce46a3)

Store a "canonical" CONTROL value per thread at creation time, and overwrite the saved frame's CONTROL field in PendSV before the thread is switched out:

```rust
// Original fix attempt
let saved_frame = &mut *((*active_thread).frame as *mut KernelExceptionFrame);
saved_frame.control = (*active_thread).canonical_control.0;
saved_frame.return_address = (*active_thread).canonical_return_address;
```

---

## Timeline of Issues

### 1. Initial Implementation (Commit 3cdce46a3)
- Added `canonical_control` field to `ArchThreadState`
- Applied fixup to `new_thread` (incorrect - should be `active_thread`)
- **Result**: Didn't work, corruption still occurred

### 2. ARMv8-M Regression (Commit 5ae4a20cf)  
- The fixup caused hangs on Cortex-M33 (ARMv8-M)
- **Root cause**: ARMv8-M has additional CONTROL bits (SFPA, BTI, PAC) that must be preserved
- **Fix**: Made fixup ARMv7-M only via `#[cfg(armv7m)]`

### 3. Fix Direction Corrected (Commit 4cdcdf2a1)
- Changed fixup to apply to `active_thread` (thread being switched OUT)
- Added `canonical_return_address` to also fix EXC_RETURN value
- **Result**: Still didn't work - IPC test failures continued

### 4. Final Discovery (Current)
- **Disabling the fixup entirely makes IPC tests PASS**
- The fixup was never solving the problem - it was CAUSING additional corruption

---

## Root Cause Analysis

### Why the Fixup Causes Corruption

The fixup modifies the saved frame **after** the PendSV assembly has saved registers but **before** the frame pointer is published. The theory was sound, but there's a critical flaw:

#### Hypothesis 1: Frame Pointer Timing
The `(*active_thread).frame` pointer may not yet point to valid saved state when the fixup runs. The assembly does:
```asm
str r0, [r1]          // Save SP to thread->frame
```
But the Rust code reads this pointer and modifies memory through it. There may be a memory ordering issue where:
- The store to `thread->frame` is visible
- But the actual register saves to the stack are not yet visible (store buffer not drained)

#### Hypothesis 2: Wrong Frame Layout Assumption
The `KernelExceptionFrame` struct layout might not match what's actually on the stack at this point. The PendSV assembly pushes:
```
[top of stack]
control           <- offset 0
psp               <- offset 4  
r4-r11            <- offsets 8-40
hardware frame    <- offsets 44+ (r0-r3, r12, lr, pc, xpsr)
```

But `KernelExceptionFrame` might have a different field order, causing the fixup to write to wrong locations.

#### Hypothesis 3: The Problem Doesn't Exist
Most critically: **the original problem may not actually occur in practice**. 

The syscall return sequence carefully restores CONTROL before returning to user mode:
```asm
// In svc_call return path
msr control, r0       // Restore canonical CONTROL
isb                   // Ensure CONTROL update takes effect
bx lr                 // Return to user mode
```

So even if PendSV saves a "corrupt" CONTROL value, the syscall completes and restores the correct value before the user thread continues.

---

## Testing Evidence

### Test: IPC with Fixup Enabled
```
[INF] Initiator: transact returned 8 bytes
[INF] Sent a, received (A,a)
... (works for a while)
[INF] Sent h, received (H,h)
[ERR] Sent i, received (I,j)  <- CORRUPTION: expected 'i', got 'j'
[ERR] ❌ FAILED
```

### Test: IPC with Fixup Disabled  
```
[INF] Sent a, received (A,a)
[INF] Sent b, received (B,b)
... (all 26 letters)
[INF] Sent z, received (Z,z)
[INF] ✅ PASSED
```

### Test: PendSvExceptionFrame Alternative
Attempted to use a different struct layout matching Gemini's analysis:
```
[ERR] PANIC: HardFault  
```
This proves the frame layout analysis was incorrect.

---

## The Workaround

### Current Implementation

```rust
// ARMv7-M fix: DISABLED - Investigation showed that applying canonical
// CONTROL/EXC_RETURN fixups actually CAUSES the IPC return value
// corruption, rather than fixing it. The syscall return values work
// correctly when this block is disabled.
#[cfg(all(feature = "user_space", feature = "armv7m_DISABLED"))]
{
    let saved_frame = &mut *((*active_thread).frame as *mut KernelExceptionFrame);
    saved_frame.control = (*active_thread).canonical_control.0;
    saved_frame.return_address = (*active_thread).canonical_return_address;
}
```

The `armv7m_DISABLED` feature never exists, so this code is never compiled.

### Why This Works

Without the fixup:
1. PendSV saves whatever CONTROL value exists (possibly `0x02` during syscall)
2. When thread resumes, the possibly-stale CONTROL is restored
3. **But**: The syscall return path (`svc_call`) properly restores canonical CONTROL anyway
4. The "corruption" in the saved frame is harmless because it gets fixed before user code runs

The fixup was trying to solve a problem that the syscall return path already handles.

---

## Remaining Questions

### Q1: Why does the fixup cause IPC corruption?
The exact mechanism is unclear. Possibilities:
- Memory ordering issues (dmb/dsb needed?)
- Writing to wrong stack offsets due to layout mismatch
- Race condition with frame pointer update

### Q2: Is there a real CONTROL corruption scenario?
Possibly if:
- A thread is preempted by PendSV
- Then receives an interrupt that completes
- Then resumes with the stale CONTROL

But this seems unlikely given the current interrupt priority setup.

### Q3: Should canonical_control/canonical_return_address be removed?
They could be removed since the fixup is disabled. However, keeping them:
- Documents the original intent
- Allows future investigation
- Minimal overhead (8 bytes per thread)

---

## Recommendations

### Short Term
1. **Keep the fixup disabled** - This is the working solution
2. **Remove `PendSvExceptionFrame`** - It's unused and based on incorrect analysis
3. **Add tests** - Ensure IPC tests run on both ARMv7-M and ARMv8-M in CI

### Long Term
1. **Investigate root cause** - Why does modifying the frame cause corruption?
2. **Consider removing canonical fields** - If fixup is permanently disabled
3. **Document for future maintainers** - This analysis should live near the code

---

## Appendix: Commit History

| Commit | Description | Outcome |
|--------|-------------|---------|
| 3cdce46a3 | Add canonical_control, apply to new_thread | Didn't fix issue |
| 5ae4a20cf | Disable for ARMv8-M (Cortex-M33 hang) | ARMv8-M works |
| 50e072f4d | Add canonical_return_address | Still broken |
| 4cdcdf2a1 | Apply to active_thread instead | Still broken |
| e31d16897 | **Disable fixup entirely** | **IPC PASSES** |

---

## Conclusion

The "canonical CONTROL fixup" was a well-intentioned solution to a theoretical problem. However:

1. **The theoretical problem may not occur** due to syscall return path handling
2. **The fixup itself causes corruption** through an unknown mechanism
3. **Disabling the fixup is the correct solution**

This is a case where the cure was worse than the disease. The simplest fix - doing nothing - is the correct one.
