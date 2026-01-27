# IPC Test Race Condition Debugging Guide

This guide shows a practical workflow for debugging race conditions in
`//pw_kernel/target/ast1030/ipc/user:ipc_test` on QEMU.

## 1) Reproduce reliably

Run the test repeatedly and capture failures for comparison. Use the existing
script to keep logs from failing runs.

```
./pw_kernel/scripts/run_ipc_test.sh 50 ast1030
```

When a failure appears, save:

- The full test output
- The failing seed/run index
- The exact QEMU invocation (from Bazel test log)

## 2) Enable targeted QEMU tracing

Instruction traces are large. Use a focused trace window and minimal flags.

Recommended flags:

```
-d exec,int,cpu -D /tmp/qemu_ipc_trace.log
```

Notes:

- `exec` gives instruction flow
- `int` shows interrupt entry/exit and IRQ routing
- `cpu` captures CPU state changes

## 3) Narrow the scope with breakpoints

Use GDB to stop at high‑value locations:

- IPC send/receive syscalls
- Scheduler entry/exit
- PendSV/SVC handlers
- Channel wait/wake paths

Once stopped, single‑step a short window and correlate with the QEMU trace.

## 4) Use record/replay for determinism

Record/replay makes races repeatable.

Record:

```
qemu-system-arm ... -icount shift=1,rr=record -D /tmp/qemu_rr.log -d exec,int
```

Replay:

```
qemu-system-arm ... -icount shift=1,rr=replay -D /tmp/qemu_rr.log -d exec,int
```

This lets you reproduce the same interleaving while stepping under GDB.

## 5) Add temporary trace markers

Add log markers around suspected race windows (e.g., queue operations, wake
signals). Use short, unique tags so you can grep quickly in logs.

Keep these changes local or on a scratch branch.

## 6) Compare “good” vs “bad” runs

Diff the traces around the critical window:

- Which thread acquired the lock first?
- Was an interrupt delayed or masked?
- Did a wake signal precede the waiter?

The deltas usually point to missing memory barriers, incorrect IRQ masks, or
improper scheduler handoff.

## 7) Capture artifacts for review

When you identify the failing interleaving, save:

- GDB transcript
- QEMU trace log
- Test output
- A short timeline of key events

These make the race condition actionable for fixes or design updates.

## Quick checklist

- [ ] Repro script confirms flakiness
- [ ] QEMU trace enabled with minimal flags
- [ ] GDB breakpoints on IPC + scheduler
- [ ] Record/replay used for deterministic stepping
- [ ] “Good vs bad” trace comparison
