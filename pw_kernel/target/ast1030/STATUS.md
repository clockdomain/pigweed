# AST1030 User Mode Enablement Status

**Date:** 2026-01-19  
**Branch:** `add-ast1030`  
**Target:** AST1030 (ASPEED Cortex-M4, ARMv7-M)

---

## Summary

| Area | Status | Notes |
|------|--------|-------|
| User mode transition | ✅ Working | Exception return to unprivileged mode confirmed |
| Context switching | ✅ Working | PendSV switches between kernel/user threads |
| MPU configuration | ✅ Working | PMSAv7 regions set up correctly |
| Syscalls (SVC) | ⚠️ Untested | Fix deployed, needs verification |
| IPC test | ⚠️ Blocked | Waiting on syscall verification |

---

## Completed Work

### 1. User Mode Execution (✅ Verified)

GDB debugging confirmed that user mode transitions work correctly:
- `CONTROL = 0x3` (nPRIV=1, SPSEL=1) — unprivileged, using PSP
- `EXC_RETURN = 0xFFFFFFFD` — thread mode, PSP
- User entry points reached (`0x20000` initiator, `0x40000` handler)

See [DEBUG_SESSION_RESULTS.md](ipc/user/DEBUG_SESSION_RESULTS.md) for detailed findings.

### 2. Root Cause Analysis (✅ Complete)

**Problem:** IPC test times out — user code calls tokenizer in kernel space instead of making syscalls.

**Root Cause:** 
1. User apps used `pw_log_backend_printf` (wrong backend)
2. Kernel code is intentionally mapped into user space for `svc_return` trampoline
3. User code could execute kernel functions without faulting

See [tokenizer_investigation_plan.md](ipc/user/tokenizer_investigation_plan.md) for full analysis.

### 3. Fix Implemented (✅ Committed)

**Fix:** Force user apps to use `log_backend_basic` (syscall-based) in `system_image.bzl`:

```python
str(Label("//pw_log/rust:pw_log_backend")): str(Label("//pw_kernel:log_backend_basic")),
```

**Branch:** `log-backend-fix` (commit `7700e5e51`)

This affects all user apps built via `system_image` rule.

---

## Remaining Work

### Phase 5: Verification (TODO)

1. **Rebuild with fix:**
   ```bash
   git checkout log-backend-fix
   bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc --config=k_qemu_ast1030
   ```

2. **Verify no tokenizer in user space:**
   ```bash
   arm-none-eabi-nm bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -i tokenize
   # Should show NO tokenizer symbols, or only in kernel range (< 0x20000)
   ```

3. **Run IPC test:**
   ```bash
   bazelisk test //pw_kernel/target/ast1030/ipc/user:ipc_test --config=k_qemu_ast1030
   ```

4. **Debug verification (if needed):**
   ```bash
   ./pw_kernel/target/ast1030/ipc/user/debug_session.sh
   # In GDB: break SVCall, continue — should hit syscall handler
   ```

---

## Known Issues

### Kernel Code Mapped to User Space

**Issue:** Entire kernel flash is mapped `ReadOnlyExecutable` into user processes.

**Why:** Required for `svc_return` trampoline to execute during privilege drop.

**Tracking:** [pwbug.dev/465500606](https://pwbug.dev/465500606)

**Proper Fix:** Isolate `svc_return` into its own linker section and map only that.

**Current Mitigation:** Use syscall-based logging to avoid user code calling kernel functions.

---

## Debug Resources

| File | Purpose |
|------|---------|
| [debug_usermode.gdb](ipc/user/debug_usermode.gdb) | GDB helper commands |
| [debug_session.sh](ipc/user/debug_session.sh) | tmux QEMU+GDB launcher |
| [DEBUG_SESSION_RESULTS.md](ipc/user/DEBUG_SESSION_RESULTS.md) | User mode verification results |
| [armv7m_usermode_debug_plan.md](ipc/user/armv7m_usermode_debug_plan.md) | ARMv7-M architecture background |
| [svc_debug_plan.md](ipc/user/svc_debug_plan.md) | Syscall debugging plan |
| [tokenizer_investigation_plan.md](ipc/user/tokenizer_investigation_plan.md) | Root cause analysis |

---

## Next Steps

1. Merge `log-backend-fix` into `add-ast1030`
2. Run IPC test to verify fix
3. If passing, submit for review
4. File follow-up for proper `svc_return` isolation (pwbug.dev/465500606)
