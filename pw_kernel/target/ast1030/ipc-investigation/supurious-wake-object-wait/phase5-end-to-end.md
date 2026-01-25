# Phase 5: End-to-End Fix Validation

## Objective

Validate the complete fix and ensure IPC works reliably on AST1030.

---

## Pre-Validation Checklist

Before running end-to-end tests:

- [ ] Phase 1-4 findings documented
- [ ] Root cause identified
- [ ] Fix implemented
- [ ] Fix reviewed for security implications

---

## Test Matrix

### Test 5.1: Basic IPC Test

```bash
bazel test --config=k_qemu_ast1030 \
    //pw_kernel/target/ast1030/ipc/user:ipc_test
```

**Expected**: PASSED

### Test 5.2: Stress Test (Multiple Runs)

```bash
for i in $(seq 1 20); do
    bazel test --config=k_qemu_ast1030 --cache_test_results=no \
        //pw_kernel/target/ast1030/ipc/user:ipc_test 2>&1 | grep -E "PASSED|FAILED|TIMEOUT"
done
```

**Expected**: 20/20 PASSED

### Test 5.3: Context Switch Test Still Works

```bash
for i in $(seq 1 20); do
    bazel test --config=k_qemu_ast1030 --cache_test_results=no \
        //pw_kernel/target/ast1030/hello_user:hello_user_test 2>&1 | grep -E "PASSED|FAILED|TIMEOUT"
done
```

**Expected**: 20/20 PASSED (no regression)

### Test 5.4: Cross-Platform Validation

Ensure fix doesn't break other platforms:

```bash
# LM3S6965 (ARMv7-M)
bazel test --config=k_qemu_lm3s6965 //pw_kernel/target/lm3s6965/ipc/user:ipc_test

# MPS2-AN505 (ARMv8-M)
bazel test --config=k_qemu_mps2_an505 //pw_kernel/target/mps2_an505/ipc/user:ipc_test

# RISC-V
bazel test --config=k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/ipc/user:ipc_test
```

---

## Regression Tests

### Kernel Tests

```bash
bazel test --config=k_qemu_ast1030 //pw_kernel/...
```

### Specific Tests to Watch

| Test | Risk Area |
|------|-----------|
| `hello_user_test` | User-mode context switch |
| `ipc_test` | IPC functionality |
| `interrupt_test` | If exists, interrupt handling |

---

## Performance Validation

If the fix involves MPU reconfiguration:

1. Measure syscall latency before/after
2. Measure context switch time before/after
3. Document any performance impact

---

## Documentation Updates

After fix is validated:

1. [ ] Update `pw_kernel/docs/ipc-subsystem-architecture.md`
2. [ ] Add comments to MPU configuration code
3. [ ] Document any platform-specific considerations
4. [ ] Update this investigation folder with findings

---

## Commit Strategy

### Commit 1: Investigation Infrastructure
- Debug logging additions (if keeping)
- Test utilities

### Commit 2: The Fix
- MPU or buffer access fix
- Clear commit message explaining the issue

### Commit 3: Cleanup
- Remove debug logging
- Final documentation

---

## Success Criteria

- [ ] IPC test passes 20/20 runs
- [ ] Context switch test passes 20/20 runs
- [ ] No regressions on other platforms
- [ ] Fix is minimal and well-documented
- [ ] Security model preserved

---

## Final Summary Template

```markdown
## IPC Fix Summary

### Root Cause
[Description of what was wrong]

### Fix
[Description of the fix]

### Files Changed
- file1.rs: [what changed]
- file2.rs: [what changed]

### Testing
- AST1030 IPC: 20/20 PASSED
- AST1030 Context Switch: 20/20 PASSED
- LM3S6965 IPC: PASSED
- MPS2-AN505 IPC: PASSED

### Security Impact
[None / Description of any security considerations]
```
