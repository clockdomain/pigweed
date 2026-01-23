# AST1030 IPC Test Debug Plan Summary

**Date:** January 22, 2026  
**Status:** 🔧 Fix applied to correct thread - **STILL FAILING** - Need deeper investigation

---

## AST1030 IPC Test Design

### Overview

The AST1030 IPC test is a two-process user-mode application that tests inter-process communication (IPC) channels on the ASPEED AST1030 BMC SoC. The test consists of:

1. **Initiator process** - Sends lowercase characters ('a'-'z') to the handler
2. **Handler process** - Receives characters, converts to uppercase, and responds

The test validates:
- User mode entry and execution
- Syscall handling (channel operations)
- Context switches between two user processes
- IPC channel transact/read/respond operations

### Hardware Platform: AST1030

| Property | Value |
|----------|-------|
| CPU | ARM Cortex-M4F @ 200 MHz |
| Architecture | ARMv7-M |
| SRAM | 768 KB (0x00000000 - 0x000BFFFF) |
| MPU | PMSAv7 (8 regions) |
| Execution | XIP not supported - runs from SRAM |

**Key Difference from MPS2-AN505:**
- MPS2-AN505 uses ARMv8-M (PMSAv8) - flexible MPU with arbitrary region sizes
- AST1030 uses ARMv7-M (PMSAv7) - restrictive MPU requiring power-of-2 regions

### PMSAv7 Memory Layout Constraints

The PMSAv7 MPU has strict requirements that significantly impact memory layout design:

#### PMSAv7 Rules

1. **Power-of-2 Region Sizes**: Regions must be 32 bytes to 4 GB, always a power of 2
2. **Size-Aligned Bases**: Region base address must be aligned to its size
3. **Sub-Region Disable (SRD)**: Each region has 8 sub-regions (each 1/8th of total)
   - Sub-regions can only be fully enabled or fully disabled
   - Used to approximate arbitrary memory ranges

#### Memory Layout Design

The AST1030 system.json5 uses carefully aligned boundaries:

```
Memory Map (576 KB total, fits in 768 KB SRAM):
┌─────────────────────────────────────────────────────────┐
│ 0x00000000 │ Vector Table      │  1,056 B │ 0x420      │
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00000420 │ Kernel Code       │ ~126 KB  │ ends 128K  │
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00020000 │ Initiator Flash   │ 128 KB   │ power-of-2 │  ← App flash start
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00040000 │ Handler Flash     │ 128 KB   │ power-of-2 │
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00060000 │ Kernel RAM        │ 128 KB   │ power-of-2 │  ← RAM starts here
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00080000 │ Initiator RAM     │  32 KB   │            │
├────────────┼───────────────────┼──────────┼────────────┤
│ 0x00088000 │ Handler RAM       │  32 KB   │            │
└─────────────────────────────────────────────────────────┘
```

**Why Power-of-2 Alignment Matters:**

Without proper alignment, PMSAv7's sub-region mechanism can cause:
1. **Over-provisioning**: Extra memory exposed beyond requested range
2. **Kernel/User overlap**: User process could access kernel memory
3. **Process isolation failure**: One process could access another's memory

Example from [protection_v7.rs](../../../../arch/arm_cortex_m/protection_v7.rs):
```
Requested range: [0x1000, 0x1100) - 256 bytes
Aligned region:  [0x1000, 0x1800) - 2KB (power-of-2 requirement)
Sub-region size: 256 bytes (2KB / 8)
Sub-region 1:    [0x1100, 0x1300) - starts at requested end
Result: Sub-region 1 is FULLY enabled, exposing [0x1100, 0x1300)
        This grants 512 bytes of unintended access!
```

### System Configuration (system.json5)

```json5
{
    arch: {
        type: "armv7m",
        vector_table_start_address: 0x00000000,
        vector_table_size_bytes: 1056,  // 0x420
    },
    kernel: {
        flash_start_address: 0x00000420,
        flash_size_bytes: 130016,         // ~126KB (ends at power-of-2 boundary)
        ram_start_address: 0x00060000,    // After all flash (power-of-2 aligned)
        ram_size_bytes: 131072,           // 128KB
    },
    apps: [
        {
            name: "initiator",
            flash_size_bytes: 131072,     // 128KB (power-of-2)
            ram_size_bytes: 32768,        // 32KB
            process: {
                objects: [{ name: "IPC", type: "channel_initiator", ... }],
                threads: [{ name: "initiator thread", stack_size_bytes: 2048 }],
            },
        },
        {
            name: "handler",
            flash_size_bytes: 131072,     // 128KB (power-of-2)
            ram_size_bytes: 32768,        // 32KB
            process: {
                objects: [{ name: "IPC", type: "channel_handler" }],
                threads: [{ name: "handler thread", stack_size_bytes: 2048 }],
            },
        },
    ],
}
```

### System Generator Address Calculation

The tooling in [system_generator/lib.rs](../../../../tooling/system_generator/lib.rs) automatically calculates addresses:

```rust
fn populate_addresses(&mut self) {
    // Stack apps after kernel in flash and RAM
    let mut next_flash_start = kernel.flash_start + kernel.flash_size;
    next_flash_start = Self::align(next_flash_start, FLASH_ALIGNMENT);  // 4-byte
    
    let mut next_ram_start = kernel.ram_start + kernel.ram_size;
    next_ram_start = Self::align(next_ram_start, RAM_ALIGNMENT);  // 8-byte

    for app in apps.iter_mut() {
        app.flash_start_address = next_flash_start;
        next_flash_start = Self::align(app.flash_start + app.flash_size, FLASH_ALIGNMENT);
        
        app.ram_start_address = next_ram_start;
        next_ram_start = Self::align(app.ram_start + app.ram_size, RAM_ALIGNMENT);
        
        app.start_fn_address = flash_start + 1;  // +1 for Thumb mode
        app.initial_sp = app.ram_start + app.ram_size;  // Stack grows down
    }
}
```

**Memory mappings added automatically:**
1. `kernel_code` - ReadOnlyExecutable (for `svc_return` execution in user mode)
2. `flash` - ReadOnlyExecutable (app's code)
3. `ram` - ReadWriteData (app's data/stack)

### IPC Test Flow

```
┌─────────────┐                    ┌─────────────┐
│  Initiator  │                    │   Handler   │
│   Process   │                    │   Process   │
└──────┬──────┘                    └──────┬──────┘
       │                                  │
       │ pw_log::info("starting")         │ pw_log::info("starting")
       │                                  │
       │ for c in 'a'..='z':              │ loop:
       │   encode(c) → send_buf           │   syscall::object_wait(READABLE)
       │   syscall::channel_transact()────┼──► (blocks until message)
       │        │                         │   syscall::channel_read()
       │        │ ◄──── context switch ───┼──► upper_c = c.to_uppercase()
       │        │                         │   syscall::channel_respond()
       │   ◄────┼─────── response ────────┼───┘
       │   verify(recv == UPPER)          │
       │                                  │
       │ syscall::debug_shutdown(Ok)      │
       ▼                                  ▼
```

Each `syscall::channel_transact` involves:
1. **SVCall** - Trap to kernel (privilege elevation)
2. **Context switch** - Save initiator, load handler thread
3. **Handler syscalls** - wait/read/respond, each causing SVCall
4. **Context switch** - Save handler, load initiator thread
5. **Return** - Resume initiator with response data

### ARMv7-M vs ARMv8-M: Key Differences

| Feature | ARMv7-M (AST1030) | ARMv8-M (MPS2-AN505) |
|---------|-------------------|----------------------|
| MPU | PMSAv7 | PMSAv8 |
| Region sizes | Power-of-2 only | Any multiple of 32 bytes |
| Region alignment | Must align to size | 32-byte alignment |
| Sub-regions | 8 per region (SRD) | Not needed |
| Max regions | 8 | 8-16 |
| TrustZone | No | Yes (secure/non-secure) |
| Stack limit | No hardware check | Hardware stack limit (PSPLIM) |

### Build Configuration

**Bazel platform definition** ([BUILD.bazel](../../../ast1030/BUILD.bazel)):
```python
platform(
    name = "ast1030",
    constraint_values = [
        ":target_ast1030",
        "//pw_build/constraints/arm:cortex-m3",  # soft-float ABI
        "@platforms//cpu:armv7-m",
        "@platforms//os:none",
    ],
    flags = {
        "//pw_kernel/config:kernel_config": ":config",
        "//pw_kernel/subsys/console:console_backend": "...:semihosting",
    },
)
```

**Kernel config** ([config.rs](../../../ast1030/config.rs)):
```rust
impl CortexMKernelConfigInterface for KernelConfig {
    const SYS_TICK_HZ: u32 = 12_000_000;  // QEMU compatible
    const NUM_MPU_REGIONS: usize = 8;      // PMSAv7 limit
}
```

### Build and Run Commands

```bash
# Build AST1030 IPC test
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc_test \
  --config=k_qemu_ast1030

# Run AST1030 IPC test
bazelisk test //pw_kernel/target/ast1030/ipc/user:ipc_test \
  --config=k_qemu_ast1030 --test_output=streamed

# Build binary for debugging
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc \
  --platforms=//pw_kernel/target/ast1030

# Disassemble for analysis
arm-none-eabi-objdump -d bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

# Run with QEMU + GDB
qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic \
  -serial mon:stdio -semihosting-config enable=on,target=native \
  -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
  -S -gdb tcp::3333
```

---

## Hello User Test (Minimal Single-Process Test)

### Purpose
Created a minimal single-process user-mode test (`hello_user`) to isolate whether the CONTROL register corruption bug occurs:
1. **On initial user mode entry** (first time entering user mode)
2. **After syscalls** (subsequent context switches)

This helps narrow down the root cause by eliminating multi-process IPC complexity.

### Implementation

**Files Created:**

| File | Purpose |
|------|---------|
| [pw_kernel/tests/hello_user/hello.rs](../../../../tests/hello_user/hello.rs) | Minimal user app that logs and exits |
| [pw_kernel/tests/hello_user/BUILD.bazel](../../../../tests/hello_user/BUILD.bazel) | App package build rules |
| [pw_kernel/target/mps2_an505/hello_user/system.json5](../../mps2_an505/hello_user/system.json5) | System config for M33 |
| [pw_kernel/target/mps2_an505/hello_user/target.rs](../../mps2_an505/hello_user/target.rs) | Kernel target for M33 |
| [pw_kernel/target/mps2_an505/hello_user/BUILD.bazel](../../mps2_an505/hello_user/BUILD.bazel) | Build rules for M33 test |

**User App Code (`hello.rs`):**
```rust
#[entry]
fn entry() -> ! {
    // If we get here, user mode entry succeeded!
    pw_log::info!("🎉 Hello from user mode!");
    pw_log::info!("User mode entry successful - no fault on initial entry");

    // Now test a simple syscall (logging uses syscalls)
    pw_log::info!("Testing syscalls via logging...");

    // Signal test passed and exit
    pw_log::info!("✅ PASSED: User mode works correctly!");
    let _ = syscall::debug_shutdown(Ok(()));
    loop {}
}
```

### Test Results

**MPS2-AN505 (Cortex-M33/ARMv8-M):** ✅ PASSED
```bash
bazelisk test //pw_kernel/target/mps2_an505/hello_user:hello_user_test \
  --config=k_qemu_mps2_an505 --test_output=streamed
```

The test confirms:
- User mode entry works correctly on ARMv8-M
- Syscalls (via logging) work correctly
- Clean shutdown succeeds

### Key Findings

1. **The `#[entry]` macro requires:**
   - Function must be named `entry` (not `main`)
   - Return type must be `-> !`
   - A `#[panic_handler]` function is required

2. **Build Configuration:**
   - Use `--config=k_qemu_<target>` to enable QEMU runner
   - The config sets `--run_under` to the QEMU runner tool
   - Without this, the test fails with "Exec format error" (tries to execute ARM binary on x86)

3. **Test architecture matches IPC test:**
   - Uses `system_image` + `system_image_test` macros
   - Requires `target.rs` kernel entry point
   - Uses same memory layout (system.json5)

### Next Steps

1. **Create AST1030 version of hello_user** to test if fault happens on M4:
   - If fault occurs → Bug is in initial user mode entry
   - If no fault → Bug is specific to IPC/multi-process syscalls

2. **Add more granular tests** if hello_user passes on AST1030:
   - Test multiple syscalls in a row
   - Test thread yield
   - Test channel creation

### Build and Run Commands

```bash
# Build the hello_user test for MPS2-AN505
bazelisk build //pw_kernel/target/mps2_an505/hello_user:hello_user_test \
  --config=k_qemu_mps2_an505

# Run the hello_user test for MPS2-AN505
bazelisk test //pw_kernel/target/mps2_an505/hello_user:hello_user_test \
  --config=k_qemu_mps2_an505 --test_output=streamed

# Build only (without running) to inspect the binary
bazelisk build //pw_kernel/target/mps2_an505/hello_user:hello_user \
  --platforms=//pw_kernel/target/mps2_an505

# Run manually with QEMU (for debugging)
qemu-system-arm -machine mps2-an505 -cpu cortex-m33 -bios none -nographic \
  -serial mon:stdio -semihosting-config enable=on,target=native \
  -kernel bazel-bin/pw_kernel/target/mps2_an505/hello_user/hello_user.elf
```

---

## The Problem

The IPC test on **AST1030 (Cortex-M4/ARMv7-M)** fails with a **HardFault** (escalated from MemoryManagement) showing:
```
control=0x00000001
```
This is an invalid state: `nPRIV=1` (unprivileged) but `SPSEL=0` (using MSP instead of PSP).

The same test **passes** on **MPS2-AN505 (Cortex-M33/ARMv8-M)**.

---

## Fix Applied (But Still Failing)

### What We Fixed
Moved the ARMv7-M fix from `new_thread` to `active_thread` in `pendsv_swap_sp()`:

```rust
unsafe {
    (*active_thread).frame = frame;

    // ARMv7-M fix: Restore canonical CONTROL/EXC_RETURN values to the
    // ACTIVE thread's frame (the one we just saved with potentially corrupted values)
    #[cfg(all(feature = "user_space", feature = "armv7m"))]
    {
        let saved_frame = &mut *(*active_thread).frame;
        saved_frame.control = (*active_thread).canonical_control;
        saved_frame.return_address = (*active_thread).canonical_return_address;
    }

    set_active_thread(core::ptr::null_mut());
}
```

### Test Results After Fix

**Cortex-M33 (MPS2-AN505):** ✅ PASSED - No regression!

**Cortex-M4 (AST1030):** ❌ STILL FAILING
```
[INF] Starting thread 'handler thread' (0x00061990)
[INF] HardFault exception triggered: HFSR=0x40000000
[INF] psp 0x0008fea8 control 0x00000001 return_address 0xfffffffd
```

### Analysis

The fault still shows `control=0x00000001`. The HardFault (HFSR bit 30 = FORCED) indicates a MemoryManagement fault was escalated.

**Possible issues:**
1. The fix only applies when PendSV switches a thread OUT - but the fault may happen on initial entry
2. Another code path corrupts CONTROL before PendSV fix runs
3. The incoming thread's frame (set at initialization) may have issues

---

## Root Cause (Identified Earlier)

CONTROL register corruption during syscall context switches:

1. User thread running with `CONTROL=0x03` (nPRIV=1, SPSEL=1)
2. SVCall handler temporarily sets `CONTROL=0x02` for privilege elevation
3. PendSV fires during syscall processing
4. PendSV captures the transient `CONTROL=0x02` instead of `0x03`
5. When thread resumes, it has wrong CONTROL value → fault

See [control_corruption_fix_review.md](control_corruption_fix_review.md) for detailed analysis.

---

## 🐛 THE BUG: Fix Applies to Wrong Thread!

### GDB Debug Session Findings

Using GDB with breakpoints on the fix code (address `0x2180`), we discovered:

```
Breakpoint 21, 0x00002180 in pendsv_swap_sp ()
$8 = 0x623ac      # r0 = frame pointer  
$9 = 0x0          # r1 = canonical_control being stored
$10 = 0x0         # frame.control before store
$11 = 0x0         # frame.control after store
```

**Key finding:** `canonical_control = 0x0` (kernel thread value), NOT `0x3` (user thread value)!

### The Logic Error

The current fix code in `pendsv_swap_sp()`:

```rust
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    let frame = &mut *(*new_thread).frame;
    frame.control = (*new_thread).canonical_control;  // ← WRONG THREAD!
}
```

**The problem:** The fix applies `new_thread.canonical_control` but the **corrupted frame belongs to `active_thread`** (the interrupted user thread)!

### PendSV Context Switch Flow

1. **`active_thread`** = the thread that was just interrupted (user thread with corrupted `control=0x1`)
2. `(*active_thread).frame = frame;` - saves the corrupted frame
3. **`new_thread`** = scheduler's next thread (often a kernel thread with `canonical_control=0x0`)
4. Fix applies `new_thread.canonical_control` (0x0) to `new_thread.frame` ← **WRONG!**

The corrupted frame belongs to `active_thread`, not `new_thread`!

### The Fix

Apply canonical values to **`active_thread`** (the interrupted thread), not `new_thread`:

```rust
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    // Fix the ACTIVE thread's frame (the one we just saved with corrupted CONTROL)
    unsafe {
        let frame = &mut *(*active_thread).frame;
        frame.control = (*active_thread).canonical_control;
        frame.return_address = (*active_thread).canonical_return_address;
    }
}
```

This must be done **after** `(*active_thread).frame = frame;` but **before** losing the `active_thread` pointer.

---

## Previous Investigation (Resolved)

### Earlier Hypothesis (WRONG)
We initially thought `pendsv_swap_sp` was never called. GDB proved this wrong - it IS called.

### GDB Evidence
```
Breakpoint 11, 0x000005da in PendSV ()
lr             0xfffffffd          -3      ← User thread (PSP return)
psp            0x87fe0             557024  ← User stack present
control        0x1                 1       ← CORRUPTED (should be 0x3)
```

The fix code at `0x2180` (store instruction) was hit multiple times - the fix IS executing, just on the wrong thread!

### Binary Analysis

```
$ arm-none-eabi-objdump -t ipc.elf | grep -i pendsv
0000045c g     F .code  0000002c PendSV
000014fa g     F .code  00000316 pendsv_swap_sp
```

Vector table at offset 0x38 (PendSV):
```
38:   0000045d   # Points to PendSV (Thumb mode = addr | 1)
```

PendSV disassembly shows it calls pendsv_swap_sp:
```asm
0000045c <PendSV>:
     45c:   mrs     r1, CONTROL
     460:   mrs     r0, PSP
     464:   push    {r0, r1, lr}
     466:   stmdb   sp!, {r4-r11}
     46a:   mov     r0, sp
     46c:   sub     sp, #4
     46e:   cpsid   i
     470:   bl      14fa <pendsv_swap_sp>   ← Should call our fix!
     474:   cpsie   i
     ...
```

---

## Next Step: Apply the Correct Fix

### Location
`pw_kernel/arch/arm_cortex_m/threads.rs` in `pendsv_swap_sp()`, after line 483:
```rust
(*active_thread).frame = frame;
```

### Code Change
Move the fix to apply to `active_thread` instead of `new_thread`:

```rust
unsafe {
    (*active_thread).frame = frame;

    // ARMv7-M fix: Restore canonical CONTROL/EXC_RETURN values to the
    // ACTIVE thread's frame. The frame we just saved may have corrupted
    // values if PendSV fired during syscall processing.
    #[cfg(all(feature = "user_space", feature = "armv7m"))]
    {
        let frame = &mut *(*active_thread).frame;
        frame.control = (*active_thread).canonical_control;
        frame.return_address = (*active_thread).canonical_return_address;
    }

    set_active_thread(core::ptr::null_mut());
}
```

And **remove** the existing fix code near line 528-533 that applies to `new_thread`.

---

## Test Results Summary

| Target | Architecture | Test | Result | Notes |
|--------|--------------|------|--------|-------|
| MPS2-AN505 | ARMv8-M (Cortex-M33) | IPC | ✅ PASS | Full test passes |
| MPS2-AN505 | ARMv8-M (Cortex-M33) | hello_user | ✅ PASS | 100 syscalls pass |
| AST1030 | ARMv7-M (Cortex-M4) | IPC | ❌ FAIL | MemoryManagement exception |
| AST1030 | ARMv7-M (Cortex-M4) | hello_user (basic) | ✅ PASS | Initial entry + few syscalls work |
| AST1030 | ARMv7-M (Cortex-M4) | hello_user (stress) | ❌ FAIL | Crashes after ~25-50 syscalls |

### Key Finding: Bug is Syscall-Related, NOT IPC-Specific

The hello_user stress test **crashes** on AST1030 after ~25-50 syscalls:
```
[INF] Completed 25 syscalls
[INF] HardFault exception triggered: HFSR=0x40000000
[INF] psp 0x00000000 control 0x00000000 return_address 0xfffffff9
```

**Crash characteristics:**
- `psp = 0x00000000` - PSP is NULL (user stack pointer lost!)
- `control = 0x00000000` - Kernel mode (should be 0x3 for user mode)
- `return_address = 0xfffffff9` - EXC_RETURN for MSP thread mode

This is **different** from the IPC crash (which showed `control=0x1`), but proves:
1. ✅ Bug is NOT specific to multi-process IPC
2. ✅ Bug is reproducible with single user thread + many syscalls  
3. ✅ Bug is cumulative - doesn't happen immediately, builds up over time
4. ✅ Bug corrupts PSP to NULL and CONTROL to kernel mode

**Minimal reproduction:**
- Single user process with one thread
- Loop calling `syscall::debug_nop()` ~25-50 times
- Crashes on ARMv7-M (AST1030), passes on ARMv8-M (MPS2-AN505)

---

## Related Files

- [control_corruption_fix_review.md](control_corruption_fix_review.md) - Detailed root cause analysis
- [svc_debug_plan.md](svc_debug_plan.md) - Full syscall debug plan
- [DEBUG_SESSION_RESULTS.md](DEBUG_SESSION_RESULTS.md) - Previous debug session results
- `pw_kernel/arch/arm_cortex_m/threads.rs` - Fix implementation
- `pw_kernel/arch/arm_cortex_m/syscall.rs` - SVCall handler
- `pw_kernel/macros/arm_cortex_m_macro.rs` - Exception handler generation

---

## Commands Reference

```bash
# Build and run AST1030 IPC test
bazel run //pw_kernel/target/ast1030/ipc/user:ipc_test

# Build binary for analysis
bazel build //pw_kernel/target/ast1030/ipc/user:ipc

# Disassemble binary
arm-none-eabi-objdump -d bazel-out/ast1030-fastbuild/bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

# Check symbols
arm-none-eabi-objdump -t bazel-out/ast1030-fastbuild/bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -i pendsv
```

---

## GDB Debug Plan: Step Through pendsv_swap_sp

### Goal
Verify that the ARMv7-M fix code actually executes and check if it correctly overwrites `frame.control` with the canonical value.

### Prerequisites

1. **Find the fix code address** in the binary:
   ```bash
   # Disassemble pendsv_swap_sp and look for the canonical_control access
   arm-none-eabi-objdump -d bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf | grep -A 200 "<pendsv_swap_sp>:" | head -250
   ```

2. **Identify key locations in pendsv_swap_sp**:
   - Function entry (where frame pointer is in r0)
   - The `#[cfg(all(feature = "user_space", feature = "armv7m"))]` block
   - Where `frame.control` is written

### Session Setup

```bash
# Terminal 1: Start QEMU with GDB server
qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic \
  -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf \
  -semihosting-config enable=on,target=native -S -gdb tcp::3333

# Terminal 2: Connect GDB  
gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
(gdb) target remote :3333
```

### Step-by-Step Debug Procedure

#### Phase 1: Set Up Conditional Breakpoint

```gdb
# Load the debug script
source pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb

# Delete auto-continue on PendSV so we can examine state
delete 5

# Set breakpoint at pendsv_swap_sp entry
break pendsv_swap_sp

# Set conditional breakpoint that only triggers when CONTROL != 0
# (i.e., when we're dealing with a user thread)
break pendsv_swap_sp if $control != 0

# Or set breakpoint when PSP is non-zero (user thread has PSP set)
break pendsv_swap_sp if $psp != 0

continue
```

#### Phase 2: When Breakpoint Hits (User Thread Context Switch)

```gdb
# Check if this is a user thread context (CONTROL should be non-zero)
info registers
print/x $control
print/x $psp

# If control=0 and psp=0, this is kernel-to-kernel, continue
# If control!=0 or psp!=0, this involves a user thread - examine!

# r0 contains pointer to FullExceptionFrame
print/x $r0

# Dump the frame
dump_pendsv_frame

# Or manually examine:
# Frame layout: r4,r5,r6,r7,r8,r9,r10,r11 (32 bytes), psp (4), control (4), exc_return (4)
x/12x $r0
print/x *(unsigned int*)($r0 + 0x24)   # control in frame
```

#### Phase 3: Step Through the Fix Code

```gdb
# Disassemble current location
disassemble

# Single-step through pendsv_swap_sp
stepi
stepi
stepi
# ... continue stepping

# Look for instructions that:
# 1. Load from new_thread->canonical_control
# 2. Store to frame->control (offset 0x24)

# Watch for:
#   ldr rX, [rY, #offset]   ; load canonical_control
#   str rX, [r0, #0x24]     ; store to frame.control
```

#### Phase 4: Check Frame Before and After Fix

```gdb
# BEFORE the fix code runs:
print/x *(unsigned int*)($r0 + 0x24)   # Should show corrupted value (0x1 or 0x2)

# Step past the fix code
stepi
stepi
# ...

# AFTER the fix code runs:
print/x *(unsigned int*)($r0 + 0x24)   # Should now be 0x3 (canonical)
```

#### Phase 5: Check new_thread->canonical_control

```gdb
# Find the new_thread pointer (it's loaded in pendsv_swap_sp)
# Look for: static CURRENT_THREAD or get_current_arch_thread_state()

# Once you have the thread state pointer:
# ArchThreadState layout includes canonical_control at some offset
# Check pw_kernel/arch/arm_cortex_m/threads.rs for exact layout

# Examine the thread state
print/x $rX              # Where rX is the thread state pointer
x/20x $rX                # Dump thread state memory
```

### Key Things to Verify

1. **Is `new_thread` non-null?**
   - The fix only runs if `new_thread` is valid

2. **Is `canonical_control` set to 0x3?**
   - For user threads, this should be 0x3

3. **Does the store to `frame.control` actually happen?**
   - Watch for `str` instruction writing to frame+0x24

4. **Is the frame pointer (r0) still valid?**
   - It should point to the stack frame

### Alternative: Add Hardware Watchpoint

```gdb
# Set a watchpoint on the control field in the frame
# First, get the frame address when at pendsv_swap_sp
print/x $r0

# Set write watchpoint on frame.control
watch *(unsigned int*)($r0 + 0x24)

# Continue - will stop when control is written
continue

# Check what value was written
print/x *(unsigned int*)($r0 + 0x24)
```

### Expected Results

**If fix is working:**
- Frame control starts as 0x1 (corrupted)
- After fix code, frame control becomes 0x3 (correct)
- PendSV returns with correct CONTROL

**If fix is NOT working:**
- Frame control stays 0x1 throughout
- Either fix code doesn't execute, or canonical_control is wrong

---

## TMUX Debug Session

```bash
# Kill existing session if any
tmux kill-session -t ast1030-debug 2>/dev/null || true

# Start new tmux session with QEMU and GDB
tmux new-session -s ast1030-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -semihosting-config enable=on,target=native -S -gdb tcp::3333' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch -x pw_kernel/target/ast1030/ipc/user/debug_usermode.gdb bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -ex "target remote :3333"' C-m
```

## GDB Logging

```gdb
set logging file ast1030_debug.log
set logging overwrite on
set logging enabled on
```

---

## Build Commands

```bash
bazelisk test //pw_kernel/target/ast1030/ipc/user:ipc_test \
  --config=k_qemu_ast1030 --test_output=streamed

# Build the test
bazelisk build //pw_kernel/target/ast1030/ipc/user:ipc_test --config=k_qemu_ast1030
```
