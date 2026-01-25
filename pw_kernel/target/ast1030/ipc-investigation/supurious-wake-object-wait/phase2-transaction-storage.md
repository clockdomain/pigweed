# Phase 2: Transaction Storage Validation

## Objective

Verify that the transaction (send buffer, recv buffer, initiator reference) is correctly stored in the handler object before the handler tries to read it.

---

## Relevant Code

**Location**: `pw_kernel/kernel/object/channel.rs`

```rust
// In ChannelInitiatorObject::channel_transact()
*active_transaction = Some(Transaction {
    send_buffer,
    recv_buffer,
    initiator: self_rc,
});

drop(active_transaction);  // Release mutex

// Signal handler AFTER storing transaction
self.handler.base.signal(kernel, Signals::READABLE);
```

---

## What Could Go Wrong

1. **Race condition**: Handler reads before transaction is stored
2. **Mutex issue**: Lock not properly released before signal
3. **Memory corruption**: Transaction struct corrupted
4. **Reference issue**: `self_rc` invalid

---

## Test Strategy

### Test 2.1: Transaction Presence Logging

Add logging in `ChannelHandlerObject::channel_read()`:

```rust
fn channel_read(&self, _kernel: K, offset: usize, mut read_buffer: SyscallBuffer) -> Result<usize> {
    let active_transaction = self.active_transaction.lock();
    
    // ADD THIS DEBUG
    if active_transaction.is_none() {
        pw_log::error!("channel_read: NO ACTIVE TRANSACTION!");
        return Err(Error::FailedPrecondition);
    }
    
    let Some(ref transaction) = *active_transaction else {
        return Err(Error::FailedPrecondition);
    };
    
    // ADD THIS DEBUG  
    pw_log::debug!("channel_read: send_buffer.size={}", transaction.send_buffer.size() as u32);
    
    transaction.send_buffer.copy_into(offset, &mut read_buffer)
}
```

### Test 2.2: Transaction Storage Logging

Add logging in `ChannelInitiatorObject::channel_transact()`:

```rust
// After storing transaction
pw_log::debug!("channel_transact: stored transaction, send_size={}, recv_size={}", 
    send_buffer.size() as u32, recv_buffer.size() as u32);

drop(active_transaction);

pw_log::debug!("channel_transact: signaling handler");
self.handler.base.signal(kernel, Signals::READABLE);
```

---

## Expected Output (if working correctly)

```
[DBG] channel_transact: stored transaction, send_size=4, recv_size=8
[DBG] channel_transact: signaling handler
[DBG] channel_read: send_buffer.size=4
```

## Expected Output (if transaction missing)

```
[DBG] channel_transact: stored transaction, send_size=4, recv_size=8
[DBG] channel_transact: signaling handler
[ERR] channel_read: NO ACTIVE TRANSACTION!
```

---

## Implementation Steps

1. Add conditional debug logging (behind a feature flag or compile-time constant)
2. Rebuild IPC test
3. Analyze output

---

## Success Criteria

- [ ] Transaction is present when handler reads
- [ ] `send_buffer.size()` matches initiator's send size
- [ ] No race between storage and read

## Findings

*(To be filled during investigation)*

---

## Next Phase

If transaction storage works → [Phase 3: SyscallBuffer Access](phase3-syscallbuffer-access.md)
