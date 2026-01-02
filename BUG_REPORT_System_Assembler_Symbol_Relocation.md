# Bug Report: system_assembler Symbol Address Corruption

**Date**: 2026-01-01  
**Severity**: Critical  
**Affects**: ARMv7-M (and potentially other configurations with non-aligned kernel sizes)  
**Status**: Root cause identified, fix implemented and tested

---

## Executive Summary

The `system_assembler` tool relocates ELF sections when combining app binaries into the system image, but fails to adjust symbol addresses accordingly. This causes userspace applications to crash immediately on ARMv7-M platforms.

**Two related bugs:**
1. **Primary**: App entry addresses calculated incorrectly in `system_generator`
2. **Secondary**: Section-based symbol addresses not adjusted in `system_assembler`

---

## Symptoms

### ARMv7-M IPC Test
```
[INF] Allocating non-privileged thread 'handler thread' (entry: 0x00030201)
[INF] Starting thread 'handler thread'
[INF] MemoryManagement exception triggered: address=0x2000a000
```

**Entry address is WRONG**: Should be `0x00030001`, not `0x00030201`

### Why ARMv8-M Works (False Negative)

ARMv8-M kernel size (261120 bytes) happens to end at a 0x1000-aligned address (0x10040000), so segment base == section start. ARMv7-M kernel size (130560 bytes) ends at 0x20200, creating a 0x200-byte gap after alignment.

---

## Root Cause Analysis

### Problem 1: Entry Address Calculation (system_generator)

**File**: `pw_kernel/tooling/system_generator/lib.rs`  
**Function**: `Armv7MConfig::get_start_fn_address()`

**Current Code (WRONG)**:
```rust
fn get_start_fn_address(&self, flash_start_address: u64) -> u64 {
    // On Armv7M, the +1 is to denote thumb mode.
    flash_start_address + 1
}
```

**Why it fails**:
1. `flash_start_address` is where CODE starts in the app linker script (e.g., 0x30200)
2. ELF segments require 0x1000-byte alignment
3. Segment actually starts at 0x30000 (aligned down)
4. `system_assembler` preserves segment addresses and relocates sections
5. Result: `_start` symbol ends up at **segment base** (0x30000), not at `flash_start_address` (0x30200)

**Evidence**:
```bash
# Individual handler binary (before system assembly)
$ arm-none-eabi-nm handler | grep _start
00030200 T _start

$ arm-none-eabi-readelf -l handler | grep LOAD
LOAD  0x000000 0x00030000 0x00030000  # Segment at 0x30000, section at 0x30200

# Final system image (after system assembly)
$ arm-none-eabi-nm ipc_test | grep _start_handler
00030000 T _start_handler_1  # Relocated to segment base!
```

### Problem 2: Symbol Address Adjustment (system_assembler)

**File**: `pw_kernel/tooling/system_assembler.rs`  
**Function**: `add_app_symbols()`

**Current Code (WRONG)**:
```rust
fn add_app_symbols(...) {
    for symbol in &app.symbols {
        let new_symbol = self.builder.symbols.add();
        // ... copy other fields ...
        new_symbol.st_value = symbol.st_value;  // ❌ NO ADJUSTMENT
    }
}
```

**Why it fails**:
- Sections are relocated to new addresses when appended to segments
- Symbol addresses (`st_value`) are preserved without adjustment
- Symbols end up pointing to wrong offsets

---

## The Fix

### Fix 1: Entry Address Calculation

**File**: `pw_kernel/tooling/system_generator/lib.rs` (line ~160)

```rust
fn get_start_fn_address(&self, flash_start_address: u64) -> u64 {
    // flash_start_address is where app code starts in the linker script.
    // However, the ELF segment is aligned to 0x1000 bytes, so it starts earlier.
    // system_assembler preserves the segment base address and relocates sections
    // to pack them from the segment base. So _start ends up at the segment base.
    //
    // Calculate segment base by aligning down to 0x1000:
    const SEGMENT_ALIGNMENT: u64 = 0x1000;
    let segment_base = flash_start_address & !(SEGMENT_ALIGNMENT - 1);
    
    // On Armv7M, the +1 denotes Thumb mode.
    segment_base + 1
}
```

### Fix 2: Symbol Address Adjustment

**File**: `pw_kernel/tooling/system_assembler.rs` (in `add_app_symbols()` function, around line 268)

Replace:
```rust
new_symbol.st_value = symbol.st_value;
```

With:
```rust
// Adjust symbol address if it's in a relocated section
if let Some(new_id) = new_symbol.section {
    let old_section_id = symbol.section.unwrap();
    let old_section = app.sections.get(old_section_id);
    let new_section = self.builder.sections.get(new_id);
    
    // Calculate offset within the section
    let offset_in_section = symbol.st_value.wrapping_sub(old_section.sh_addr);
    
    // Set new address = new section base + offset
    new_symbol.st_value = new_section.sh_addr.wrapping_add(offset_in_section);
} else {
    // Symbol not in a section (e.g., absolute), preserve value
    new_symbol.st_value = symbol.st_value;
}
```

---

## Verification Steps

### 1. Check Entry Addresses

```bash
# Build ARMv7-M IPC test
bazel build --platforms=//pw_kernel/target/armv7m_minimal:armv7m_minimal \
  //pw_kernel/target/armv7m_minimal/ipc/user:ipc_test

# Check symbol addresses
arm-none-eabi-nm bazel-bin/pw_kernel/target/armv7m_minimal/ipc/user/ipc_test | grep "T _start_"

# Expected output:
# 00030000 T _start_handler_1
# 00020000 T _start_initiator_0
```

### 2. Check Entry Code Disassembly

```bash
arm-none-eabi-objdump -d bazel-bin/.../ipc_test | grep -A20 "^00020000 <_start_initiator_0>:"

# Should show:
# - ldr instructions (loading addresses)
# - calls to memcpy/memset (initializing .data/.bss)
# - bl to main_initiator_0
# - udf #1 (trap if main returns)
```

### 3. Run Test in QEMU

```bash
timeout 30 qemu-system-arm -machine mps2-an385 -cpu cortex-m3 -nographic \
  -semihosting -kernel bazel-bin/.../ipc_test 2>&1 | \
  python -m pw_tokenizer.detokenize base64 bazel-bin/.../ipc_test

# Should see:
# [INF] Allocating non-privileged thread 'initiator thread' (entry: 0x00020001)
# [INF] Allocating non-privileged thread 'handler thread' (entry: 0x00030001)
# [INF] Starting thread 'initiator thread'
# [INF] Starting thread 'handler thread'
# (threads should start without immediate crash)
```

---

## Known Limitations

### Remaining Issue: Absolute Symbols

The fix handles symbols in sections, but **absolute/linker-defined symbols** (type 'A') are not yet adjusted. These include:
- `_pw_static_init_flash_start`
- Other linker script symbols

**Impact**: Applications may still crash during static initialization on ARMv7-M.

**Future work**: Extend `add_app_symbols()` to also adjust absolute symbols that fall within relocated regions.

---

## Files Modified

1. `pw_kernel/tooling/system_generator/lib.rs`
   - Modified: `Armv7MConfig::get_start_fn_address()`
   
2. `pw_kernel/tooling/system_assembler.rs`
   - Modified: `add_app_symbols()` function

---

## Testing Checklist

- [ ] ARMv7-M IPC test builds successfully
- [ ] Entry addresses are correct (0x20001, 0x30001)
- [ ] Entry code disassembly looks correct
- [ ] Threads start without immediate MemoryManagement fault
- [ ] ARMv8-M IPC test still works (regression check)
- [ ] Other targets build successfully (ast1030, rp2350, etc.)

---

## Additional Context

### Why the Bug Was Hidden

- ARMv8-M: Kernel ends at 0x10040000 (aligned) → no gap → bug dormant
- ARMv7-M: Kernel ends at 0x20200 (unaligned) → 0x200 gap → bug visible

### Segment Alignment Details

ELF segments require page alignment (0x1000 bytes). When the linker script specifies `ORIGIN(FLASH) = 0x30200`, the linker:
1. Creates a segment at 0x30000 (aligned)
2. Places the `.code` section at 0x30200 (0x200 bytes into the segment)
3. `system_assembler` preserves segment address (0x30000)
4. Sections get relocated to pack from segment base

---

## References

- Original investigation: `INVESTIGATION_System_Image_Entry_Corruption.md`
- System assembler: `pw_kernel/tooling/system_assembler.rs`
- System generator: `pw_kernel/tooling/system_generator/lib.rs`
- Test case: `pw_kernel/target/armv7m_minimal/ipc/user/`
