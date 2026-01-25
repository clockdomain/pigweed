# Gemini Assessment: Early Wakeup / Syscall Corruption Bug

## Agreement with Investigation

I agree with the analysis in `code-comparison.md`. The finding that the generated assembly for `handle_svc`, `PendSV`, and `SVCall` is identical between ARMv7-M and ARMv8-M is crucial. It confirms that the issue is not a compiler bug or a divergence in the low-level assembly generation.

However, I can confirm that **Hypothesis B: ARMv7-M Frame Fixup Bug** is the correct root cause. The mechanism of failure is a **Struct Layout Mismatch** that occurs only in the ARMv7-M specific path.

## Root Cause Analysis: Struct Layout Mismatch

The bug is caused by the `#[cfg(armv7m)]` fixup block in `pendsv_swap_sp` (in `threads.rs`) treating the stack frame pointer as a `KernelExceptionFrame` struct, when the actual stack layout created by the `PendSV` assembly handler does not match that struct.

### 1. The Mismatch

**Rust Struct Layout (`KernelExceptionFrame`)**
Defined in `pw_kernel/arch/arm_cortex_m/exceptions.rs` with `#[repr(C)]`.
*   Offset 0x00: `r4`
*   ...
*   Offset 0x20: `psp` (32)
*   Offset 0x24: `control` (36)
*   Offset 0x28: `return_address` (40)

**PendSV Stack Layout (ARMv7-M Assembly)**
Created by `stmdb sp!, {r1, r2, r4-r11}` followed by `push {r0, lr}`. The `stmdb` instruction stores registers in descending order (highest register at highest address).
*   Offset 0x00: `r0` (Pushed by `push {r0, lr}`)
*   Offset 0x04: `lr` (Pushed by `push {r0, lr}`)
*   Offset 0x08: `r1` (CONTROL)
*   Offset 0x0c: `r2` (PSP)
*   Offset 0x10: `r4`
*   ...
*   Offset 0x24: `r9`
*   Offset 0x28: `r10`
*   Offset 0x2c: `r11` (Frame Pointer)

**Wait, let's re-verify the `PendSV` push order.**
`stmdb sp!, {r1, r2, r4, r5, r6, r7, r8, r9, sl, fp}` writes:
- Top address: `fp` (r11)
- ...
- Bottom address: `r1`

Then `push {r0, lr}` pushes `lr` then `r0`.
So the stack looks like:
- High Addr -> `fp` (r11)
- ...
- `r1`
- `lr`
- Low Addr -> `r0`

Wait, let's look at the disassembly provided in `code-comparison.md` again.
```asm
4c2:	e92d 0ff6 	stmdb	sp!, {r1, r2, r4, r5, r6, r7, r8, r9, sl, fp}
4c6:	4668      	mov	r0, sp
4c8:	b081      	sub	sp, #4        ; Stack alignment
4ca:	f7ff ff9d 	bl	408
```
It pushes `r1, r2, r4-r11`.
Then it passes `sp` to the function.
Then it aligns the stack.

The `KernelExceptionFrame` struct expects:
`r4, r5, r6, r7, r8, r9, r10, r11, psp, control, return_address` (if we look at `SVCall` usage).

The `PendSV` pushes `r1(CONTROL), r2(PSP), r4..r11`.
`stmdb` pushes highest register to highest address.
So `SP` points to `r1`.
`SP+0`: `r1` (CONTROL)
`SP+4`: `r2` (PSP)
`SP+8`: `r4`
...
`SP+36`: `r11`

The Rust struct `KernelExceptionFrame` expects:
`r4` at offset 0.

**The layout is completely inverted/shuffled compared to what the Rust struct expects from the `SVCall` context.**

### 2. The Corruption Mechanism

When `pendsv_swap_sp` executes the ARMv7-M fixup:

```rust
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    let saved_frame = &mut *(*active_thread).frame;
    saved_frame.control = (*active_thread).canonical_control;
    saved_frame.return_address = (*active_thread).canonical_return_address;
}
```

It calculates the address of `control` and `return_address` based on the `KernelExceptionFrame` struct definition.

*   `control` is at offset **36 (0x24)** in the struct (after r4-r11 + psp).
*   `return_address` is at offset **40 (0x28)**.

In the `PendSV` stack frame:
*   Offset **36** corresponds to **`r11` (Frame Pointer)** (or similar high register depending on exact packing).
*   Offset **40** corresponds to **`r0`** (if pushed) possibly?

Actually, let's look at `PendSV` again carefully.
```asm
4b8:	b501      	push	{r0, lr}
4ba:	f3ef 8209 	mrs	r2, PSP
4be:	f3ef 8114 	mrs	r1, CONTROL
4c2:	e92d 0ff6 	stmdb	sp!, {r1, r2, r4, r5, r6, r7, r8, r9, sl, fp}
```
Stack after `push {r0, lr}`:
`SP_old - 4`: `lr`
`SP_old - 8`: `r0` (Current SP)

Stack after `stmdb`:
It pushes `fp, sl, r9, r8, r7, r6, r5, r4, r2, r1` downwards.
`SP`: `r1` (CONTROL)
`SP+4`: `r2` (PSP)
`SP+8`: `r4`
...
`SP+36`: `fp` (r11)

Wait, `KernelExceptionFrame` defines `r4` as the *first* element.
So `(*active_thread).frame` points to `SP` which holds `r1`.
Rust thinks `SP` points to `r4`.

Rust writes to `saved_frame.return_address` (Offset 40 / 0x28).
`SP + 40` points to...
`SP + 36` is `fp`.
`SP + 40` is the `r0` pushed at the very beginning by `push {r0, lr}`!

**The Corruption:**
*   Rust writes `canonical_return_address` (0xFFFFFFFD for user threads) to Offset 40.
*   Offset 40 on the stack is **`r0`**.
*   **`r0` holds the return value of the syscall** (e.g., the result of `object_wait`).

This perfectly explains why the syscall returns `0xFFFFFFFD` (or similar large negative number) instead of the actual result. The fixup code overwrites the syscall return value register on the stack.

## Why ARMv8-M Works
The ARMv8-M build creates `pendsv_swap_sp` **without the fixup block**. It relies on hardware stacking or different context preservation, so it never performs these incorrect writes that corrupt the stack frame.

## Conclusion
The bug is a logic error in `pw_kernel/arch/arm_cortex_m/threads.rs`. The code assumes it can interpret the `PendSV` stack frame using the `KernelExceptionFrame` struct, but `PendSV` constructs a stack frame that is incompatible with that struct layout. On ARMv7-M, the manual fixup code blindly writes into this incompatible frame, overwriting the syscall return value stored in `r0`.
