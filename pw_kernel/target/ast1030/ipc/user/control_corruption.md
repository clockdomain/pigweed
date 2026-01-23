# CONTROL Register Corruption During Syscall Context Switch

**Date:** January 21, 2026  
**Status:** 🔴 ROOT CAUSE IDENTIFIED - Fix Pending  
**Affects:** AST1030 (ARMv7-M) and potentially all ARM Cortex-M targets with user-space support

## Executive Summary

When a context switch (PendSV) occurs during syscall processing, the PendSV handler reads the **live** CONTROL register value instead of the original user-space value. This causes user threads to resume with incorrect privilege/stack settings, eventually leading to a HardFault.

## Symptoms

### Test Output with Context Switch Tracing

After enabling `LOG_CONTEXT_SWITCH = true` in threads.rs:

```
[INF] Starting thread 'initiator thread' (0x00061080)
[INF] KernelFrame: psp=0x00087fe0 control=0x00000003  ← CORRECT (nPRIV=1, SPSEL=1)
...
[INF] KernelFrame: psp=0x00087e80 control=0x00000001  ← WRONG (SPSEL changed to 0!)
...
[INF] KernelFrame: psp=0x0008fea8 control=0x00000000  ← WRONG (fully privileged!)
[INF] KernelFrame: psp=0x00087f58 control=0x00000000  ← WRONG
...
[INF] HardFault: CFSR=0x00020000 UFSR=0x0002 (INVPC)
[INF] pc=0x00002298 psr=0x00000000  ← Thumb bit missing!
```

### Fault Analysis

| Field | Value | Meaning |
|-------|-------|---------|
| CFSR | `0x00020000` | UsageFault active |
| UFSR | `0x0002` | **INVPC** - Invalid PC load |
| PC | `0x00002298` | User space address |
| **PSR** | **`0x00000000`** | **⚠️ Thumb bit NOT set!** |

## CONTROL Register Values

| CONTROL | nPRIV | SPSEL | Meaning | Expected for User Thread? |
|---------|-------|-------|---------|---------------------------|
| `0x03` | 1 | 1 | Unprivileged, PSP | ✅ YES |
| `0x02` | 0 | 1 | Privileged, PSP | During syscall only |
| `0x01` | 1 | 0 | Unprivileged, **MSP** | ❌ NO - Invalid! |
| `0x00` | 0 | 0 | Privileged, MSP | Kernel threads only |

## Root Cause

### The Bug: PendSV Reads LIVE CONTROL During Syscall

When PendSV fires during syscall processing, it reads the **live** CONTROL register value instead of the original user-space value that was saved by the SVCall handler.

### Execution Flow Showing the Bug

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ Step │ Action                                    │ CONTROL │ Notes             │
├──────┼───────────────────────────────────────────┼─────────┼───────────────────┤
│  1   │ User thread A running                     │ 0x03    │ Correct           │
│  2   │ User thread makes syscall (SVC)           │ 0x03    │                   │
│  3   │ SVCall saves frame: mrs r2, control       │ 0x03    │ Saved to stack ✅ │
│  4   │ SVCall elevates: bfc r2,#0,#1; msr ctrl   │ 0x02    │ Now privileged    │
│  5   │ SVCall returns to thread mode (kernel)    │ 0x02    │ Modified value    │
│  6   │ Kernel code runs, triggers reschedule     │ 0x02    │                   │
│  7   │ PendSV fires                              │ 0x02    │                   │
│  8   │ PendSV saves: mrs r1, control             │ 0x02    │ ❌ WRONG VALUE!   │
│  9   │ PendSV switches to thread B               │ ---     │                   │
│ ...  │ Thread B runs...                          │         │                   │
│  N   │ PendSV switches back to thread A          │         │                   │
│ N+1  │ PendSV restores: msr control, r1          │ 0x02    │ ❌ Corrupted!     │
│ N+2  │ Thread A resumes in kernel with 0x02      │ 0x02    │                   │
│ N+3  │ svc_return fixes: orr r1,r1,0x3           │ 0x03    │ ✅ Fixed here     │
└─────────────────────────────────────────────────────────────────────────────────┘
```

**However**, if another context switch happens before `svc_return` completes, or if the stack is corrupted, the wrong CONTROL value persists and propagates.

## Source Code Analysis

### PendSV Wrapper (arm_cortex_m_macro.rs:183-188)

```rust
fn save_exception_frame(asm: &mut String, kernel_mode: &KernelMode) {
    if kernel_mode.save_psp_needed() {
        asm.push_str(
            "
            mrs     r1, control      // ❌ BUG: Reads LIVE control, not saved value!
            mrs     r0, psp
            push    {{ r0 - r1, lr }}
            push    {{ r4 - r11 }}
            ...
```

### SVCall Handler (syscall.rs:107-125)

```rust
pub unsafe extern "C" fn SVCall() -> ! {
    core::arch::naked_asm!(
        "
            cpsid i
            
            mrs     r2, control          // Read original CONTROL (0x03)
            mrs     r1, psp
            push    {{ r1 - r2, lr }}    // ✅ Saves correct value to kernel stack
            push    {{ r4 - r11 }}
            ...
            
            bfc     r2, #0, #1           // Clear nPRIV bit: 0x03 → 0x02
            msr     control, r2          // ❌ Modifies LIVE register!
```

### svc_return (syscall.rs:199-206)

```rust
pub unsafe extern "C" fn svc_return() -> ! {
    core::arch::naked_asm!(
        "
            ...
            pop     {{ r0 - r1, lr }}    // r1 = original CONTROL from SVCall frame
            msr     psp, r0
            orr     r1, r1, 0x3          // Ensure nPRIV and SPSEL are set
            msr     control, r1          // ✅ Restores correct value
```

## Why CONTROL=0x01 Appears

The value `0x01` (nPRIV=1, SPSEL=0) is particularly strange because:
- User threads should NEVER have SPSEL=0 (they use PSP, not MSP)
- This suggests multiple corruptions or bit mixing between threads

Possible explanation: When context switching between threads with different states, bits from multiple threads' CONTROL values may be getting mixed due to the corruption cascade.

## Proposed Fixes

### Option 1: Nested Exception Frame Awareness

Detect if we're in syscall context (check if SVCall frame exists on stack) and use the CONTROL from the SVCall frame instead of the live register.

**Pros:** Minimal changes, preserves existing architecture  
**Cons:** Complex detection logic, fragile

### Option 2: Thread-Local CONTROL Storage (Recommended)

Store the "canonical" CONTROL value in `ArchThreadState`. Always use this stored value instead of reading the live register.

```rust
pub struct ArchThreadState {
    pub frame: *mut KernelExceptionFrame,
    pub memory_config: *const MemoryConfig,
    pub local: ThreadLocalState,
    pub canonical_control: ControlVal,  // NEW: Always-correct CONTROL value
}
```

PendSV would save/restore this value instead of reading `mrs control`.

**Pros:** Clean, robust, explicit  
**Cons:** Requires struct change, slightly more memory per thread

### Option 3: Syscall-in-Progress Flag

Add a flag in thread state indicating syscall is in progress. PendSV checks the flag and handles CONTROL specially.

**Pros:** Minimal struct change  
**Cons:** Still requires special-case logic, potential for bugs

## Verification Steps

### GDB Verification

```gdb
# Break at PendSV entry to check CONTROL
b PendSV
commands
  silent
  printf "PendSV entry: CONTROL=0x%x\n", $control
  # Check if we're in syscall context
  bt
  continue
end

# Break at syscall entry
b SVCall
commands
  silent
  printf "SVCall entry: CONTROL=0x%x\n", $control
  continue
end

continue
```

### Expected vs Actual

| Event | Expected CONTROL | Actual CONTROL |
|-------|------------------|----------------|
| User thread running | 0x03 | 0x03 ✅ |
| SVCall entry | 0x03 | 0x03 ✅ |
| After SVCall elevates | 0x02 | 0x02 ✅ |
| PendSV during syscall (saved) | 0x03 | 0x02 ❌ |
| Thread resume | 0x03 | 0x02 ❌ |

## Related Documentation

- `pw_kernel/target/ast1030/ipc/user/control_register_fix.md` - Related EXC_RETURN bug (different issue)
- `pw_kernel/target/ast1030/ipc/user/tokenizer_investigation/svc_debug_plan.md` - Full debug session log

## References

1. ARMv7-M Architecture Reference Manual (DDI 0403E.e)
   - Section B1.4.4: CONTROL register
   - Section B1.5.8: Exception return behavior
2. `pw_kernel/macros/arm_cortex_m_macro.rs` - Exception handler generation
3. `pw_kernel/arch/arm_cortex_m/syscall.rs` - Syscall implementation
