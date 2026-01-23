# Missing SVC Instructions on ARMv7-M Targets

**Date:** January 20, 2026  
**Status:** ✅ FIXED  
**Affects:** AST1030 and all ARMv7-M (Cortex-M4, M3) targets using `syscall_user` crate

## Executive Summary

User applications on ARMv7-M targets had **no syscall capability** because the `syscall_user` crate's `crate_features` select() was missing `@platforms//cpu:armv7-m`. This caused ARMv7-M targets to fall through to `//conditions:default`, which selected the `arch_host` stub implementation - a test-only stub with no `svc` instructions.

## Root Cause

### The Bug

In [pw_kernel/syscall/BUILD.bazel](../../../../syscall/BUILD.bazel), the `syscall_user` library had:

```python
crate_features = select({
    "@platforms//cpu:armv8-m": ["arch_arm_cortex_m"],  # ✓ ARMv8-M (Cortex-M33)
    "@platforms//cpu:riscv32": ["arch_riscv"],          # ✓ RISC-V
    "//conditions:default": ["arch_host"],              # ← ARMv7-M fell here!
}),
```

**AST1030 uses a Cortex-M4 (ARMv7-M), NOT ARMv8-M!** So it matched `//conditions:default` and used `host.rs`.

### The Host Stub

The `host.rs` implementation is meant for **unit testing on x86/x64 hosts only**. It has no `svc` instruction:

```rust
// syscall_user/host.rs - NO svc instruction!
pub fn channel_transact(...) -> Result<u32> {
    Err(pw_status::Error::Unimplemented)
}

pub fn debug_log(...) -> Result<()> {
    Err(pw_status::Error::Unimplemented)
}
// ... all syscalls return Err(Unimplemented)
```

### The Real ARM Implementation

The `arm_cortex_m.rs` implementation generates actual syscalls:

```rust
// syscall_user/arm_cortex_m.rs - Has svc instruction!
macro_rules! syscall_asm {
    ($id:expr, ...) => {
        unsafe {
            core::arch::asm!(
                "mov r11, {id}",
                "svc 0",           // ← Triggers SVCall exception
                ...
            )
        }
    };
}
```

## The Fix

Added `@platforms//cpu:armv7-m` to the select():

```python
crate_features = select({
    "@platforms//cpu:armv7-m": ["arch_arm_cortex_m"],  # ← ADDED
    "@platforms//cpu:armv8-m": ["arch_arm_cortex_m"],
    "@platforms//cpu:riscv32": ["arch_riscv"],
    "//conditions:default": ["arch_host"],
}),
```

## Verification

### Before Fix

```bash
$ arm-none-eabi-objdump -d ipc.elf | grep -E "svc\s"
# (no output - NO svc instructions in user code!)
```

### After Fix

```bash
$ arm-none-eabi-objdump -d ipc.elf | grep -E "svc\s"
   20050:       df00            svc     0   # initiator: debug_log
   20068:       df00            svc     0   # initiator: channel_transact
   2007e:       df00            svc     0   # initiator: shutdown
   40044:       df00            svc     0   # handler: object_wait
   40060:       df00            svc     0   # handler: channel_read
   4007a:       df00            svc     0   # handler: channel_respond
   40094:       df00            svc     0   # handler: debug_log
   400aa:       df00            svc     0   # handler: shutdown
```

## Symptoms Before Fix

With the yield fix applied and SysTick enabled:
- User threads were created and scheduled correctly
- MPU was properly reprogrammed for each user process
- Context switches happened continuously
- **But user code could never make syscalls** - all syscall wrappers just returned `Err(Unimplemented)`
- User threads would spin forever after attempting to log or communicate

### Why the Infinite Loop

The `pw_log::info!()` macro expands to syscall code. Without the `svc` instruction:

```rust
// What user code intends:
fn main() {
    pw_log::info!("Handler starting");  // Should invoke debug_log syscall
    loop {
        syscall::object_wait(...)?;      // Should block on IPC channel
        // ...
    }
}

// What the compiler sees (with host.rs stub):
fn main() {
    let _ = Err(Unimplemented);  // Log returns immediately
    loop {
        let _ = Err(Unimplemented);  // object_wait returns immediately
        // Empty loop body - no side effects
    }
}
```

The compiler optimizes the empty loop to:
```asm
40084:  b.n 0x40084    ; loop {} → infinite spin
```

## Files Changed

| File | Change |
|------|--------|
| [pw_kernel/syscall/BUILD.bazel](../../../../syscall/BUILD.bazel) | Added `"@platforms//cpu:armv7-m": ["arch_arm_cortex_m"]` |

## Affected Targets

This fix is required for any ARMv7-M based target:
- **AST1030** (Cortex-M4)
- **LM3S6965** (Cortex-M3)
- Any other Cortex-M3/M4/M4F targets

ARMv8-M targets (Cortex-M33, M23) were already working correctly.

## Diagnostic Commands

```bash
# Check for svc instructions in user code
arm-none-eabi-objdump -d ipc.elf | grep -E "svc\s"

# Check which syscall implementation is being used
bazelisk cquery "//pw_kernel/syscall:syscall_user" --config=k_qemu_ast1030 --output=starlark --starlark:expr="str(providers(target))"

# Verify crate_features selection
bazelisk query "//pw_kernel/syscall:syscall_user" --output=build
```

## Related Documentation

- [theories.md](theories.md) - Full investigation history
- [CURRENT_STATUS.md](CURRENT_STATUS.md) - Overall debug status

## Lessons Learned

1. **Bazel `select()` statements must cover all target architectures explicitly** - relying on `//conditions:default` for production code is risky
2. **Binary verification is essential** - always check the compiled output (`objdump`) to confirm expected instructions are present
3. **ARMv7-M and ARMv8-M are different platforms** - code that works on Cortex-M33 may not work on Cortex-M4 due to Bazel platform constraints
