.. _docs-pw_kernel-cortex-m-priorities:

======================================
Cortex-M Exception Priority Design
======================================

This document describes the exception priority configuration in pw_kernel's
ARM Cortex-M port and the rationale behind the design decisions.

Current Priority Configuration
==============================

The kernel configures system exception priorities in ``early_init()``
(see ``arch/arm_cortex_m/threads.rs``):

.. code-block:: rust

   // Only the high two bits of the priority are guaranteed to be implemented.
   // Note: Higher numeric values = lower priority

   scb.set_priority(scb::SystemHandler::SVCall,  0b1111_1111);  // 0xFF
   scb.set_priority(scb::SystemHandler::PendSV,  0b1111_1111);  // 0xFF (same as SVCall)
   scb.set_priority(scb::SystemHandler::SysTick, 0b0111_1111);  // 0x7F

External IRQ priorities are set in the NVIC initialization:

.. code-block:: rust

   nvic_regs.set_priority(i, 0b0100_0000);  // 0x40

Priority Hierarchy
------------------

With only the top 2 bits guaranteed to be implemented, the effective
priority levels are:

+----------------+-------+----------------+------------------+
| Exception      | Value | Top 2 bits     | Effective Level  |
+================+=======+================+==================+
| External IRQs  | 0x40  | 0b01           | 1 (highest)      |
+----------------+-------+----------------+------------------+
| SysTick        | 0x7F  | 0b01           | 1 (highest)      |
+----------------+-------+----------------+------------------+
| PendSV         | 0xFF  | 0b11           | 3 (lowest)       |
+----------------+-------+----------------+------------------+
| SVCall         | 0xFF  | 0b11           | 3 (lowest)       |
+----------------+-------+----------------+------------------+

Design Rationale
================

PendSV and SVCall are both set to the lowest priority (0xFF), following
ARM's recommendation. This ensures:

- Context switches only occur when no other exceptions need servicing
- No race conditions between PendSV and SVCall handlers
- Alignment with standard RTOS practice (FreeRTOS, Zephyr, ThreadX)

How Syscalls Work in pw_kernel
------------------------------

The kernel uses a non-standard syscall technique to work around Cortex-M
exception stacking limitations (see ``arch/arm_cortex_m/syscall.rs``):

1. User code executes ``SVC`` instruction
2. ``SVCall`` handler immediately returns to **Thread mode** via a fake
   exception frame
3. Actual syscall processing happens in ``handle_svc()`` running in
   Thread mode (not Handler mode)
4. On syscall completion, ``svc_return`` trampoline returns to user space

This design avoids the problem of nested SVCall exceptions when a blocking
syscall causes a context switch to another thread that also makes a syscall.

Why Same Priority Works
-----------------------

Since ``handle_svc()`` runs in Thread mode (not Handler mode), PendSV can
always preempt it regardless of priority. The priority relationship only
matters for the brief moment inside the actual ``SVCall`` exception handler.

Context switch flow:

1. Thread A makes a blocking syscall
2. ``SVCall`` handler returns immediately to Thread mode
3. ``handle_svc()`` runs in Thread mode, determines Thread A should block
4. Scheduler sets PendSV pending and drops the scheduler lock
5. PendSV fires and performs context switch to Thread B

Comparison with ARM's Recommendation
====================================

ARM's documentation recommends:

   "PendSV should be set to the lowest priority level to ensure context
   switching only occurs when no other exceptions need servicing."

Standard RTOS Practice
----------------------

Most RTOSes (FreeRTOS, Zephyr, ThreadX) configure:

+----------------+------------------+
| Exception      | Priority         |
+================+==================+
| External IRQs  | Various          |
+----------------+------------------+
| SysTick        | Low (but > PendSV)|
+----------------+------------------+
| SVCall         | Lowest           |
+----------------+------------------+
| PendSV         | Lowest (same)    |
+----------------+------------------+

The rationale:

- PendSV at lowest priority ensures context switches only happen when
  all interrupt handlers have completed
- This prevents stack corruption from switching mid-handler
- SysTick can safely pend a context switch knowing PendSV will run later

Architecture Notes
==================

The exception priority model is fundamentally the same across ARMv7-M
(Cortex-M3, M4, M7) and ARMv8-M (Cortex-M23, M33, M55, M85). ARMv8-M adds
TrustZone security extensions but inherits the same priority-based preemption
model with tail-chaining and late-arrival optimization.

Why Lowest Priority Works
-------------------------

The current configuration works correctly because:

1. The ``SVCall`` handler returns immediately to Thread mode
2. Actual syscall processing happens in Thread mode where PendSV can
   always preempt regardless of priority settings
3. Context switches from interrupt handlers (like SysTick) leave PendSV
   pending until handler completion via tail-chaining

Current Configuration Benefits
==============================

The current configuration (PendSV and SVCall both at 0xFF):

- Matches standard RTOS practice (FreeRTOS, Zephyr, ThreadX)
- Simplifies reasoning about exception behavior
- Works correctly with the existing syscall trampoline design
- Follows ARM's documented recommendations for both ARMv7-M and ARMv8-M
- Eliminates the race condition described in the audit findings below

The syscall design ensures context switches work correctly because
``handle_svc()`` runs in Thread mode, making the PendSV priority relative
to SVCall irrelevant for normal operation.

Audit Findings (Historical)
===========================

An audit identified a potential issue with a **previous** priority
configuration where PendSV (0xBF) had higher priority than SVCall (0xFF).
This has been fixed by setting both to 0xFF.

Potential Race: PendSV Preempting SVCall Handler
------------------------------------------------

**Scenario:**

1. Thread A is running
2. SysTick fires and determines Thread B should run
3. ``context_switch()`` sets ``active_thread = Thread A`` and sets PendSV pending
4. SysTick returns, Thread A continues (PendSV hasn't fired yet)
5. Thread A executes SVC instruction before PendSV fires
6. SVCall handler runs, reaching the ``cpsie i`` instruction (line 162 in syscall.rs)
7. **PendSV preempts SVCall** (because PendSV has higher priority)
8. PendSV saves the current frame (which is mid-SVCall handler) to Thread A
9. PendSV switches to Thread B

**Problem:**

When Thread A is later resumed, it resumes in the middle of the SVCall
handler with stale state. The exception frame saved contains:

- PC pointing to SVCall handler code (between ``cpsie i`` and ``bx lr``)
- LR containing either the original or fake EXC_RETURN value
- r0 pointing to a stale ``KernelExceptionFrame`` from a previous syscall

This could cause:

- Resuming in the wrong execution context
- Corrupted syscall return path
- Use of stale frame pointers

**Window of vulnerability:**

The SVCall handler has a 2-instruction window where interrupts are enabled
but still in Handler mode (lines 162-167 in syscall.rs):

.. code-block:: asm

   cpsie i                    // Interrupts enabled here
   ldr lr, ={exc_return}      // PendSV could fire here
   bx lr                      // Or here

**Why this works today:**

In practice, this race may be unlikely because:

1. The window is very small (2 instructions)
2. PendSV is only pending after a context switch decision
3. The timing would require SysTick, context switch decision, SysTick return,
   and SVC all within a few cycles

However, this is a latent bug that could manifest under specific timing
conditions or on faster processors.

**Resolution:**

This issue was fixed by setting PendSV to the same priority as SVCall (0xFF).
PendSV cannot preempt SVCall when they have equal priority, eliminating this
race entirely.

Code Paths That Set PendSV Pending
----------------------------------

Only one location sets PendSV pending:

- ``arch/arm_cortex_m/threads.rs:137`` - Inside ``context_switch()``

This is called from:

- ``scheduler::tick()`` via ``try_reschedule()`` - from SysTick handler
- ``handle_svc()`` via blocking syscalls - from Thread mode
- Various scheduler operations - typically from Thread mode

No Implicit Dependencies Found
------------------------------

The audit found **no code that intentionally depends** on PendSV preempting
SVCall. The previous priority configuration appeared to be based on a
misunderstanding that PendSV needs higher priority than SVCall to enable
context switching from syscalls. In reality, since ``handle_svc()`` runs in
Thread mode, PendSV can always preempt it regardless of relative priority.

References
==========

- ARM Cortex-M3 Technical Reference Manual, Section 5.3 (Exception Priorities)
- ARMv7-M Architecture Reference Manual, Section B1.5.4
- ARMv8-M Architecture Reference Manual, Section D1.2
- ``pw_kernel/arch/arm_cortex_m/threads.rs`` - Priority configuration
- ``pw_kernel/arch/arm_cortex_m/syscall.rs`` - Syscall implementation
