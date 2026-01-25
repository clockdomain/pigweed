# AST1030 Two-Process Context Switch Test Plan

**Date:** January 23, 2026  
**Target:** AST1030 (ARM Cortex-M4, ARMv7-M, PMSAv7)  
**Goal:** Stress test context switching between two user-mode processes

## Overview

Create a simple test with two independent processes that:
1. **Process A:** Waits for varying durations, logs progress
2. **Process B:** Waits for different durations, logs progress
3. Both run concurrently, causing frequent context switches via scheduler
4. No IPC needed - just time-based syscalls (`sleep`/`yield`)

This tests:
- User-mode process context switching
- MPU region switching between processes
- SVCall/PendSV priority handling (our recent fix)
- Scheduler fairness

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Kernel (Privileged)                      │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │   Scheduler  │◄──►│   SysTick    │    │ MPU Manager  │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
└─────────────────────────────────────────────────────────────────┘
        │                      │                    │
        ▼                      ▼                    ▼
┌───────────────────┐    ┌───────────────────┐
│  Process Alpha    │    │  Process Beta     │
│  (Unprivileged)   │    │  (Unprivileged)   │
│                   │    │                   │
│  for i in 1..50:  │    │  for i in 1..50:  │
│    sleep(rand)    │    │    sleep(rand)    │
│    log progress   │    │    log progress   │
└───────────────────┘    └───────────────────┘
         ▲                        ▲
         └────── Context Switch ──┘
```

## Test Logic

### Process Alpha
```rust
fn main() {
    pw_log::info!("Alpha: starting");
    for i in 0..50 {
        // Pseudo-random delay: (i * 7 + 3) % 10 ms
        let delay_ms = ((i * 7 + 3) % 10) + 1;
        syscall::sleep(Duration::from_millis(delay_ms));
        pw_log::info!("Alpha: iteration {}", i);
    }
    pw_log::info!("Alpha: completed 50 iterations");
    syscall::debug_shutdown(Ok(()));
}
```

### Process Beta
```rust
fn main() {
    pw_log::info!("Beta: starting");
    for i in 0..50 {
        // Different pattern: (i * 11 + 5) % 10 ms
        let delay_ms = ((i * 11 + 5) % 10) + 1;
        syscall::sleep(Duration::from_millis(delay_ms));
        pw_log::info!("Beta: iteration {}", i);
    }
    pw_log::info!("Beta: completed 50 iterations");
    // Don't shutdown - let Alpha handle it
}
```

---

## File Structure

```
pw_kernel/target/ast1030/context_switch_test/
├── BUILD.bazel           # Build configuration
├── system.json5          # Two-process system configuration
└── target.rs             # Target-specific code

pw_kernel/tests/context_switch/
├── BUILD.bazel           # Test app build
├── alpha.rs              # Process Alpha
└── beta.rs               # Process Beta
```

---

## Implementation TODOs

### Phase 1: Setup Infrastructure
- [ ] **1.1** Create `pw_kernel/target/ast1030/context_switch_test/` directory
- [ ] **1.2** Create `system.json5` with two-process configuration
  - Process Alpha (no IPC objects, just a thread)
  - Process Beta (no IPC objects, just a thread)
  - PMSAv7-compliant memory layout
- [ ] **1.3** Create `target.rs` (copy from hello_user)
- [ ] **1.4** Create `BUILD.bazel` for target

### Phase 2: Test Application Code
- [ ] **2.1** Create `pw_kernel/tests/context_switch/` directory
- [ ] **2.2** Create `alpha.rs` - loops 50x with varying sleeps
- [ ] **2.3** Create `beta.rs` - loops 50x with different sleep pattern
- [ ] **2.4** Create `BUILD.bazel` for test apps

### Phase 3: Integration & Testing
- [ ] **3.1** Build the context_switch_test target
  ```bash
  bazelisk build //pw_kernel/target/ast1030/context_switch_test:context_switch_test \
      --config=k_qemu_ast1030
  ```
- [ ] **3.2** Run single test
  ```bash
  bazelisk test //pw_kernel/target/ast1030/context_switch_test:context_switch_test_test \
      --config=k_qemu_ast1030 --test_output=streamed --test_timeout=60
  ```
- [ ] **3.3** Run 20x stability test
  ```bash
  for i in $(seq 1 20); do
      echo "=== Run $i ==="
      bazelisk test //pw_kernel/target/ast1030/context_switch_test:context_switch_test_test \
          --config=k_qemu_ast1030 --test_timeout=60 --nocache_test_results 2>&1 \
          | grep -E "PASSED|FAILED|TIMEOUT|Alpha.*completed|Beta.*completed"
  done
  ```

---

## Memory Layout (PMSAv7-Compliant)

```
Address         Size    Description
─────────────────────────────────────────────
0x00000000     1056B    Vector table
0x00000420    ~126KB    Kernel code (ends at 0x00020000)
0x00020000     128KB    Alpha app code (power-of-2 aligned)
0x00040000     128KB    Beta app code (power-of-2 aligned)
0x00060000     128KB    Kernel RAM (power-of-2 aligned)
0x00080000      32KB    Alpha app RAM
0x00088000      32KB    Beta app RAM
─────────────────────────────────────────────
Total:         576KB    (fits in AST1030's 768KB SRAM)
```

---

## Success Criteria

1. **Functional:** Both processes complete 50 iterations each
2. **Interleaving:** Log output shows Alpha/Beta messages interleaved (context switching)
3. **Stability:** 20/20 test runs pass without HardFault or timeout
4. **No Regressions:** MPS2-AN505 tests continue to pass

---

## Expected Output

```
[INF] Alpha: starting
[INF] Beta: starting
[INF] Alpha: iteration 0
[INF] Beta: iteration 0
[INF] Beta: iteration 1
[INF] Alpha: iteration 1
[INF] Alpha: iteration 2
[INF] Beta: iteration 2
... (interleaved based on sleep durations)
[INF] Alpha: completed 50 iterations
[INF] Beta: completed 50 iterations
[INF] Shutting down with code 0
```

---

## Notes

- No IPC channels needed - simpler than existing IPC test
- Uses only `sleep()` syscall for timing + logging syscalls
- Pseudo-random delays ensure varied interleaving patterns
- Alpha process handles shutdown after both complete
- Longer timeout (60s) since 50 iterations with sleeps takes time

