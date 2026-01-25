# AST1030 Hello User Mode Test Conclusion

**Date:** January 23, 2026  
**Target:** AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7)  
**Status:** FLAKY - Partial Fix Applied

## Test Results Summary

### 10-Run Flakiness Test

| Run | User Entry | 100 Syscalls | Final Result |
|-----|------------|--------------|--------------|
| 1   | ✅ OK      | ✅ Completed | ✅ PASSED    |
| 2   | ✅ OK      | ✅ Completed | ❌ HardFault (shutdown) |
| 3   | ❌ Fault   | -            | ❌ FAILED    |
| 4   | ✅ OK      | ✅ Completed | ❌ FAILED    |
| 5   | ❌ Fault   | -            | ❌ FAILED    |
| 6   | ❌ Fault   | -            | ❌ FAILED    |
| 7   | ✅ OK      | ❌ Fault     | ❌ FAILED    |
| 8   | ✅ OK      | ❌ Fault     | ❌ FAILED    |
| 9   | ✅ OK      | ❌ Fault     | ❌ FAILED    |
| 10  | ✅ OK      | ❌ Fault     | ❌ FAILED    |

**Pass Rate: 1/10 (10%)**

### Observations

1. **User mode entry sometimes succeeds** - The initial transition to unprivileged mode works ~70% of the time
2. **Syscalls sometimes complete** - When entry works, syscalls often complete but then crash during shutdown
3. **HardFault at two different stack locations:**
   - `0x041ffc` - Early crash (during user entry or early syscalls)
   - `0x041fac` - Late crash (after syscalls, during shutdown)

## Current Fix Applied

**Commit `a13385610`:** Added DSB+ISB barriers after CONTROL register modification

```rust
// In syscall.rs - svc_return
"msr control, r1",
"dsb",      // Added - ensure CONTROL write completes
"isb",      // Existing - flush pipeline
```

This fix **reduced** the failure rate but did **not eliminate** it.

---

## Theories

### Theory 1: Missing Barrier in Another Code Path

**Hypothesis:** The DSB+ISB fix was applied to `svc_return` but there may be other code paths that modify CONTROL without proper barriers.

**Locations to check:**
- `PendSV` handler - context switch between threads
- Thread initialization - when first setting up unprivileged mode
- Exception return paths - any `bx lr` with EXC_RETURN

**Evidence:** The crash happens at different points (entry, mid-syscall, shutdown), suggesting multiple vulnerable code paths.

### Theory 2: PSP/MSP Stack Pointer Corruption

**Hypothesis:** The Process Stack Pointer (PSP) or Main Stack Pointer (MSP) is being corrupted during context switches.

**Evidence:**
- Crash shows `psp 0x00000000` in some cases
- `control 0x00000000` indicates thread is still in privileged mode
- Different stack frame addresses (`0x041ffc` vs `0x041fac`) suggest stack corruption

**Investigation:** Add stack canaries or check PSP validity before exception return.

### Theory 3: MPU Region Race Condition

**Hypothesis:** The MPU regions are not fully configured before the user thread starts executing.

**Evidence:** 
- PMSAv7 requires careful sequencing of MPU register writes
- The `protection_v7.rs` code writes multiple MPU regions
- A DSB may be needed after ALL regions are configured, not just individual ones

**Investigation:** Add barrier after MPU configuration completes in `configure_memory_for_process()`.

### Theory 4: QEMU Timing Sensitivity

**Hypothesis:** QEMU's instruction timing differs from real hardware, exposing races that wouldn't occur on actual AST1030.

**Evidence:**
- MPS2-AN505 (also QEMU) passes reliably
- The test is non-deterministic, suggesting timing dependency
- ARM Cortex-M4 vs M33 have different pipeline behaviors

**Investigation:** 
- Run on real AST1030 hardware
- Add artificial delays after barriers
- Try QEMU with different `-icount` settings

### Theory 5: Interrupt During Critical Section

**Hypothesis:** An interrupt (SysTick, PendSV) fires during the privilege transition, corrupting state.

**Evidence:**
- `HFSR=0x40000000` indicates forced HardFault (escalated from another exception)
- The system has SysTick running for scheduling
- PRIMASK may not be set during all critical sections

**Investigation:** Disable interrupts during `svc_return` and re-enable after privilege transition completes.

### Theory 6: EXC_RETURN Value Corruption

**Hypothesis:** The EXC_RETURN value (`0xFFFFFFFD`) is being corrupted before the exception return.

**Evidence:**
- EXC_RETURN determines which stack (PSP/MSP) and mode (Thread/Handler) to use
- If corrupted, the CPU will return to wrong state
- The LR register shows `0xfffffffd` in crash dumps - needs verification if this is correct

**Investigation:** Verify LR value immediately before `bx lr` in exception handlers.

---

## Recommended Next Steps

### Immediate Actions

1. **Add DSB+ISB to ALL privilege transition points:**
   - Check `PendSV` handler in `threads.rs`
   - Check thread initialization code
   - Check any exception return that goes to unprivileged mode

2. **Add validation before exception return:**
   ```rust
   // Before bx lr with EXC_RETURN
   assert!(psp != 0, "PSP is null before return to user mode");
   assert!(control & 1 == 1, "CONTROL.nPRIV not set");
   ```

3. **Disable interrupts during svc_return:**
   ```asm
   cpsid i          // Disable interrupts
   msr control, r1
   dsb
   isb
   bx lr            // Return re-enables interrupts via EXC_RETURN
   ```

### Deeper Investigation

4. **Compare with MPS2-AN505 code paths:**
   - Why does ARMv8-M work reliably?
   - Is there ARMv7-M specific code that's buggy?

5. **Add tracing to identify exact failure point:**
   - Log PSP, MSP, CONTROL, LR before each transition
   - Use semihosting to output values

6. **Test on real hardware:**
   - If available, run on actual AST1030 EVB
   - May behave differently than QEMU

---

## Files Involved

| File | Purpose | Status |
|------|---------|--------|
| `pw_kernel/arch/arm_cortex_m/syscall.rs` | SVCall handler, svc_return | ✅ DSB+ISB added |
| `pw_kernel/macros/arm_cortex_m_macro.rs` | restore_exception_frame | ✅ DSB+ISB added |
| `pw_kernel/arch/arm_cortex_m/protection_v8.rs` | MPU config (v8) | ✅ DSB+ISB added |
| `pw_kernel/arch/arm_cortex_m/protection_v7.rs` | MPU config (v7) | ❓ Needs review |
| `pw_kernel/arch/arm_cortex_m/threads.rs` | Thread/context switch | ❓ Needs review |

---

## Build & Test Commands

```bash
# Build
bazelisk build //pw_kernel/target/ast1030/hello_user:hello_user --config=k_qemu_ast1030

# Test (single run)
bazelisk test //pw_kernel/target/ast1030/hello_user:hello_user_test \
    --config=k_qemu_ast1030 --test_output=streamed --nocache_test_results --test_timeout=10

# Test (10 runs for flakiness)
for i in {1..10}; do
    echo "=== Run $i ==="
    bazelisk test //pw_kernel/target/ast1030/hello_user:hello_user_test \
        --config=k_qemu_ast1030 --test_timeout=10 --nocache_test_results 2>&1 \
        | grep -E "PASSED|FAILED|completed|fault|exception" | head -5
done

# Direct QEMU run
qemu-system-arm -machine ast1030-evb \
    -kernel bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf \
    -nographic -semihosting
```
