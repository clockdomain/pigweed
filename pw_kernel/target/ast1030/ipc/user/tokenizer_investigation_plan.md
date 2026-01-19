# Tokenizer Linkage Investigation Plan

## Issue Summary

**Problem:** AST1030 IPC test times out because user code calls tokenizer functions in kernel space.

**Root Cause:** The `pw_tokenizer` library is linked into the kernel binary but NOT into user app binaries. When user code calls `pw_log::info!`, it jumps to kernel address 0x3ec6 instead of triggering an SVC syscall.

**Why it doesn't fault:** The kernel code region is **intentionally mapped into user space** with `ReadOnlyExecutable` permissions.

---

## Why Kernel Code Execution Doesn't Fault (2026-01-19)

### Investigation Summary

Initially suspected QEMU MPU emulation issue, but the actual cause is an **intentional design decision** in the system generator.

### Root Cause

In [pw_kernel/tooling/system_generator/lib.rs](../../../../tooling/system_generator/lib.rs) (lines 110-125 for ARMv8-M, lines 163-178 for ARMv7-M):

```rust
// Add a ReadOnlyExecutable mapping for the kernel's code into userspace
// to allow the cortex_m's `svc_return` to drop privilege and still
// be executable.
//
// TODO: https://pwbug.dev/465500606 - Isolate `svc_return` into its own section
// to allow selectively mapping it into userspace instead of the whole kernel.
app.process.memory_mappings.insert(
    0,
    MemoryMapping {
        name: "kernel_code".to_string(),
        ty: MemoryMappingType::ReadOnlyExecutable,
        start_address: config.kernel.flash_start_address,
        size_bytes: config.kernel.flash_size_bytes,
    },
);
```

**The entire kernel code region is mapped as `ReadOnlyExecutable` into every user process!**

### Git History

This was added by **Erik Gilling** (konkers@google.com) on **December 5, 2025** in commit `41e6a4517d`:

```
pw_kernel: Refactor ARM syscalls to eliminate race

Bug: https://pwbug.dev/465499154
Change-Id: I12b9043ea07641d5cec8f08453ad1d60e212e6f9
Reviewed-by: Travis Geiselbrecht <travisg@google.com>
```

The decision to map kernel code into user space was a deliberate trade-off to fix a syscall race condition, with a follow-up TODO bug (pwbug.dev/465500606) to properly isolate just the `svc_return` trampoline.

### Why This Exists

The `svc_return` assembly trampoline in [pw_kernel/arch/arm_cortex_m/syscall.rs](../../../../arch/arm_cortex_m/syscall.rs) needs to:
1. Execute while transitioning from privileged to unprivileged mode
2. The transition happens mid-instruction-stream via exception return

Without mapping kernel code into user space, the CPU would fault when the `svc_return` code attempts to execute after dropping privileges.

### PMSAv7 MPU Configuration

The MPU region for kernel code is configured in [protection_v7.rs](../../../../arch/arm_cortex_m/protection_v7.rs) with:
- `RasrAp::RoAny` (0b110) = Read-only access for both privileged and unprivileged
- `XN = false` = Execute permitted

This grants unprivileged execute permission to the entire kernel flash region.

### Security Implications

1. **Code disclosure:** User processes can read all kernel code
2. **Unintended execution:** User code can call any kernel function directly (as we discovered with the tokenizer)
3. **Partial mitigation:** Kernel code cannot write to kernel RAM from user mode, limiting damage

### Proper Fix (TODO: pwbug.dev/465500606)

Isolate `svc_return` into its own linker section and map only that small section into user space, rather than the entire kernel.

### Workaround (Implemented)

The fix in [system_image.bzl](../../../../tooling/system_image.bzl) forces user apps to use `log_backend_basic` (syscall-based) instead of `log_backend_tokenized`, preventing user code from calling the tokenizer directly.

---

## Phase 1: Compare with Working MPS2_AN505 Build ✅ COMPLETED

### Goal
Determine if MPS2_AN505 (ARMv8-M) has the tokenizer in user space or if it works differently.

### Commands Executed (2026-01-19)

```bash
# Build MPS2_AN505 IPC test
bazelisk build //pw_kernel/target/mps2_an505/ipc/user:ipc --config=k_qemu_mps2_an505
# Build completed successfully

# Check tokenizer symbol locations  
arm-none-eabi-nm bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf | grep -i "tokenize_to_default"
# Result: 0x1000408a - tokenize_to_default_writer (USER SPACE!)

arm-none-eabi-nm bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -i "tokenize_to_default"
# Result: 0x00003ec6 - tokenize_to_default_writer (KERNEL SPACE!)
```

### KEY FINDINGS

| Target | `tokenize_to_default_writer` Address | Location |
|--------|-------------------------------------|----------|
| **MPS2_AN505** | `0x1000408a` | ✅ USER CODE (0x10000000+ range) |
| **AST1030** | `0x00003ec6` | ❌ KERNEL CODE (inside .code @ 0x420) |

### AST1030 Memory Layout (from objdump)

| Section | VMA | Size | Purpose |
|---------|-----|------|---------|
| `.code` | 0x00000420 | 0x4d8c | **Kernel code (tokenizer at 0x3ec6 is HERE!)** |
| `.code.initiator_0` | 0x00020000 | 0x05f8 (~1.5KB) | User initiator app |
| `.code.handler_1` | 0x00040000 | 0x05dc (~1.5KB) | User handler app |

### MPS2_AN505 vs AST1030 Comparison

The MPS2_AN505 binary shows tokenizer at `0x1000408a` which is in the **user code range** (0x10000000+), while AST1030 has it at `0x00003ec6` in the **kernel code range**.

**This confirms the hypothesis:** MPS2_AN505 properly links the tokenizer into user binaries, but AST1030 does NOT.

### Additional Finding: No User-Side Tokenizer in AST1030

```bash
arm-none-eabi-nm bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -E "initiator_0.*tokenize|handler_1.*tokenize"
# Result: NO MATCHES - tokenizer is completely missing from user sections
```

### Conclusion

**Root cause confirmed:** The tokenizer is linked into kernel space on AST1030, but into user space on MPS2_AN505. This is likely a linker script or build configuration difference.

**Next step:** Phase 2 - Analyze build configuration differences to find why.

---

## Phase 2: Analyze Build Configuration Differences ✅ COMPLETED

### Goal
Find why tokenizer is not linked into user apps.

### Findings (2026-01-19)

#### BUILD.bazel Comparison
```bash
diff pw_kernel/target/ast1030/ipc/user/BUILD.bazel pw_kernel/target/mps2_an505/ipc/user/BUILD.bazel
```
**Result:** Files are identical except for platform paths. Not the cause.

#### Main Platform BUILD Comparison
```bash
diff pw_kernel/target/ast1030/BUILD.bazel pw_kernel/target/mps2_an505/BUILD.bazel
```
**Differences:**
- CPU constraints: `cortex-m3/armv7-m` (AST1030) vs `cortex-m33/armv8-m` (MPS2_AN505)
- Linker template: `target.ld.tmpl` vs `target.ld.jinja`

#### Log Backend Configuration

**Default setting in `pw_kernel/flags.bzl`:**
```python
KERNEL_COMMON_FLAGS = {
    # Default to using the tokenized backend
    "@pigweed//pw_log/rust:pw_log_backend": "@pigweed//pw_kernel:log_backend_tokenized",
    ...
}
```

**Key discovery from `pw_kernel/BUILD.bazel`:**
```python
alias(
    name = "log_backend_tokenized",
    actual = select({
        "//pw_kernel/userspace:userspace_build_enabled": "//pw_kernel/userspace/log_backend:tokenized",
        "//conditions:default": "//pw_kernel/subsys/console:pw_log_backend_tokenized",
    }),
)
```

**This means:**
- When `userspace_build=True` → uses `//pw_kernel/userspace/log_backend:tokenized`
- When `userspace_build=False` (default) → uses `//pw_kernel/subsys/console:pw_log_backend_tokenized`

**The userspace tokenized backend (`pw_kernel/userspace/log_backend/BUILD.bazel`):**
```python
rust_library(
    name = "tokenized",
    deps = [
        "//pw_tokenizer/rust:pw_tokenizer",  # <-- Has tokenizer dep!
        ...
    ],
)
```

#### Linker Script Comparison
```bash
diff pw_kernel/target/ast1030/target.ld.tmpl pw_kernel/target/mps2_an505/target.ld.jinja
```
**Differences:**
- AST1030 has tokenizer sections embedded directly
- MPS2_AN505 uses `{% include "pigweed_linker_sections.ld.jinja" %}`
- Both produce the same `.pw_tokenizer.entries` section for metadata

#### User App Dependencies
```bash
bazelisk query "deps(//pw_kernel/tests/ipc/user:initiator)" 2>&1 | grep -i "tokenize\|log_backend"
```
**Result:**
- `//pw_log/rust:pw_log_backend_printf` - NOT tokenized!
- No tokenizer deps found

**This is the bug!** The user apps are using `pw_log_backend_printf` instead of the kernel's tokenized backend.

### Root Cause Analysis

The `system_image` rule in `system_image.bzl` applies `_app_target_transition` which sets:
```python
flags = {
    "//pw_kernel/userspace:userspace_build": True,
    ...
}
```

But it does NOT set `//pw_log/rust:pw_log_backend`. The user apps inherit the global default `pw_log_backend_printf` instead of the kernel-specific `log_backend_tokenized`.

**Why MPS2_AN505 works:** Unknown - need to rebuild MPS2 and verify the tokenizer location is actually different or if there's another difference (e.g., MPU behavior).

### Recommended Fix

**Option 1:** Add `pw_log_backend` to `_app_target_transition`:
```python
def _app_target_transition_impl(_, attr):
    flags = {
        "//command_line_option:platforms": str(attr.platform),
        str(Label("//pw_kernel/target:system_config_file")): str(attr.system_config),
        str(Label("//pw_kernel/userspace:userspace_build")): True,
        # ADD THIS:
        str(Label("//pw_log/rust:pw_log_backend")): str(Label("//pw_kernel:log_backend_tokenized")),
    }
    return flags
```

**Option 2 (IMPLEMENTED ✅):** Use `log_backend_basic` for user apps (no tokenizer needed, uses syscall):
```python
str(Label("//pw_log/rust:pw_log_backend")): str(Label("//pw_kernel:log_backend_basic")),
```

This fix was implemented in the `log-backend-fix` branch (commit `7700e5e51`). See [system_image.bzl](../../../../tooling/system_image.bzl).

---

## Phase 3: Investigate MPU Configuration

### Goal
Understand why unprivileged code can execute kernel functions without faulting.

### PMSAv7 MPU Background

On ARMv7-M, the MPU has:
- Up to 8 regions
- Each region has: Base Address, Size, Access Permissions, Execute Never (XN) bit
- Access Permissions (AP bits): Define privileged vs unprivileged read/write
- XN bit: Prevents instruction fetch (execution) from region

### Check Current MPU Config

```bash
# In GDB, after hitting user breakpoint:
dump_mpu

# Or manually:
# MPU_TYPE at 0xE000ED90 - number of regions
# MPU_CTRL at 0xE000ED94 - enable, PRIVDEFENA, HFNMIENA
# MPU_RNR at 0xE000ED98 - region number select
# MPU_RBAR at 0xE000ED9C - region base address
# MPU_RASR at 0xE000EDA0 - region attribute and size
```

### RASR Register Bits (PMSAv7)

| Bits | Field | Description |
|------|-------|-------------|
| 0 | ENABLE | Region enable |
| 1-5 | SIZE | Region size (2^(SIZE+1) bytes) |
| 8-15 | SRD | Subregion disable |
| 16-18 | B,C,S | Memory type |
| 19 | TEX[0] | Memory type |
| 24-26 | AP | Access permissions |
| 28 | XN | Execute Never |

### AP Field Values

| AP | Privileged | Unprivileged |
|----|------------|--------------|
| 000 | No access | No access |
| 001 | RW | No access |
| 010 | RW | RO |
| 011 | RW | RW |
| 101 | RO | No access |
| 110 | RO | RO |
| 111 | RO | RO |

### Key Check

For kernel code region (0x0 - 0x1FFFF), verify:
- **XN bit (bit 28)** should be SET for unprivileged to prevent execution
- **AP bits** should be 001 or 101 (privileged only)

```gdb
# Select kernel code region (likely region 0)
set *(unsigned int*)0xE000ED98 = 0

# Read RASR
p/x *(unsigned int*)0xE000EDA0

# Check XN bit (bit 28)
# If RASR & 0x10000000 == 0, XN is NOT set (bug!)
```

### Files to Check

```bash
# MPU configuration code
grep -r "mpu\|MPU\|rasr\|RASR" pw_kernel/arch/arm_cortex_m/
grep -r "XN\|execute" pw_kernel/arch/arm_cortex_m/

# PMSAv7 specific config
cat pw_kernel/arch/arm_cortex_m/pmsav7.rs
```

---

## Phase 4: Fix Options

### Option A: Link Tokenizer into User Binary

**Approach:** Modify build to include tokenizer in user app.

**Pros:**
- User code works as-is
- No syscall overhead for logging

**Cons:**
- Increases user binary size
- Duplicates tokenizer code (kernel + each user app)

**Implementation:**
```python
# In pw_kernel/userspace/log_backend/BUILD.bazel
# Ensure tokenizer is properly linked:
rust_library(
    name = "tokenized",
    ...
    deps = [
        ...
        "//pw_tokenizer/rust:pw_tokenizer",  # This should be in user space
    ],
)
```

### Option B: Use Basic (Syscall) Log Backend

**Approach:** Use `log_backend:basic` which calls syscall for all logging.

**Pros:**
- No tokenizer needed in user space
- Simpler user binary

**Cons:**
- More syscall overhead
- Kernel must handle logging

**Implementation:**
```python
# In pw_kernel/BUILD.bazel, change log backend selection:
"//pw_kernel/userspace:userspace_build_enabled": "//pw_kernel/userspace/log_backend:basic",
```

### Option C: Fix MPU to Fault on Kernel Execution

**Approach:** Configure MPU to set XN bit on kernel code for unprivileged.

**Pros:**
- Proper security
- Catches similar bugs in future

**Cons:**
- Doesn't fix the actual linkage issue
- Test will fault instead of succeed

**Implementation:**
```rust
// In pw_kernel/arch/arm_cortex_m/pmsav7.rs
// When configuring kernel region, ensure XN bit is set:
let rasr = (size_bits << 1)       // SIZE
         | (ap_priv_only << 24)   // AP = privileged only
         | (1 << 28)              // XN = execute never for unprivileged
         | 1;                     // ENABLE
```

### Option D: Fix Linker Script to Include All User Code

**Approach:** Ensure linker script places all user deps in user sections.

**Pros:**
- Clean fix
- User code self-contained

**Cons:**
- May require significant linker script changes

**Files:**
```
pw_kernel/target/ast1030/ipc/user/linker_script.ld
pw_kernel/userspace/arm_cortex_m/linker_user.ld
```

---

## Phase 5: Testing the Fix

### After Implementing Fix

```bash
# Rebuild
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc --config=k_qemu_ast1030

# Verify tokenizer is now in user space
arm-none-eabi-nm bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -i tokenize
# Should show addresses >= 0x20000

# Check user code section size increased
arm-none-eabi-objdump -h bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep "code\."
# Should show larger .code.initiator_0 section

# Run test
bazelisk run //pw_kernel/target/ast1030/ipc/user:ipc --config=k_qemu_ast1030
```

### Debug Verification

```bash
# Start debug session
./pw_kernel/target/ast1030/ipc/user/debug_session.sh

# In GDB:
trace_startup
continue

# Should now hit SVCall after main
break SVCall
continue
```

---

## Quick Reference: Address Ranges

| Region | Start | End | Purpose |
|--------|-------|-----|---------|
| Kernel Code | 0x00000000 | 0x0001FFFF | Kernel .code section |
| User Initiator Code | 0x00020000 | 0x0003FFFF | Initiator app |
| User Handler Code | 0x00040000 | 0x0005FFFF | Handler app |
| Kernel RAM | 0x00060000 | 0x0007FFFF | Kernel .data, .bss, stack |
| User Initiator RAM | 0x00080000 | 0x00087FFF | Initiator stack |
| User Handler RAM | 0x00088000 | 0x0008FFFF | Handler stack |

---

## Checklist

- [x] Phase 1: Compare MPS2_AN505 tokenizer location ✅ **CONFIRMED: tokenizer at 0x1000408a (user space)**
- [x] Phase 1: Verify MPS2_AN505 has tokenizer in user code ✅ **AST1030 has it in kernel at 0x3ec6**
- [x] Phase 2: Identify BUILD.bazel differences ✅ **BUILD files identical except platform paths**
- [x] Phase 2: Check linker script differences ✅ **Both produce same tokenizer sections**
- [x] Phase 2: Find root cause ✅ **User apps use default pw_log_backend_printf, not kernel's tokenized backend**
- [x] Phase 3: Analyze MPU config ✅ **Found via code review: kernel code intentionally mapped RoAny + XN=false for svc_return (see system_generator/lib.rs)**
- [ ] Phase 3: Dump MPU config in GDB (optional - root cause found via code analysis)
- [x] Phase 4: Choose fix option (A, B, C, or D) ✅ **Option B (log_backend_basic)**
- [x] Phase 4: Implement fix ✅ **Committed in log-backend-fix branch (7700e5e51)**
- [ ] Phase 5: Verify tokenizer in user space
- [ ] Phase 5: Run test to completion
- [ ] Phase 5: Debug session shows SVCall triggered
