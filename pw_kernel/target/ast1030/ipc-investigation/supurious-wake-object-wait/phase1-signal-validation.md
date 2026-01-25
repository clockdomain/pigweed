# Phase 1: Signal & Wait Validation

## Objective

Verify that the signal mechanism correctly coordinates between initiator and handler processes.

---

## Status: 🔴 ISSUE FOUND

### Finding

The handler's `object_wait(READABLE)` returns **BEFORE** the initiator's `channel_transact()` has:
1. Stored the transaction in `active_transaction`
2. Called `signal(READABLE)` on the handler

### Evidence

From debug trace (ipc_test_output2.log):
```
[INF] Handler: waiting for READABLE
[INF] Initiator: sending char 97         <-- Log happens before syscall
[INF] Handler: object_wait OK            <-- BUT handler already woke up!
[INF] Handler: calling channel_read
[DBG] syscall: 0x0002                    <-- channel_read syscall
[INF] KERNEL: channel_read handle=0 offset=0 addr=0x8ffbc len=4
[DBG] syscall: 0x0001                    <-- channel_transact syscall AFTER channel_read started!
```

### Analysis

The syscall trace shows:
- `0x0002` (channel_read) enters kernel BEFORE
- `0x0001` (channel_transact) enters kernel

This is backwards! The handler is reading before the initiator has stored the transaction.

---

## Possible Causes

### 1. Spurious Wakeup in Event System
The `Event::wait_until()` may be returning early without actually being signaled.

**Files to check:**
- `pw_kernel/kernel/sync/event.rs`

### 2. Signal Pre-Set
The READABLE signal might be pre-set on channel creation.

**Check:**
```rust
// In ObjectBaseState::new()
active_signals: Signals::new(),  // Returns Signals(0) - should be empty
```

### 3. Race in Scheduler
Context switch timing issue causing premature wakeup.

### 4. ARM-Specific Issue
Something in the ARMv7-M port causing incorrect wait behavior.

---

## Relevant Code

### ObjectBase::wait_until() (object.rs)
```rust
pub fn wait_until(...) -> Result<Signals> {
    let mut state = self.state.lock(kernel);

    // Skip waiting if signals are already pending.
    if state.active_signals.contains(signal_mask) {  // <-- Check this first
        return Ok(state.active_signals);
    }

    let event = Event::new(kernel, EventConfig::ManualReset);
    // ... creates waiter, adds to list ...
    
    drop(state);  // Release lock
    let wait_result = event.wait_until(deadline);  // <-- Block here
    // ...
}
```

### ObjectBase::signal() (object.rs)
```rust
pub fn signal(&self, kernel: K, active_signals: Signals) {
    let mut state = self.state.lock(kernel);
    state.active_signals = active_signals;

    let _ = state.waiters.for_each(|waiter| -> Result<()> {
        if waiter.signal_mask.contains(active_signals) {
            unsafe { waiter.wait_result.set(Ok(active_signals)) };
            waiter.signaler.signal();  // <-- Wake up waiter
        }
        Ok(())
    });
}
```

---

## Next Investigation Steps

1. Add logging to `ObjectBase::wait_until()`:
   - Log when signals already pending (early return)
   - Log when actually waiting on event
   - Log when event returns

2. Add logging to `ObjectBase::signal()`:
   - Log when called and with what signals
   - Log number of waiters woken

3. Check if LM3S6965 has same issue:
   ```bash
   bazelisk test --config=k_qemu_lm3s6965 //pw_kernel/target/lm3s6965/ipc/user:ipc_test
   ```

---

## Success Criteria

- [ ] Understand why handler wakes before transaction is stored
- [ ] Identify the specific code path causing premature wakeup
- [ ] Implement fix
- [ ] Verify handler only wakes AFTER initiator signals

---

## Next Phase

Once signal timing is fixed → [Phase 2: Transaction Storage](phase2-transaction-storage.md)
