# User Exception Frame Initialization Investigation

## Related Documentation

- [CONTROL Register Derivation Fix](control_register_derivation_fix.md) - Previous fix that is now verified working

## Issue Summary

After verifying the CONTROL register fix is correctly compiled into the binary, the AST1030 IPC userspace test still crashes with a MemManage fault during the first kernel-to-userspace context switch.

## Current Symptoms

```
[INF] PendSV returning frame: psp=0x000a8400 control=0x00000003 ret_addr=0xfffffffd
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
[DBG] MPU[1]: RBAR=0x000A0000 RASR=0x130BE31F
...
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x0a8408:
[INF] r0  0x00000000 r1 0x00000000 r2  0x00000000 r3  0x00000000
[INF] r12 0x00000000 lr 0x00000000 pc  0x00000000 psr 0x40000000
```

**Critical observation**: The user exception frame at `0x0A8408` shows:
- `pc = 0x00000000` (should be `0x00060421` - handler entry point)
- `psr = 0x40000000` (T-bit = 0, invalid for Thumb mode; should have T-bit set)
- All registers zero

This indicates the exception frame was either:
1. Never properly initialized, OR
2. Initialized at wrong address, OR
3. Corrupted before exception return

## What We Know Works

- ✅ CONTROL register derivation from EXC_RETURN (verified in disassembly)
- ✅ MPU configuration (entry point 0x60421 in enabled executable subregion 3)
- ✅ EXC_RETURN value 0xFFFFFFFD (Thread mode, PSP, non-FP)
- ✅ PSP value 0xA8400 logged correctly before context switch

## Investigation Plan

### Phase 1: Trace User Frame Initialization

1. **Find `initialize_user_frame` implementation**
   - Locate in `pw_kernel/kernel` or arch-specific code
   - Verify it writes correct PC (entry point) and PSR (with T-bit set) to the frame

2. **Check frame layout expectations**
   - ARMv7-M exception frame layout (without FP):
     ```
     PSP+0x00: R0
     PSP+0x04: R1
     PSP+0x08: R2
     PSP+0x0C: R3
     PSP+0x10: R12
     PSP+0x14: LR (return address)
     PSP+0x18: PC (entry point)
     PSP+0x1C: xPSR (must have T-bit set: bit 24 = 1)
     ```

3. **Verify addresses match**
   - Log shows `psp=0x000a8400`
   - Exception frame dump is at `0x0a8408` (PSP + 8 bytes offset?)
   - Why the 8-byte discrepancy?

### Phase 2: Memory Verification

1. **Check if memory at 0xA8400 is actually RAM**
   - Verify QEMU memory map for AST1030
   - Check `system.json5` RAM configuration

2. **Verify PSP is set correctly before exception return**
   - The restore path does `msr psp, r0` with r0 from stack
   - Confirm the value being written matches expected user stack

### Phase 3: Add Debug Instrumentation

1. **Dump user frame contents before `bx lr`**
   - Add inline assembly to read memory at PSP and log it
   - Or use QEMU's `-d` tracing options

2. **Check if frame gets corrupted between init and use**
   - Log frame contents immediately after `initialize_user_frame`
   - Log again just before PendSV returns

## Files to Examine

| File | Purpose |
|------|---------|
| `pw_kernel/kernel/src/thread.rs` | Thread initialization, likely calls `initialize_user_frame` |
| `pw_kernel/arch/arm_cortex_m/src/lib.rs` | Arch-specific thread frame setup |
| `pw_kernel/arch/arm_cortex_m/src/exceptions.rs` | Exception frame definitions |
| `pw_kernel/target/ast1030/ipc/user/system.json5` | Memory layout configuration |
| `pw_kernel/kernel/src/scheduler.rs` | Context switch logic |

## Hypotheses

### Hypothesis 1: Frame Written to Wrong Address
The `initialize_user_frame` function may be writing the exception frame to a different address than where PSP points during exception return.

**Test**: Log the address where frame is written vs PSP value at context switch.

### Hypothesis 2: PSP Offset Issue
The log shows exception frame at `0x0a8408` but PSP is `0x000a8400`. The 8-byte difference suggests:
- Frame may be at PSP-8 (pre-decremented)
- Or there's confusion about frame pointer vs stack pointer

**Test**: Verify exception frame layout and stack pointer conventions.

### Hypothesis 3: Memory Not Mapped in QEMU
The RAM at `0xA8400` may not be present in QEMU's AST1030 emulation.

**Test**: Check QEMU AST1030 machine definition and memory map.

### Hypothesis 4: Zero-Initialization Overwrites Frame
The frame may be initialized correctly but then zeroed by BSS initialization or similar.

**Test**: Add canary values to frame and check if they're preserved.

## Cross-Target Comparison (2026-01-15)

Running the same IPC test on LM3S6965 (Cortex-M3) reveals critical differences:

### LM3S6965 (Cortex-M3) - User frame is VALID
```
[INF] initialize_user_frame: user_frame=0x2000bfe0 kernel_frame=0x20001070 psp=0x2000bfe0 pc=0x00028201 exc_ret=0xfffffffd
[INF] PendSV returning frame: psp=0x2000bfe0 control=0x00000003 ret_addr=0xfffffffd
...
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x2000bfe0:
[INF] pc  0x00028204 psr 0x01000000
```
- `pc = 0x00028204` - Valid PC (near entry 0x00028201, +3 bytes is expected)
- `psr = 0x01000000` - T-bit SET (bit 24 = 1) - valid Thumb mode
- **Frame is correctly initialized**

### AST1030 (Cortex-M4) - User frame is GARBAGE
```
[INF] initialize_user_frame: user_frame=0x000a8400 kernel_frame=0x00041448 psp=0x000a8400 pc=0x00060421 exc_ret=0xfffffffd
[INF] PendSV returning frame: psp=0x000a8400 control=0x00000003 ret_addr=0xfffffffd
...
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x0a8408:
[INF] pc  0x00000000 psr 0x40000000
```
- `pc = 0x00000000` - ZERO (should be 0x00060421)
- `psr = 0x40000000` - T-bit CLEAR - invalid for Thumb mode
- **Frame shows garbage/zeros**

### Key Differences

| Aspect | LM3S6965 | AST1030 |
|--------|----------|---------|
| User frame address | 0x2000bfe0 (SRAM) | 0x000a8400 (low addr) |
| Memory region | Standard SRAM | Unusual low address |
| PC in frame | Valid (0x28204) | Zero |
| PSR T-bit | Set (valid) | Clear (invalid) |

### AST1030 Memory Map (Confirmed)

From Zephyr device tree / ASPEED documentation:

| Region | Base Address | Size | Description |
|--------|-------------|------|-------------|
| sram0 (cached) | `0x00000000` | 448 KB | Code and data memory (ends at 0x70000) |
| sram1 (non-cached) | `0x00070000` | 320 KB | Non-cached data memory (ends at 0xC0000) |

**User frame at `0x000A8400`** falls within sram1 (0x70000-0xC0000) - this **IS valid RAM**.

### Analysis Update

Looking more closely at LM3S6965 output:
```
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x2000bfe0:
[INF] pc  0x00028204 psr 0x01000000
```

The PC is `0x00028204` (offset from entry `0x00028201`), meaning **code DID start executing** before faulting. The fault is a null pointer access (address 0x0).

**Different failure modes:**
| Target | Frame Valid? | Code Executed? | Fault Cause |
|--------|-------------|----------------|-------------|
| LM3S6965 | ✅ Yes | ✅ Yes (PC=0x28204) | Null pointer in user code |
| AST1030 | ❌ No (zeros) | ❌ No (PC=0) | Frame not initialized |

### New Hypothesis: FPU Frame Size Mismatch (Cortex-M4F)

**Key difference**: AST1030 has **Cortex-M4F** (with FPU), while LM3S6965 has **Cortex-M3** (no FPU).

The EXC_RETURN value `0xFFFFFFFD` has:
- Bit 4 (FType) = 1 → Standard frame (no FP context)

But if QEMU's Cortex-M4 has FPU enabled by default with lazy stacking:
- The CPU may expect an **extended frame** (26 words / 104 bytes) with FP registers
- The kernel sets up a **standard frame** (8 words / 32 bytes)
- PSP would be pointing to wrong offset in the frame

**Exception frame sizes:**
| Frame Type | Size | Contents |
|------------|------|----------|
| Standard (no FPU) | 32 bytes | R0-R3, R12, LR, PC, xPSR |
| Extended (with FPU) | 104 bytes | Standard + S0-S15, FPSCR, reserved |

The kernel code at `pw_kernel/arch/arm_cortex_m/threads.rs:355-358` creates EXC_RETURN with:
```rust
let exc_return = ExcReturn::new(
    ExcReturnStack::ThreadSecure,
    ExcReturnRegisterStacking::Default,
    ExcReturnFrameType::Standard,  // <-- No FP context
    ...
);
```

**To test**: Disable FPU in QEMU or ensure FPCCR.LSPEN=0 (lazy stacking disabled).

### Alternative: QEMU sram1 Not Emulated

The AST1030 frame address `0x000A8400` should be in valid sram1, but QEMU may:
1. Only emulate sram0 (0x00000000 - 0x70000), not sram1 (0x70000 - 0xC0000)
2. Have different memory map than real hardware
3. Writes to 0xA8400 go nowhere, reads return zeros

**To test**: Check QEMU AST1030 source for memory regions.

## Commands

### Run the test
```bash
# AST1030
bazel test --test_output=streamed --test_timeout=10 \
  --cache_test_results=no --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/ipc/user:ipc_test

# LM3S6965 (for comparison)
bazel test --test_output=streamed --test_timeout=30 \
  --cache_test_results=no --config=k_qemu_lm3s6965 \
  //pw_kernel/target/lm3s6965/ipc/user:ipc_test
```

### Find frame initialization code
```bash
grep -rn "initialize_user_frame\|user_frame\|init.*frame" pw_kernel/
```

### Check memory layout
```bash
cat pw_kernel/target/ast1030/ipc/user/system.json5
cat pw_kernel/target/lm3s6965/ipc/user/system.json5
```

## Fix Plan: Disable FPU Lazy Stacking in Kernel Early Init

### Root Cause Confirmed

The AST1030 QEMU config uses `--cpu cortex-m4` which has an FPU with lazy stacking enabled by default. The kernel:
1. Uses soft-float ABI (no FPU instructions)
2. Sets up 32-byte standard exception frames
3. Does NOT disable FPU lazy stacking

When lazy stacking is enabled, the CPU may expect 104-byte extended frames with FP registers, causing PSP to point to the wrong offset and read zeros.

### Implementation

Add FPU lazy stacking disable to `pw_kernel/arch/arm_cortex_m/threads.rs` in `early_init()`:

```rust
// Disable FPU lazy stacking to prevent exception frame size mismatch.
// On Cortex-M4/M4F, if FPU is present and lazy stacking is enabled (default),
// exception frames can be 104 bytes instead of 32 bytes. Since this kernel
// uses soft-float ABI and doesn't save/restore FP context, we must disable
// lazy stacking to ensure consistent 32-byte exception frames.
//
// FPCCR (Floating-Point Context Control Register) at 0xE000EF34:
// - Bit 31 (ASPEN): Automatic state preservation enable
// - Bit 30 (LSPEN): Lazy state preservation enable
// Setting both to 0 disables FPU context stacking entirely.
unsafe {
    let fpccr = 0xE000EF34 as *mut u32;
    let val = fpccr.read_volatile();
    // Clear ASPEN (bit 31) and LSPEN (bit 30)
    fpccr.write_volatile(val & !(0x3 << 30));
}
```

### File to Modify

- `pw_kernel/arch/arm_cortex_m/threads.rs` - Add FPCCR configuration in `early_init()` around line 188 where the TODO comment mentions "FPU initial state"

### Verification

```bash
source ./activate.sh && bazel test --test_output=streamed --test_timeout=30 \
  --cache_test_results=no --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/ipc/user:ipc_test
```

Expected:
- No MemManage fault during context switch to userspace
- User frame shows valid PC (0x60421) and PSR (T-bit set)
- Test may still timeout on null pointer in user code (same as LM3S6965), but that's a separate issue

## Update (2026-01-15): FPCCR Fix Did NOT Resolve Issue

### FPU Lazy Stacking Fix Applied

Added FPCCR configuration to disable FPU lazy stacking in `early_init()`:
```rust
unsafe {
    let fpccr = 0xE000_EF34 as *mut u32;
    let val = fpccr.read_volatile();
    fpccr.write_volatile(val & !(0x3 << 30));
}
```

**Result**: Test still fails with same symptoms - exception frame shows zeros.

### QEMU Memory Analysis: Memory IS Mapped

Analysis of QEMU source code confirms:

| Finding | Details |
|---------|---------|
| SRAM Region | 0x00000000 - 0x000BFFFF (768 KB) |
| Address 0xA8400 | **IS within mapped RAM** |
| Source | `hw/arm/aspeed_ast10x0.c:495`: `sc->sram_size = 0xc0000` |

**Conclusion**: Memory IS mapped in QEMU. The zeros are NOT due to missing memory.

### Hypotheses Ruled Out

| Hypothesis | Status | Reason |
|------------|--------|--------|
| FPU lazy stacking | ❌ Ruled out | FPCCR fix applied, problem persists |
| QEMU memory not mapped | ❌ Ruled out | QEMU maps 768KB at 0x00000000 |

### Remaining Hypotheses

1. **Frame written to wrong address** - `initialize_user_frame` writes to different location than PSP points to
2. **Initialization order issue** - Frame zeroed after initialization (BSS init, etc.)
3. **MPU access permissions** - Frame in region without read access during exception return
4. **Cache coherency** - QEMU may not handle cached/non-cached SRAM distinction

### Next Investigation Steps

1. ~~Add debug output to dump frame contents immediately after `initialize_user_frame`~~ ✅ Done
2. ~~Verify the address passed to `initialize_user_frame` matches PSP at context switch~~ ✅ Done

## ROOT CAUSE FOUND (2026-01-15): MPU Region Overlap Bug

### Summary

The user exception frame IS being initialized correctly. The actual root cause is a **MPU region overlap bug** where the app's code region overlaps with kernel RAM, making kernel stack writes fail during exception stacking.

### Evidence

Added debug logging showed:
```
[INF] frame_init verify: addr=0x000a8400 pc=0x00060421 psr=0x01000000
[INF] User frame at PSP: pc=0x00060421 psr=0x01000000
```

The frame is correctly initialized with valid PC and PSR (T-bit set).

But the actual fault is:
```
[INF] HardFault exception triggered: HFSR=0x40000000 CFSR=0x00000092
[INF]   MMFSR.DACCVIOL: Data access violation
[INF]   MMFSR.MSTKERR: MemManage fault on stacking
[INF]   MMFSR.MMARVALID: MMFAR=0x00041c8c
```

The fault occurs at address `0x00041c8c` - this is **kernel RAM** (kernel RAM is 0x40420 - 0xA0420), not user RAM.

### Analysis

Looking at the initiator's MPU configuration:
```
[DBG] MPU[0]: RBAR=0x00040000 RASR=0x060FE023
```

Decoded RASR=0x060FE023:
- SIZE=17 → 256KB region (0x40000 - 0x7FFFF)
- SRD=0xE0 → subregions 5,6,7 disabled
- **AP=0x06 → RoAny (READ-ONLY for both privileged and unprivileged)**

The initiator's code region MPU[0] covers 0x40000-0x7FFFF with **read-only** permissions. But the kernel stack at `0x00041c8c` is within this range!

When an interrupt (like SysTick) fires after MPU is configured but before exception return completes:
1. CPU tries to stack exception frame on MSP (kernel stack)
2. MPU denies write access because region is RO
3. MSTKERR fault occurs

### The Bug

The app's MPU code region (starting at 0x40000) **overlaps with kernel RAM** (0x40420 - 0xA0420). The app region uses `RoAny` permission which makes the overlapping kernel memory read-only, breaking exception stacking.

### Memory Layout Issue

From system.json5:
- Kernel code: 0x00000420 - 0x00040420 (256KB)
- Kernel RAM: 0x00040420 - 0x000A0420 (384KB)

But MPU region for app code starts at 0x40000, not accounting for the 0x420 offset. This causes overlap.

### Fix Options

1. **Fix memory layout** - Ensure app regions don't overlap with kernel
2. **Fix MPU region calculation** - Account for base address offsets properly
3. **Disable interrupts during context switch** - Prevent SysTick from firing between MPU config and exception return (workaround, not fix)

### Why This Didn't Affect Handler Thread (First Context Switch)

Looking at handler's MPU config:
```
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
```

The handler's region starts at 0x00000000 and uses subregion masking that doesn't cover 0x40000-range kernel RAM as heavily. The initiator's config is different and causes the overlap.

### Previous Symptom Explanation

The earlier tests showed "zeros in exception frame" because:
1. First PendSV switched to handler (worked, but triggered SysTick)
2. SysTick preempted and triggered another PendSV
3. Second PendSV configured initiator MPU (which has the overlap bug)
4. MSTKERR occurred trying to stack on kernel RAM
5. HardFault showed different fault signature

The "zeros" were actually from the HardFault's exception frame dump, not the actual user frame.

## Updated Analysis (2026-01-15): Handler MPU Region Also Has Overlap

### Current Test Output

Running the test shows the bug is still present:

```
[INF] initialize_user_frame: user_frame=0x000a8400 kernel_frame=0x00041448 psp=0x000a8400 pc=0x00060421 exc_ret=0xfffffffd
[INF] frame_init verify: addr=0x000a8400 pc=0x00060421 psr=0x01000000
[INF] PendSV returning frame: psp=0x000a8400 control=0x00000003 ret_addr=0xfffffffd
[INF] User frame at PSP: pc=0x00060421 psr=0x01000000
[INF] Programming 8 MPU regions (PMSAv7)
[DBG] MPU[0]: RBAR=0x00000000 RASR=0x060FE727
[DBG] MPU[1]: RBAR=0x000A0000 RASR=0x130BE31F
...
[INF] MemoryManagement exception triggered: address=0x00000000
[INF] Exception frame 0x0a8408:
[INF] pc  0x00000000 psr 0x40000000
```

### Handler's MPU Region Decode

**MPU[0]**: `RBAR=0x00000000 RASR=0x060FE727`

Decoding RASR=0x060FE727:
- SIZE (bits 1-5) = 0x13 = 19 → Region size = 2^(19+1) = 2^20 = **1MB**
- SRD (bits 8-15) = 0xE7 = 0b11100111 → Subregions 0,1,2,5,6,7 **disabled**
- AP (bits 24-26) = 0x06 → **RoAny (read-only)**

With 1MB region at base 0, subregion size = 1MB/8 = 128KB:

| Subregion | Address Range | SRD Bit | Status |
|-----------|---------------|---------|--------|
| 0 | 0x00000 - 0x1FFFF | 1 | Disabled |
| 1 | 0x20000 - 0x3FFFF | 1 | Disabled |
| 2 | 0x40000 - 0x5FFFF | 1 | Disabled |
| **3** | **0x60000 - 0x7FFFF** | 0 | **ENABLED (RO)** |
| **4** | **0x80000 - 0x9FFFF** | 0 | **ENABLED (RO)** |
| 5 | 0xA0000 - 0xBFFFF | 1 | Disabled |
| 6 | 0xC0000 - 0xDFFFF | 1 | Disabled |
| 7 | 0xE0000 - 0xFFFFF | 1 | Disabled |

### The Overlap

Kernel RAM: `0x00040420 - 0x000A0420`

Kernel RAM spans:
- 0x40420 - 0x5FFFF → Subregion 2 (disabled) ✓
- **0x60000 - 0x7FFFF → Subregion 3 (ENABLED as RO)** ← BUG!
- **0x80000 - 0x9FFFF → Subregion 4 (ENABLED as RO)** ← BUG!
- 0xA0000 - 0xA0420 → Subregion 5 (disabled) ✓

**Result**: Kernel RAM from 0x60000-0x9FFFF is marked read-only by the handler's MPU code region.

### Why This Happens

The handler's code is at `[0x60420, 0x80420)` (128KB). The PMSAv7 MPU calculation:

1. Requested range: `[0x60420, 0x80420)` (128KB)
2. Round up to power-of-2: 128KB = 2^17
3. Align base: `0x60420 & ~(0x20000-1) = 0x60000`
4. Check coverage: `0x60000 + 0x20000 = 0x80000 < 0x80420` - doesn't cover!
5. Double region: 256KB, realign to `0x40000`
6. Check: `0x40000 + 0x40000 = 0x80000 < 0x80420` - still doesn't cover!
7. Keep doubling until 1MB region at base 0 covers the range
8. Use SRD to disable unused subregions

The algorithm correctly calculates the smallest PMSAv7-compliant region. The problem is that **the memory layout places app code at addresses that cause unavoidable overlap with kernel RAM**.

### Root Cause

The 0x420 offset (from vector table) causes app addresses to be misaligned relative to power-of-2 boundaries. This forces the MPU algorithm to use larger regions with subregion masking, and some subregions inevitably overlap kernel RAM.

### Proposed Fix: PMSAv7-Aligned Memory Layout

For PMSAv7, memory regions should be placed at power-of-2 aligned addresses to minimize MPU region bloat.

**Current Layout** (problematic):
```
0x00000000 - 0x00000420: Vector table (1056 bytes)
0x00000420 - 0x00040420: Kernel code (256KB)
0x00040420 - 0x000A0420: Kernel RAM (384KB)
0x000A0420 - ...: Apps (misaligned)
```

**Proposed Layout** (PMSAv7-friendly):
```
0x00000000 - 0x00000420: Vector table (1056 bytes)
0x00000420 - 0x00040000: Kernel code (~255KB, ends at 256KB boundary)
0x00040000 - 0x00080000: Kernel RAM (256KB, power-of-2 aligned)
0x00080000 - 0x000A0000: Handler code (128KB, power-of-2 aligned)
0x000A0000 - 0x000C0000: Initiator code (128KB, power-of-2 aligned)
0x000C0000 - ...: App RAM
```

Key changes:
1. **Kernel RAM ends at 0x80000** (power-of-2 boundary)
2. **App code starts at 0x80000** (power-of-2 aligned)
3. Each app region is naturally aligned, requiring minimal MPU region size

With this layout, handler code at `[0x80000, 0xA0000)` would produce:
- MPU region: base=0x80000, size=128KB (exact fit!)
- No subregion masking needed
- No overlap with kernel RAM at 0x40000-0x80000

### Implementation

A validation script is available to check PMSAv7 compatibility and suggest fixes:

```bash
python3 pw_kernel/target/ast1030/ipc/user/validate_memory_layout.py --suggest
```

The script analyzes the current `system.json5` and outputs:
1. Current memory layout with all regions
2. PMSAv7 subregion analysis showing which subregions overlap kernel RAM
3. A suggested PMSAv7-friendly layout with power-of-2 aligned addresses
4. Verification that the suggested layout has no overlaps

**Example suggested layout:**

```json5
kernel: {
    flash_start_address: 0x00000420,
    flash_size_bytes: 130016,          // ~126KB (ends at 0x00020000)
    ram_start_address: 0x00060000,     // After all flash ends
    ram_size_bytes: 131072,            // 128KB
},
// Apps will be placed:
//   Flash: starting at 0x00020000 (after kernel flash)
//   RAM: starting at 0x00080000 (after kernel RAM)
```

This layout places:
- Kernel flash: 0x420 - 0x20000 (ends at power-of-2 boundary)
- App flash: 0x20000 - 0x60000 (power-of-2 aligned, 128KB each)
- Kernel RAM: 0x60000 - 0x80000 (after all flash, power-of-2 aligned)
- App RAM: 0x80000+ (after kernel RAM)

Total: 544KB, fits within AST1030's 768KB SRAM.

## Next Steps

### Immediate Actions

1. **Update system.json5** with PMSAv7-friendly memory layout:
   ```bash
   # First, validate the suggested layout
   python3 pw_kernel/target/ast1030/ipc/user/validate_memory_layout.py --suggest
   ```
   Then update `pw_kernel/target/ast1030/ipc/user/system.json5` with the suggested values.

2. **Rebuild and test**:
   ```bash
   bazelisk test --test_output=streamed --test_timeout=30 \
     --cache_test_results=no --config=k_qemu_ast1030 \
     //pw_kernel/target/ast1030/ipc/user:ipc_test
   ```

3. **Verify MPU regions** in test output:
   - App code regions should have 1.0x bloat (exact fit)
   - No subregions should overlap kernel RAM

### Expected Outcome After Fix

- No MemManage fault during context switch to userspace
- User code should start executing (PC will advance from entry point)
- Test may still fail on null pointer dereference in user code (same as LM3S6965), but that's a separate issue in the user application

### Future Improvements

1. **Add validation to system generator** - The system generator (`pw_kernel/tooling/system_generator/lib.rs`) could warn when memory layouts are PMSAv7-unfriendly

2. **Consider PMSAv7 constraints in `populate_addresses()`** - Automatically align app regions to power-of-2 boundaries on ARMv7-M targets

3. **Document PMSAv7 constraints** - Add documentation about memory layout requirements for ARMv7-M targets in `pw_kernel/docs/`

## Explored But Not Needed: CONTROL Register Derivation

During debugging, we explored deriving the CONTROL register from EXC_RETURN instead of using the stacked value. This was investigated because we suspected CONTROL might be corrupted.

**Investigation finding**: The original code is correct. The save path pushes a placeholder (0xDEAD) for CONTROL because CONTROL is invariant per-thread. The restore path pops from the **incoming** thread's kernel frame (which was correctly initialized during `initialize_frame`). So the stacked value being restored is always correct for the new thread.

The CONTROL derivation change was reverted as unnecessary - the original design is sound
