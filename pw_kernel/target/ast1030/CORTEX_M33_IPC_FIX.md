# Cortex-M33 IPC Test Fix

## Issue

The IPC test on Cortex-M33 (MPS2-AN505) hangs after printing the thread list.

## Root Cause

Commit `3cdce46a3` ("Fix: Store canonical CONTROL value to prevent corruption during syscall") introduced code that overwrites `frame.control` with `canonical_control` in `pendsv_swap_sp()`.

This causes the test to hang on **ARMv8-M (Cortex-M33)** but works fine on **ARMv7-M (Cortex-M4/AST1030)**.

### Breaking Code

```rust
// In pw_kernel/arch/arm_cortex_m/threads.rs, pendsv_swap_sp()
#[cfg(feature = "user_space")]
unsafe {
    (*(*new_thread).frame).control = (*new_thread).canonical_control;
}
```

## Why It Fails on Cortex-M33

The CONTROL register differs between ARMv7-M and ARMv8-M:

| Bit | ARMv7-M (Cortex-M4) | ARMv8-M (Cortex-M33) |
|-----|---------------------|----------------------|
| 0 | nPRIV | nPRIV |
| 1 | SPSEL | SPSEL |
| 2 | FPCA | FPCA |
| 3 | Reserved (RAZ/WI) | **SFPA** (Secure FP Active) |
| 4-7 | Reserved | **BTI_EN, UBTI_EN, PAC_EN, UPAC_EN** |

On ARMv7-M, overwriting CONTROL with a value that has bits 3-7 as zero is harmless (RAZ/WI).

On ARMv8-M, this may corrupt TrustZone or security-related state, causing the hang.

## Fix

**Disable the canonical_control overwrite in PendSV:**

```rust
// NOTE: The canonical_control overwrite has been disabled because it causes
// hangs on ARMv8-M (Cortex-M33). The original intent was to restore the
// canonical CONTROL value in case PendSV fired during syscall processing
// when privilege was temporarily elevated. However, this approach doesn't
// work correctly on ARMv8-M, possibly due to differences in CONTROL register
// handling or additional bits (SFPA, BTI, PAC) that shouldn't be touched.
//
// TODO: Investigate proper ARMv8-M CONTROL register handling during context switch.
// #[cfg(feature = "user_space")]
// unsafe {
//     (*(*new_thread).frame).control = (*new_thread).canonical_control;
// }
```

## Verification

```bash
# Bisect identified the breaking commit
git bisect start
git bisect bad HEAD
git bisect good f110d349556dad5
# Result: 3cdce46a3 is the first bad commit

# Test passes 3x after fix
./run_ipc_test_m33.sh
# === All 3 runs completed successfully ===
```

## Files Changed

- `pw_kernel/arch/arm_cortex_m/threads.rs` - Comment out the canonical_control overwrite

## Future Work

The original commit was trying to fix CONTROL register corruption during syscall processing. A proper fix for ARMv8-M would need to:

1. Only modify bits 0-1 (nPRIV, SPSEL) and preserve other bits
2. Or handle ARMv7-M and ARMv8-M differently with conditional compilation
3. Or find an alternative approach that doesn't touch CONTROL in PendSV

## Date

2026-01-22
