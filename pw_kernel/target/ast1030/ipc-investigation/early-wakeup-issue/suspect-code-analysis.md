# Suspect Code Analysis - Frame Pointer Corruption

## Summary

The frame pointer passed to `svc_return` is **0x20 bytes (32 bytes) off**. The valid frame is below where MSP points.

## Suspect #1: Stack Alignment in SVCall

**File**: `pw_kernel/arch/arm_cortex_m/syscall.rs` (lines 116-119)

```rust
// SVCall handler assembly:
"
    // r0 holds the address of the KernelExceptionFrame
    mov     r0, sp

    // Arm exception frames need to be aligned to 8 bytes
    ands    r3, r0, #0x4
    it ne
    subne   sp, 4       ← SP adjusted, but r0 is NOT updated!
"
```

### Problem

1. Frame is pushed, `r0 = sp` (correct frame address)
2. If SP is misaligned, `sp -= 4` to align
3. **r0 still points to the old (pre-alignment) address**
4. Fake exception frame is pushed at new aligned SP
5. When `handle_svc` returns, r0 is passed to `svc_return`

### Analysis

This alignment code looks suspicious, but `r0` is set BEFORE the alignment and isn't modified after. The frame pointer should be correct.

**However**, if a context switch happens during syscall processing, the frame could be saved/restored incorrectly.

## Suspect #2: PendSV Frame Corruption

**File**: `pw_kernel/arch/arm_cortex_m/threads.rs` (lines 488-501)

```rust
#[cfg(all(feature = "user_space", feature = "armv7m"))]
{
    let saved_frame = &mut *(*active_thread).frame;
    saved_frame.control = (*active_thread).canonical_control;
    saved_frame.return_address = (*active_thread).canonical_return_address;
}
```

### Problem

PendSV overwrites CONTROL and EXC_RETURN in the saved frame with "canonical" values. If:
1. PendSV fires during syscall processing
2. The wrong frame address is saved to `active_thread.frame`
3. PendSV writes canonical values to the wrong location

This could explain why we see CONTROL (0x3) appearing in wrong frame slots.

## Suspect #3: handle_svc Return Value

**File**: `pw_kernel/arch/arm_cortex_m/syscall.rs` (lines 254-272)

```rust
extern "C" fn handle_svc(frame_ptr: *mut KernelExceptionFrame) -> *mut KernelExceptionFrame {
    let frame = unsafe { &mut *frame_ptr };
    // ... syscall handling ...
    frame_ptr  // Returns same pointer it received
}
```

### Analysis

`handle_svc` returns the same `frame_ptr` it was given. If input is correct, output is correct.

**But**: During syscall processing (especially `object_wait` which can block), a context switch may occur. When the thread resumes, does it get the correct frame pointer back?

## Suspect #4: Context Switch Frame Save

**File**: `pw_kernel/arch/arm_cortex_m/threads.rs` (line 486)

```rust
(*active_thread).frame = frame;  // Save incoming frame
```

### Question

When PendSV fires, it saves `frame` to `active_thread.frame`. But what is `frame` at this point?

- If PendSV fires during syscall processing, `frame` might be the **syscall's kernel frame**
- When thread resumes, it restores from this saved frame
- If the save/restore is off by one frame, we get the 0x20 byte offset

## Key Evidence

| Observation | Implication |
|-------------|-------------|
| Frame 0x20 bytes off | Exactly one KernelExceptionFrame size |
| LR=0x3 in user frame | CONTROL value leaked into LR slot |
| Kernel pointers in user regs | Restoring from wrong frame |
| Only happens with blocking syscalls | Context switch involvement |

## Next Steps

1. **Add logging to PendSV**: Print frame address when saving/restoring
2. **Trace frame pointer**: From SVCall entry → handle_svc → svc_return
3. **Check if PendSV fires**: During object_wait syscall processing
4. **Compare working vs broken**: What's different about blocking syscalls?

## Working Hypothesis

During a blocking syscall (object_wait):
1. SVCall builds frame at address X
2. Syscall blocks, triggers context switch
3. PendSV saves wrong frame address (X + 0x20) to thread state
4. Thread resumes, restores from X + 0x20
5. User gets garbage (kernel data from adjacent frame)
