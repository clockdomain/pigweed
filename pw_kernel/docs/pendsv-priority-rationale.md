# PendSV and SVCall Priority Configuration

This document explains the relationship between PendSV and SVCall exception
priorities in pw_kernel, and why the syscall trampoline design affects the
optimal priority assignment.

## Current Configuration (Main Branch)

```rust
scb.set_priority(scb::SystemHandler::SVCall, 0b1111_1111);  // 0xFF (lowest)

// Set PendSV (used by context switching) to just above SVCall so
// that system calls can context switch.
scb.set_priority(scb::SystemHandler::PendSV, 0b1011_1111);  // 0xBF (higher)
```

The comment suggests PendSV needs higher priority than SVCall "so that system
calls can context switch." However, due to pw_kernel's syscall trampoline
design, this priority relationship is not actually required.

## The Syscall Trampoline Design

pw_kernel uses a [trampoline](syscall-trampoline.md) where the SVCall
handler returns immediately and actual syscall processing happens in
Thread mode:

```
Thread A (Thread mode)
    │
    │ SVC instruction
    ▼
SVCall handler (Handler mode)
    │
    │ 1. Push fake exception frame to kernel stack
    │ 2. Load crafted EXC_RETURN into LR
    │ 3. bx lr (return immediately)
    ▼
handle_svc() (Thread mode!)  ◄── Syscall processing happens HERE
    │
    │ Process syscall...
    │ If blocking: set PendSV pending
    ▼
PendSV fires (Handler mode)
    │
    │ Context switch to Thread B
    ▼
Thread B runs
```

The SVCall handler does minimal work - it sets up a trampoline and returns.
The actual syscall processing happens in `handle_svc()` which runs in
**Thread mode**.

## Why Priority Doesn't Affect Syscall Context Switches

Since `handle_svc()` runs in Thread mode (not Handler mode), PendSV can
always preempt it regardless of their relative exception priorities.

Exception priorities determine:
- Which handler runs first when multiple exceptions are pending
- Whether one handler can preempt another handler

Thread mode code can be preempted by any exception, regardless of priority.

```
┌─────────────────────────────────────────────────────────────┐
│                     Exception Priorities                     │
│                                                              │
│   Relevant for Handler mode preemption:                     │
│   - Can SysTick preempt SVCall handler? (Yes, if higher)    │
│   - Can PendSV preempt SVCall handler? (Yes, if higher)     │
│                                                              │
│   Not relevant for Thread mode:                              │
│   - Can PendSV preempt handle_svc()? ALWAYS YES             │
│     (Thread mode can be preempted by any exception)          │
└─────────────────────────────────────────────────────────────┘
```

## Potential Issue with Current Configuration

With PendSV having higher priority than SVCall, PendSV can preempt the
SVCall handler itself (not just `handle_svc()`). This creates a potential
issue:

1. Thread A executes SVC
2. SVCall handler runs, enables interrupts (`cpsie i`)
3. SysTick fires, preempts SVCall, sets PendSV pending
4. SysTick returns
5. PendSV tail-chains (because PendSV priority > SVCall priority)
6. PendSV saves what it thinks is Thread A's context - but actually saves
   the mid-SVCall handler state
7. Thread A is later restored with inconsistent context

See [corruption-scenario.md](corruption-scenario.md) for detailed analysis.

## Recommended Configuration

Setting PendSV to lower or equal priority than SVCall avoids this issue:

```rust
// PendSV at LOWEST priority - cannot preempt any handler
scb.set_priority(scb::SystemHandler::PendSV, 0b1111_1111);  // 0xFF

// SVCall above PendSV - syscalls complete without context switch interruption
scb.set_priority(scb::SystemHandler::SVCall, 0b1011_1111);  // 0xBF
```

This ensures:
1. PendSV cannot preempt the SVCall handler
2. PendSV can still preempt `handle_svc()` (because it's Thread mode)
3. Context switches only happen when all handlers have completed

## Summary

| Priority Configuration | Syscall Context Switch Works? | Handler Preemption Risk |
|------------------------|------------------------------|------------------------|
| PendSV > SVCall | Yes | Yes - PendSV can preempt SVCall handler |
| PendSV ≤ SVCall | Yes | No - SVCall handler completes first |

Both configurations allow syscall context switches because `handle_svc()`
runs in Thread mode. The difference is whether PendSV can preempt the
SVCall handler itself.

## Standard RTOS Practice

Most RTOSes (FreeRTOS, Zephyr, ThreadX) set PendSV to the lowest priority.
ARM's official documentation recommends this configuration:

> "PendSV is an interrupt-driven request for system-level service. In an OS
> environment, use PendSV for context switching when no other exception is
> active."
>
> — ARM Cortex-M3 Devices Generic User Guide (ARM DUI0552A)

This ensures context switches only occur when all other exception processing
has completed.

## References

- ARM Cortex-M3 Devices Generic User Guide (ARM DUI0552A) - PendSV recommendation
- [corruption-scenario.md](corruption-scenario.md) - Detailed analysis
- [scheduler-design.md](scheduler-design.md) - Syscall trampoline design
- FreeRTOS port.c - PendSV/SVCall priority configuration
