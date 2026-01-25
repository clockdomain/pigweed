# Phase 4: MPU Configuration Analysis

## Objective

Analyze and potentially fix the PMSAv7 MPU configuration to allow kernel-mode access to user-space memory across process boundaries.

---

## PMSAv7 MPU Background

### Key Characteristics

1. **8 regions** (typical for Cortex-M3/M4)
2. **Region size**: Must be power-of-2, minimum 32 bytes
3. **Region alignment**: Must be aligned to region size
4. **Subregions**: Each region can disable 8 equal subregions
5. **Privileged vs Unprivileged**: Separate access permissions

### Access Permission Encoding (AP bits)

| AP | Privileged | Unprivileged |
|----|------------|--------------|
| 000 | No access | No access |
| 001 | RW | No access |
| 010 | RW | RO |
| 011 | RW | RW |
| 101 | RO | No access |
| 110 | RO | RO |
| 111 | RO | RO |

### Background Region (PRIVDEFENA)

- If enabled: Privileged code can access any memory not covered by MPU regions
- If disabled: All accesses must match an MPU region
- **Critical for IPC**: If disabled and no region covers initiator's RAM, access fails

---

## Current MPU Configuration

**Location**: `pw_kernel/arch/arm_cortex_m/src/armv7m/mpu.rs`

Need to examine:
1. Is PRIVDEFENA enabled?
2. What regions are configured for each process?
3. Do regions allow privileged access to other process memory?

---

## Investigation Steps

### Step 4.1: Dump MPU Configuration

Add code to dump MPU state during syscall:

```rust
fn dump_mpu_config() {
    let mpu = unsafe { &*cortex_m::peripheral::MPU::PTR };
    
    pw_log::debug!("MPU CTRL: {:#010x}", mpu.ctrl.read());
    pw_log::debug!("  ENABLE={}, PRIVDEFENA={}, HFNMIENA={}",
        (mpu.ctrl.read() >> 0) & 1,
        (mpu.ctrl.read() >> 2) & 1,  // Background region enable
        (mpu.ctrl.read() >> 1) & 1);
    
    let num_regions = (mpu.type_.read() >> 8) & 0xFF;
    pw_log::debug!("MPU regions: {}", num_regions);
    
    for i in 0..num_regions {
        mpu.rnr.write(i);
        let rbar = mpu.rbar.read();
        let rasr = mpu.rasr.read();
        
        if rasr & 1 != 0 {  // Region enabled
            let base = rbar & !0x1F;
            let size = 1 << (((rasr >> 1) & 0x1F) + 1);
            let ap = (rasr >> 24) & 0x7;
            let xn = (rasr >> 28) & 1;
            
            pw_log::debug!("  Region {}: base={:#010x}, size={:#x}, AP={}, XN={}",
                i, base, size, ap, xn);
        }
    }
}
```

### Step 4.2: Check PRIVDEFENA Status

```rust
// In syscall handler, check if background region is enabled
let mpu_ctrl = unsafe { (*cortex_m::peripheral::MPU::PTR).ctrl.read() };
let privdefena = (mpu_ctrl >> 2) & 1;
pw_log::debug!("PRIVDEFENA={}", privdefena);
```

### Step 4.3: Check Memory Region Coverage

For the IPC to work, when handler's syscall runs:
- Handler's RAM region is configured (for handler's buffers)
- **Initiator's RAM region must also be accessible** (for initiator's send buffer)

---

## Potential Fixes

### Fix 4.1: Enable PRIVDEFENA

If background region is disabled, enable it:

```rust
unsafe {
    let mpu = &*cortex_m::peripheral::MPU::PTR;
    let ctrl = mpu.ctrl.read();
    mpu.ctrl.write(ctrl | (1 << 2));  // Set PRIVDEFENA
}
```

**Risk**: This gives privileged code access to all memory, reducing isolation.

### Fix 4.2: Keep All User Regions Active

Instead of swapping MPU regions on context switch, keep all user regions active with different permissions:
- Current process: Unprivileged RW
- Other processes: Privileged RW only (unprivileged no access)

**Benefit**: Kernel can always access any user memory.
**Cost**: Uses more MPU regions.

### Fix 4.3: Kernel Memory Access Window

Create a temporary MPU region for cross-process access:

```rust
fn with_cross_process_access<F, R>(addr: usize, size: usize, f: F) -> R 
where F: FnOnce() -> R 
{
    // Save current MPU region 7
    // Configure region 7 to cover addr..addr+size with privileged RW
    // Execute f()
    // Restore region 7
}
```

### Fix 4.4: Use MPU Background Region Only During Syscalls

```rust
fn enter_syscall() {
    // Enable PRIVDEFENA
}

fn exit_syscall() {
    // Disable PRIVDEFENA
}
```

---

## ARMv8-M vs ARMv7-M Comparison

| Feature | ARMv7-M (PMSAv7) | ARMv8-M (PMSAv8) |
|---------|------------------|------------------|
| Regions | 8 | 8-16 |
| Min size | 32 bytes | 32 bytes |
| Alignment | Power-of-2 | Any (start/end aligned to 32) |
| Background region | PRIVDEFENA | PRIVDEFENA |

The LM3S6965 test (also ARMv7-M) should have the same issue if it exists.

---

## Validation

### Check if LM3S6965 IPC Works

```bash
bazel test --config=k_qemu_lm3s6965 //pw_kernel/target/lm3s6965/ipc/user:ipc_test
```

If LM3S6965 passes, compare its MPU configuration to AST1030's.

---

## Success Criteria

- [ ] MPU configuration understood and documented
- [ ] Root cause of access failure identified
- [ ] Fix implemented without breaking security model
- [ ] IPC test passes

## Findings

*(To be filled during investigation)*

---

## Next Phase

→ [Phase 5: End-to-End Fix](phase5-end-to-end.md)
