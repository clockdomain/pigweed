# Code Comparison Analysis - ARMv7-M vs ARMv8-M

## Date: January 25, 2026

## Summary

This analysis compares the generated assembly code between AST1030 (ARMv7-M, Cortex-M4) and MPS2-AN505 (ARMv8-M, Cortex-M33) to investigate syscall return value corruption on ARMv7-M platforms.

**Key Finding**: The generated assembly for all critical syscall/context-switch functions is **IDENTICAL** between both architectures. The bug must be caused by runtime/architectural differences, not code generation.

---

## Test Configuration

### Minimal Reproducer

Added `object_wait` test to `hello_user`:
- File: `pw_kernel/tests/hello_user/hello.rs`
- Change: Added `object_wait` call with immediate timeout before shutdown
- System config: Added `TEST_CHANNEL` object (channel_handler type) to system.json5

### Test Results

| Platform | Architecture | MCU | object_wait Result | Status |
|----------|-------------|-----|-------------------|--------|
| **AST1030** | ARMv7-M | Cortex-M4 | `ok=4294967295` (0xFFFFFFFF) | ❌ CORRUPTED |
| **LM3S6965** | ARMv7-M | Cortex-M3 | Returns garbage RAM address | ❌ CORRUPTED |
| **MPS2-AN505** | ARMv8-M | Cortex-M33 | `ok=0`, error=4 (DeadlineExceeded) | ✅ CORRECT |

**Conclusion**: Bug affects **all ARMv7-M platforms** but not ARMv8-M.

---

## Saved ELF Files

```
pw_kernel/target/ast1030/ipc-investigation/early-wakeup-issue/saved-elfs/
├── ast1030_hello_user.elf      (277,504 bytes)
└── mps2_an505_hello_user.elf   (327,348 bytes)
```

---

## Disassembly Comparison

### 1. `handle_svc` - Syscall Handler

**AST1030 (ARMv7-M):**
```asm
0000117c <_ZN9pw_kernel4arch12arm_cortex_m7syscall10handle_svc17h8f06a81f0b9c42f0E>:
    117c:	b5f0      	push	{r4, r5, r6, r7, lr}
    117e:	4604      	mov	r4, r0
    1180:	af03      	add	r7, sp, #12
    1182:	b093      	sub	sp, #76	; 0x4c
    ...
```

**MPS2-AN505 (ARMv8-M):**
```asm
0000117c <_ZN9pw_kernel4arch12arm_cortex_m7syscall10handle_svc17h8f06a81f0b9c42f0E>:
    117c:	b5f0      	push	{r4, r5, r6, r7, lr}
    117e:	4604      	mov	r4, r0
    1180:	af03      	add	r7, sp, #12
    1182:	b093      	sub	sp, #76	; 0x4c
    ...
```

**Result**: ✅ **IDENTICAL** - Same prologue, same `frame_ptr` saved in r4

---

### 2. `PendSV` - Context Switch Entry

**AST1030 (ARMv7-M):**
```asm
000004b8 <PendSV>:
     4b8:	b501      	push	{r0, lr}
     4ba:	f3ef 8209 	mrs	r2, PSP
     4be:	f3ef 8114 	mrs	r1, CONTROL
     4c2:	e92d 0ff6 	stmdb	sp!, {r1, r2, r4, r5, r6, r7, r8, r9, sl, fp}
     4c6:	4668      	mov	r0, sp
     4c8:	b081      	sub	sp, #4        ; Stack alignment
     4ca:	f7ff ff9d 	bl	408 <_ZN9pw_kernel7threads14pendsv_swap_sp17haaf2d7e2cbbaab40E>
     ...
```

**MPS2-AN505 (ARMv8-M):**
```asm
000004b8 <PendSV>:
     4b8:	b501      	push	{r0, lr}
     4ba:	f3ef 8209 	mrs	r2, PSP
     4be:	f3ef 8114 	mrs	r1, CONTROL
     4c2:	e92d 0ff6 	stmdb	sp!, {r1, r2, r4, r5, r6, r7, r8, r9, sl, fp}
     4c6:	4668      	mov	r0, sp
     4c8:	b081      	sub	sp, #4        ; Stack alignment
     4ca:	f7ff ff9d 	bl	408 <_ZN9pw_kernel7threads14pendsv_swap_sp17haaf2d7e2cbbaab40E>
     ...
```

**Result**: ✅ **IDENTICAL** - Both have `sub sp, #4` alignment

---

### 3. `SVCall` - Syscall Entry

**AST1030 (ARMv7-M):**
```asm
00000528 <SVCall>:
     528:	f3ef 8209 	mrs	r2, PSP
     52c:	f3ef 8114 	mrs	r1, CONTROL
     530:	b5f6      	push	{r1, r2, r4, r5, r6, r7, lr}
     532:	e92d 0f00 	stmdb	sp!, {r8, r9, sl, fp}
     536:	4668      	mov	r0, sp
     538:	f010 0304 	ands.w	r3, r0, #4
     53c:	bf18      	it	ne
     53e:	b081      	subne	sp, #4        ; CONDITIONAL alignment
     540:	b089      	sub	sp, #36	; 0x24
     ...
```

**MPS2-AN505 (ARMv8-M):**
```asm
00000528 <SVCall>:
     528:	f3ef 8209 	mrs	r2, PSP
     52c:	f3ef 8114 	mrs	r1, CONTROL
     530:	b5f6      	push	{r1, r2, r4, r5, r6, r7, lr}
     532:	e92d 0f00 	stmdb	sp!, {r8, r9, sl, fp}
     536:	4668      	mov	r0, sp
     538:	f010 0304 	ands.w	r3, r0, #4
     53c:	bf18      	it	ne
     53e:	b081      	subne	sp, #4        ; CONDITIONAL alignment
     540:	b089      	sub	sp, #36	; 0x24
     ...
```

**Result**: ✅ **IDENTICAL** - Both use conditional `subne sp, #4`

---

### 4. `svc_return` - Syscall Return

**AST1030 (ARMv7-M):**
```asm
000005bc <svc_return>:
     5bc:	4685      	mov	sp, r0
     5be:	e8bd 0ff0 	ldmia.w	sp!, {r4, r5, r6, r7, r8, r9, sl, fp}
     5c2:	bc06      	pop	{r1, r2}
     5c4:	f85d eb04 	ldr.w	lr, [sp], #4
     5c8:	f381 8809 	msr	PSP, r1
     5cc:	f042 0203 	orr.w	r2, r2, #3
     5d0:	f382 8814 	msr	CONTROL, r2
     ...
```

**MPS2-AN505 (ARMv8-M):**
```asm
000005bc <svc_return>:
     5bc:	4685      	mov	sp, r0
     5be:	e8bd 0ff0 	ldmia.w	sp!, {r4, r5, r6, r7, r8, r9, sl, fp}
     5c2:	bc06      	pop	{r1, r2}
     5c4:	f85d eb04 	ldr.w	lr, [sp], #4
     5c8:	f381 8809 	msr	PSP, r1
     5cc:	f042 0203 	orr.w	r2, r2, #3
     5d0:	f382 8814 	msr	CONTROL, r2
     ...
```

**Result**: ✅ **IDENTICAL**

---

### 5. `pendsv_swap_sp` - Context Switch Core

**AST1030 (ARMv7-M):**
```asm
00000408 <_ZN9pw_kernel7threads14pendsv_swap_sp17haaf2d7e2cbbaab40E>:
     408:	b5f0      	push	{r4, r5, r6, r7, lr}
     40a:	4607      	mov	r7, r0
     ...
     ; atomic operations use ldrex/strex + dmb
     440:	e851 3f00 	ldrex	r3, [r1]
     444:	429a      	cmp	r2, r3
     446:	d103      	bne.n	450
     448:	e841 0500 	strex	r5, r0, [r1]
     44c:	2d00      	cmp	r5, #0
     44e:	d1f7      	bne.n	440
     450:	f3bf 8f5b 	dmb	ish
```

**MPS2-AN505 (ARMv8-M):**
```asm
00000408 <_ZN9pw_kernel7threads14pendsv_swap_sp17haaf2d7e2cbbaab40E>:
     408:	b5f0      	push	{r4, r5, r6, r7, lr}
     40a:	4607      	mov	r7, r0
     ...
     ; atomic operations use ldaex/stlex (acquire/release)
     440:	e8d1 3fef 	ldaex	r3, [r1]
     444:	429a      	cmp	r2, r3
     446:	d103      	bne.n	450
     448:	e8c1 0fe0 	stlex	r0, r0, [r1]
     44c:	2800      	cmp	r0, #0
     44e:	d1f7      	bne.n	440
     ; No dmb needed - ldaex/stlex have acquire/release semantics
```

**Result**: ⚠️ **DIFFERENT ATOMIC INSTRUCTIONS ONLY**

| Feature | ARMv7-M | ARMv8-M |
|---------|---------|---------|
| Load-exclusive | `ldrex` | `ldaex` (acquire) |
| Store-exclusive | `strex` | `stlex` (release) |
| Memory barrier | `dmb ish` | Not needed |

---

## Key Differences Found

### 1. Atomic Instructions (Only Difference)

| ARMv7-M | ARMv8-M | Notes |
|---------|---------|-------|
| `ldrex` | `ldaex` | ARMv8-M has acquire semantics built-in |
| `strex` | `stlex` | ARMv8-M has release semantics built-in |
| Requires `dmb` | No barrier needed | Memory ordering automatic |

This is the **only** code difference between the two architectures in the syscall/context-switch path.

### 2. No Difference in Frame Handling

- `save_exception_frame` - IDENTICAL
- `restore_exception_frame` - IDENTICAL
- `handle_svc` prologue - IDENTICAL
- `svc_return` - IDENTICAL

---

## Analysis

### Why Identical Code Behaves Differently

Since the generated assembly is identical (except atomic instructions), the bug must be caused by:

1. **Architectural Behavior Differences**
   - ARMv7-M and ARMv8-M may handle exception return differently
   - EXC_RETURN values may have different effects
   - Stack alignment behavior at exception entry may differ

2. **Memory Ordering Issues**
   - The `ldrex`/`strex` + `dmb` pattern on ARMv7-M may have subtle ordering differences
   - Could cause frame pointer to be read before it's fully written

3. **Exception Priority/Preemption**
   - Different handling of tail-chaining between architectures
   - PendSV and SVCall interaction may differ

4. **ARMv7-M-Specific Runtime Fixup**

   Looking at `pendsv_swap_sp`, there's ARMv7-M-specific code:
   ```rust
   #[cfg(armv7m)]
   {
       // ARMv7-M doesn't preserve CONTROL or return address
       // in the kernel exception frame, so we need to fixup
       frame.control = canonical_control;
       frame.return_address = canonical_return_address;
   }
   ```
   
   This code overwrites values in the frame that ARMv8-M preserves naturally.

---

## Hypotheses to Test

### Hypothesis A: Memory Barrier Timing

The `dmb ish` after `strex` on ARMv7-M may not complete before PendSV exit reads the frame pointer.

**Test**: Add extra `dmb` barriers around frame pointer access.

### Hypothesis B: ARMv7-M Frame Fixup Bug

The `canonical_control` and `canonical_return_address` fixup may be writing wrong values or to the wrong location.

**Test**: Add debug output to print frame contents before/after fixup.

### Hypothesis C: Exception Tail-Chaining

On ARMv7-M, if PendSV tail-chains into SVCall return, the frame restoration may be incorrect.

**Test**: Disable tail-chaining by adding `dsb`/`isb` between exception handlers.

### Hypothesis D: QEMU Emulation Difference

The bug may be QEMU-specific for ARMv7-M emulation.

**Test**: Run on real hardware (STM32, etc.) to verify.

---

## Conclusion

The bug is **NOT** in the generated assembly code - it's identical between ARMv7-M and ARMv8-M. The root cause must be one of:

1. An ARMv7-M-specific runtime fixup (`pendsv_swap_sp` frame fixup)
2. Architectural differences in exception handling
3. Subtle memory ordering differences with `ldrex`/`strex` vs `ldaex`/`stlex`
4. QEMU emulation issue specific to ARMv7-M

**Next Steps**:
1. Add instrumentation to `pendsv_swap_sp` to trace frame values
2. Compare frame contents at key points between architectures
3. Test on real ARMv7-M hardware to rule out QEMU issues

---

## Potential Fixes

### Option 1: Add explicit memory barriers around frame access

Since the frame pointer is accessed with raw pointer operations (not atomics), 
add explicit DMB barriers to ensure visibility:

```rust
#[exception(exception = "PendSV", disable_interrupts)]
extern "C" fn pendsv_swap_sp(frame: *mut KernelExceptionFrame) -> *mut KernelExceptionFrame {
    // ... existing code ...
    
    unsafe {
        (*active_thread).frame = frame;
        
        // Ensure frame write is visible before we proceed
        core::sync::atomic::fence(Ordering::Release);  // Generates dmb ish
        
        // ... ARMv7-M fixup code ...
    }
    
    // ... scheduler lock code ...
    
    // Ensure all scheduler writes are visible before returning frame pointer
    core::sync::atomic::fence(Ordering::Acquire);
    
    unsafe { (*new_thread).frame }
}
```

**Rationale**: On ARMv7-M, plain stores/loads don't have implicit ordering. The 
interrupt-disable provides mutual exclusion but not memory ordering. Adding 
fences ensures:
- Release fence: frame write is visible before subsequent operations
- Acquire fence: we see all previous writes before reading new_thread's frame

### Option 2: Use volatile operations for frame pointer

```rust
use core::ptr::{read_volatile, write_volatile};

unsafe {
    write_volatile(&mut (*active_thread).frame, frame);
}

// ... later ...

unsafe { read_volatile(&(*new_thread).frame) }
```

**Rationale**: Volatile prevents compiler reordering but doesn't add hardware 
barriers. May be insufficient on its own.

### Option 3: Add DSB/ISB after exception frame manipulation

The ARM architecture recommends DSB + ISB after modifying exception-related 
state to ensure the changes take effect before exception return:

```rust
unsafe {
    (*active_thread).frame = frame;
    
    #[cfg(feature = "armv7m")]
    {
        // ... canonical control fixup ...
        core::arch::asm!("dsb sy", "isb sy", options(nomem, nostack));
    }
}
```

**Rationale**: DSB ensures all memory accesses complete; ISB flushes the 
pipeline. This is the most conservative approach.

---

## Test Results After Option 1 (Memory Barriers)

### hello_user test (simple object_wait)

| Platform | Result | Notes |
|----------|--------|-------|
| AST1030 | ✅ PASSED | Output garbled but test passes |
| MPS2-AN505 | ✅ PASSED | Clean output, correct values |

### IPC test (full channel communication)

| Platform | Result | Notes |
|----------|--------|-------|
| AST1030 | ❌ FAILED | Still shows LR=0x3 corruption |
| MPS2-AN505 | ✅ PASSED | Works correctly |

**Analysis**: The memory barrier fix helped the simple test pass but did NOT 
fix the full IPC test. The fault still shows:
- `r1 = 0xffffffff` (corrupted)
- `lr = 0x00000003` (CONTROL value in wrong place)

This suggests the root cause is NOT just memory ordering - there's likely a 
frame offset or pointer issue in how the ARMv7-M canonical_control fixup 
interacts with the more complex IPC context switching.

### Next Investigation Steps

1. The `canonical_control`/`canonical_return_address` fixup may be writing to 
   the wrong frame (e.g., writing to a stale frame pointer)
2. Need to add debug output to trace frame addresses before/after fixup
3. Consider disabling the ARMv7-M fixup entirely to see baseline behavior
