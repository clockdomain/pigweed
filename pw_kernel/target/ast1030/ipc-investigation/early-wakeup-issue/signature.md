# Early Wakeup Issue - Failure Signature

## Test Output (Normal Run - No GDB)

```
[INF] Welcome to Maize on AST1030 User IPC!
[INF] Cortex-M early initialization
[INF] CPUID: revision=0x0, part_number=0xc24, architecture=0xf, variant=0x0, implementor=0x41
[INF] MPU regions: 8
[INF] Starting monotonic SysTick timer
[INF] Created initial thread; bootstrapping
[INF] Welcome to the first thread, continuing bootstrap
[INF] Cortex-M initialization
[INF] Ticks per 10ms: 0
[INF] Starting thread 'idle' (0x00060074)
[INF] Allocating non-privileged process 'initiator process'
[INF] Allocating non-privileged thread 'initiator thread' (entry: 0x00020001)
[INF] Initializing non-privileged thread 'initiator thread'
[INF] Starting thread 'initiator thread' (0x000610a8)
[INF] Allocating non-privileged process 'handler process'
[INF] Allocating non-privileged thread 'handler thread' (entry: 0x00040001)
[INF] Initializing non-privileged thread 'handler thread'
[INF] Starting thread 'handler thread' (0x00061990)
[INF] IPC service starting
[INF] Ipc test starting
[INF] Handler: waiting for READABLE
[INF] Initiator: sending char 97
[%s] Handler: object_wait OK
[%s] Handler: calling channel_read
[INF] Initiator: transact returned 0 bytes
[ERR] Received 0 bytes, 8 expected
[ERR] ❌ FAILED: 11
```

## Bug Signature

| Event | Expected | Actual |
|-------|----------|--------|
| Handler: object_wait | Block until data ready | Returns immediately (OK) |
| Handler: channel_read | Read 8 bytes | Reads 0 bytes |
| Initiator: transact | Complete transaction | Returns 0 bytes sent |

## Sequence Diagram (What Happens)

```
Handler                          Initiator
   |                                |
   | object_wait(READABLE)          |
   |-----> RETURNS OK (WRONG!)      |
   |                                | channel_transact(...)
   | channel_read()                 |     (hasn't stored data yet!)
   |-----> 0 bytes (no data)        |
   |                                |
```

## Root Cause Hypothesis

The `object_wait` on the handler side returns **before** the initiator's `channel_transact` stores the transaction data. Either:

1. READABLE signal is pre-set on the channel
2. Spurious wakeup from Event system
3. Race condition in signal/wait mechanism

## Relationship to MUNSTKERR

| Environment | Behavior |
|-------------|----------|
| Normal run (no GDB) | Early wakeup → 0 bytes |
| Under GDB (with breakpoints) | MUNSTKERR or DACCVIOL fault |

The GDB breakpoints change timing, exposing a different manifestation of what may be the same underlying bug in syscall return handling.
