# Phase 3: SyscallBuffer Cross-Process Access

## Objective

Verify that `SyscallBuffer::copy_into()` correctly copies data from the initiator's address space to the handler's buffer.

**This is the most likely failure point.**

---

## Relevant Code

**Location**: `pw_kernel/kernel/object/buffer.rs`

```rust
pub fn copy_into(&self, offset: usize, into_buffer: &mut SyscallBuffer) -> Result<usize> {
    // Permission check
    if !self.access_type.is_readable() || !into_buffer.access_type.is_writeable() {
        return Err(Error::PermissionDenied);
    }

    // Bounds check
    if offset > self.size {
        return Err(Error::OutOfRange);
    }

    let available_bytes = self.size - offset;
    let copy_len = min(available_bytes, into_buffer.size);

    // THE ACTUAL COPY - this may fail silently!
    unsafe {
        self.addr
            .byte_add(offset)
            .copy_to(into_buffer.addr, copy_len);
    }

    Ok(copy_len)
}
```

---

## What Could Go Wrong

### Issue 3.1: Permission Check Fails
- `self.access_type.is_readable()` returns false
- `into_buffer.access_type.is_writeable()` returns false

### Issue 3.2: Size Calculation Returns 0
- `self.size` is 0
- `offset >= self.size`
- `into_buffer.size` is 0

### Issue 3.3: Memory Access Fails (Most Likely)
- `self.addr` points to initiator's memory
- Handler's context may not have access to initiator's address space
- On PMSAv7, this could cause a MemoryManagement fault OR silent failure

### Issue 3.4: Address Space Mismatch
- Initiator's buffer virtual address not valid in handler's context
- Need to consider if kernel runs with different MPU config than user processes

---

## Test Strategy

### Test 3.1: Log All Parameters

```rust
pub fn copy_into(&self, offset: usize, into_buffer: &mut SyscallBuffer) -> Result<usize> {
    pw_log::debug!("copy_into: self.addr={:#x}, self.size={}, self.access={:?}",
        self.addr.as_ptr() as usize,
        self.size as u32,
        self.access_type);
    pw_log::debug!("copy_into: into.addr={:#x}, into.size={}, into.access={:?}",
        into_buffer.addr.as_ptr() as usize,
        into_buffer.size as u32,
        into_buffer.access_type);
    pw_log::debug!("copy_into: offset={}", offset as u32);

    if !self.access_type.is_readable() {
        pw_log::error!("copy_into: source not readable!");
        return Err(Error::PermissionDenied);
    }
    if !into_buffer.access_type.is_writeable() {
        pw_log::error!("copy_into: dest not writeable!");
        return Err(Error::PermissionDenied);
    }

    if offset > self.size {
        pw_log::error!("copy_into: offset > size!");
        return Err(Error::OutOfRange);
    }

    let available_bytes = self.size - offset;
    let copy_len = min(available_bytes, into_buffer.size);
    
    pw_log::debug!("copy_into: will copy {} bytes", copy_len as u32);

    unsafe {
        self.addr.byte_add(offset).copy_to(into_buffer.addr, copy_len);
    }

    pw_log::debug!("copy_into: copy complete");
    Ok(copy_len)
}
```

### Test 3.2: Verify Memory Access

Before the copy, try reading the source address:

```rust
// Debug: try to read first byte of source
let test_byte = unsafe { *self.addr.as_ptr() };
pw_log::debug!("copy_into: source[0]={:#x}", test_byte as u32);
```

If this causes a fault, we've found the issue.

### Test 3.3: Check Current Execution Context

```rust
// Check if we're in privileged mode
let control = cortex_m::register::control::read();
pw_log::debug!("copy_into: CONTROL.nPRIV={}", control.npriv() as u32);
```

---

## Key Questions

1. **Is the kernel privileged when handling syscalls?**
   - It should be - syscall handler runs in Handler mode
   
2. **Does PMSAv7 MPU allow privileged access to all memory?**
   - Need to check MPU configuration
   - Background region may be disabled

3. **Are user buffers in MPU-allowed regions?**
   - Initiator's send buffer must be accessible

---

## Memory Layout Reference

From `system.json5`:
```
Initiator RAM: 0x00080000 - 0x00088000 (32KB)
Handler RAM:   0x00088000 - 0x00090000 (32KB)
```

The initiator's send buffer is in **initiator's RAM** (0x00080000-0x00088000).
When handler's syscall runs, kernel needs to access initiator's RAM.

---

## Success Criteria

- [ ] All copy_into parameters logged correctly
- [ ] Source buffer readable (no MemFault)
- [ ] Copy length > 0
- [ ] Data actually transferred

## Findings

*(To be filled during investigation)*

---

## Next Phase

If SyscallBuffer works → [Phase 4: MPU Configuration](phase4-mpu-configuration.md)
If SyscallBuffer fails → Phase 4 becomes the fix phase
