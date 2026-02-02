# PendSV/SVCall Priority Corruption Scenario

This document describes a corruption scenario that can occur with pw_kernel's
Cortex-M exception priority configuration.

## Priority Configuration

pw_kernel configures exception priorities as follows:

| Exception     | Value | Top 2 bits | Effective Priority |
|---------------|-------|------------|-------------------|
| External IRQs | 0x40  | 0b01       | 1 (highest)       |
| SysTick       | 0x7F  | 0b01       | 1 (highest)       |
| PendSV        | 0xBF  | 0b10       | 2                 |
| SVCall        | 0xFF  | 0b11       | 3 (lowest)        |

The key issue: **PendSV has higher priority than SVCall**.

## The Corruption Scenario

### Setup

- Thread A is running in Thread mode
- No exceptions are pending

### Sequence of Events

```
1. Thread A executes SVC instruction
   ├─ Processor enters SVCall handler (Handler mode)
   └─ SVCall handler begins executing

2. SVCall handler reaches `cpsie i` (line 162 in syscall.rs)
   └─ Interrupts are now enabled

3. SysTick fires (priority 0x7F > SVCall's 0xFF)
   ├─ Processor preempts SVCall handler
   ├─ SVCall's partial state is pushed to stack
   └─ SysTick handler begins executing

4. SysTick handler determines Thread B should run
   ├─ Calls context_switch()
   ├─ Sets active_thread = Thread A
   └─ Sets PendSV pending

5. SysTick handler returns
   ├─ Processor checks for pending exceptions
   ├─ PendSV is pending with priority 0xBF
   ├─ SVCall handler has priority 0xFF
   └─ Since 0xBF > 0xFF: PendSV TAIL-CHAINS (does not return to SVCall)

6. PendSV handler executes
   ├─ Thinks it's saving Thread A's normal context
   ├─ Actually saves: mid-SVCall handler state
   │   - PC pointing into SVCall handler code
   │   - LR with partial EXC_RETURN value
   │   - Stale register values
   └─ Switches to Thread B

7. Later: Thread A is scheduled to run again
   ├─ PendSV restores the corrupted context
   ├─ Execution resumes INSIDE SVCall handler
   └─ With stale/invalid state
```

### Why Tail-Chaining Matters

When SysTick returns at step 5, the processor must decide where to go:

- **Without PendSV pending**: Return to preempted SVCall handler
- **With PendSV pending**: Compare priorities
  - PendSV (0xBF) has higher priority than SVCall (0xFF)
  - Tail-chain directly to PendSV instead of returning to SVCall

This is the critical detail: PendSV doesn't wait for SVCall to complete.
It tail-chains immediately because it has higher priority.

## The Vulnerable Window

The SVCall handler builds a fake exception frame on the stack to implement the
syscall trampoline. After the frame is complete, it re-enables interrupts
**while still in Handler mode**:

```asm
// In syscall.rs SVCall handler (simplified):

// Build fake exception frame for trampoline...
ldr     r5, =svc_return      // LR value in fake frame
ldr     r6, =handle_svc      // PC value in fake frame (where to "return" to)
push    {{ r4-r6 }}          // Push as r12, lr, pc
push    {{ r0-r3 }}          // Push r0-r3

// Comment from code: "Reenable interrupts now that the exception
// stack state is coherent."
cpsie i                      // <-- Interrupts enabled, STILL IN HANDLER MODE

// Return from exception into handle_svc() in Thread mode
ldr lr, ={exc_return}        // <-- SysTick can preempt here
bx lr                        // <-- Or here (exception return → Thread mode)
```


The `cpsie i` creates a window where
we're in Handler mode with interrupts enabled.

If SysTick fires in this window and sets PendSV pending, PendSV will tail-chain
(because PendSV priority 0xBF > SVCall priority 0xFF) and save the mid-handler
state before the `bx lr` can return to Thread mode.

The window is small (2-3 instructions) but the corruption is severe.

## What Gets Corrupted

When Thread A resumes with the corrupted context:

| Register | Expected Value | Actual Value |
|----------|---------------|--------------|
| PC | User code address | SVCall handler address |
| LR | User return address | EXC_RETURN or garbage |
| r0 | Syscall result | Stale KernelExceptionFrame pointer |
| SP | User stack pointer | Possibly handler stack |

### The LR / EXC_RETURN Problem

When an exception occurs on Cortex-M, the processor loads LR with a special
**EXC_RETURN** value (not a normal return address). These are magic values:

| Value        | Meaning |
|--------------|---------|
| 0xFFFFFFF1   | Return to Handler mode, use MSP |
| 0xFFFFFFF9   | Return to Thread mode, use MSP |
| 0xFFFFFFFD   | Return to Thread mode, use PSP |

In the SVCall handler, LR goes through several states:

```asm
# SVCall entry - LR = EXC_RETURN (e.g., 0xFFFFFFFD)
# This tells processor "return to Thread A using PSP"

...handler code...

cpsie i                    # LR still = original EXC_RETURN
ldr lr, ={fake_exc_return} # LR = new fake EXC_RETURN for trampoline
bx lr                      # Use it to return
```

If PendSV preempts in that window (before `bx lr` completes), the saved state
includes:

1. **PC pointing into SVCall handler code** (Handler mode code)
2. **LR containing EXC_RETURN** (either original or fake trampoline value)

Note: `handle_svc()` runs in Thread mode, but **the corruption happens before
we ever reach handle_svc**. PendSV preempts the SVCall handler while it's
still in Handler mode, before the trampoline return (`bx lr`) can execute.

When Thread A is "restored" later:

- PendSV returns to Thread mode (normal exception return)
- PC points into SVCall handler code
- LR contains an EXC_RETURN value
- We're now executing Handler mode code in **Thread mode context**

If that code later executes `bx lr` with the EXC_RETURN value, it will
attempt an exception return from Thread mode, which is undefined behavior.

## Consequences

1. **Wrong execution context**: Thread resumes in Handler mode code while in Thread mode
2. **Invalid memory access**: Stale frame pointers reference wrong memory
3. **Syscall corruption**: Original syscall never completes properly
4. **Unpredictable behavior**: Could crash, hang, or silently corrupt data

## Diagram

```
    Thread A          SVCall           SysTick          PendSV
        │
        │ SVC
        ├────────────────►│
        │                 │ cpsie i
        │                 │◄────────────────┐
        │                 │   (preempted)   │ SysTick fires
        │                 │                 │
        │                 │                 │ context_switch()
        │                 │                 │ pend PendSV
        │                 │                 │
        │                 │    ┌────────────┤ return
        │                 │    │ tail-chain │
        │                 │    │            │
        │                 │    │            ├────────────────►│
        │                 │    │            │                 │
        │                 │    │            │                 │ save "Thread A"
        │                 │    │            │                 │ (actually SVCall state!)
        │                 │    │            │                 │
        │   NEVER RETURNS │    │            │                 │ switch to Thread B
        │                 ▼    ▼            │                 │
                    (abandoned)             │                 ▼
```

## Recommended Fixes

### Fix 1: PendSV Lowest Priority

Set PendSV to the lowest priority (0xFF), as recommended by ARM:

> "PendSV is an interrupt-driven request for system-level service. In an OS
> environment, use PendSV for context switching when no other exception is
> active."
>
> — ARM Cortex-M3 Devices Generic User Guide (ARM DUI0552A)

This ensures PendSV only runs after all other handlers complete, eliminating
the tail-chaining corruption scenario.

### Fix 2: Deferred Interrupt Enable (Implemented)

Move the `cpsie i` from the SVCall handler to `handle_svc()`. Since
`handle_svc()` runs in Thread mode (after exception return), there's no
Handler mode window where PendSV could tail-chain.

**Changes made to `syscall.rs`:**
- Removed `cpsie i` from SVCall handler (after building fake frame)
- Added `cpsie i` as first instruction in `handle_svc()`

This provides defense in depth: even if PendSV priorities were misconfigured,
no vulnerability window exists because interrupts remain disabled until
Thread mode.

See [deferred-interrupt-enable.md](deferred-interrupt-enable.md) for details.

## References

- ARM Cortex-M3 Devices Generic User Guide (ARM DUI0552A) - PendSV recommendation
- ARMv7-M Architecture Reference Manual, Section B1.5.4 - Exception return and tail-chaining
- `pw_kernel/arch/arm_cortex_m/threads.rs` - Priority configuration
- `pw_kernel/arch/arm_cortex_m/syscall.rs` - SVCall handler with vulnerable window
