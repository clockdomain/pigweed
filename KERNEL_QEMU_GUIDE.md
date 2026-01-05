# Running Pigweed Rust Kernel on QEMU

## Building the Kernel

To build just the Pigweed Rust kernel library:

```bash
bazelisk build //pw_kernel/kernel:kernel
```

## Running on QEMU

The Pigweed kernel supports several target configurations:

- `k_host` - Run on your host machine (Linux, macOS)
- `k_qemu_mps2_an505` - QEMU emulating Arm Cortex-M33 (MPS2-AN505) - **ARMv8-M**
- `k_qemu_lm3s6965` - QEMU emulating Arm Cortex-M3 (LM3S6965EVB) - **ARMv7-M**
- `k_qemu_ast1030` - QEMU emulating Aspeed AST1030 BMC (Cortex-M4) - **ARMv7-M**
- `k_qemu_virt_riscv32` - QEMU emulating RISC-V 32-bit system
- `k_rp2350` - Raspberry Pi RP2350 microcontroller

## ARMv7-M Threading Test (Cortex-M3)

### Quick Start

To run the ARMv7-M threading test on QEMU LM3S6965EVB:

```bash
bazelisk test \
  --test_output=all \
  --config=k_qemu_lm3s6965 \
  //pw_kernel/target/lm3s6965/threads/kernel:threads_test
```

### Expected Output

On success, you should see:

```
[INF] Welcome to Maize on ARMv7-M Minimal Kernel Threads!
[INF] Cortex-M early initialization
[INF] CPUID: revision=0x1, part_number=0xc23, architecture=0xf
[INF] MPU regions: 8
[INF] Thread B starting
[INF] Thread A: Incrementing counter
[INF] Thread B: Counter value 1
[INF] Thread A: Incrementing counter
[INF] Thread B: Counter value 2
[INF] Thread B: Done
[INF] Thread A: Done
[INF] ✅ PASSED

//pw_kernel/target/lm3s6965/threads/kernel:threads_test  PASSED in 2.9s
```

### Troubleshooting: CIPD Toolchain Timeout

If you encounter a timeout downloading the ARM GCC toolchain:

```
Failed to fetch CIPD repository `+_repo_rules5+gcc_arm_none_eabi_toolchain`: Timed out
```

**Solution**: Use increased timeouts for the first build:

```bash
bazelisk test \
  --experimental_scale_timeouts=10.0 \
  --experimental_repository_downloader_retries=10 \
  --test_output=all \
  --config=k_qemu_lm3s6965 \
  //pw_kernel/target/lm3s6965/threads/kernel:threads_test
```

This gives the download ~100 minutes instead of ~10 minutes. After the first successful build, the toolchain is cached and subsequent builds are much faster.

### What the Test Validates

The threading test verifies:
- Thread creation and scheduling on ARMv7-M
- Context switching between threads
- Thread synchronization using timeouts
- Shared memory access between threads
- Proper thread lifecycle (creation, execution, termination)

The test creates two threads that coordinate via a shared counter, demonstrating that the kernel's threading primitives work correctly on the Cortex-M3 architecture.

## AST1030 Tests (Cortex-M4)

The AST1030 platform (Aspeed BMC SoC) provides comprehensive kernel testing on ARMv7-M Cortex-M4.

### Available Tests

#### Threading Test
```bash
bazelisk test \
  --test_output=all \
  --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/threads/kernel:threads_test
```

Tests basic thread creation, scheduling, and synchronization.

#### Thread Termination Test
```bash
bazelisk test \
  --test_output=all \
  --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/thread_termination/kernel:thread_termination_test
```

Validates thread lifecycle and cleanup in 4 scenarios:
- **Terminate Sleep**: Thread terminated while sleeping
- **Signaled Termination**: Thread signaled to exit
- **Mutex**: Thread terminated while waiting on mutex
- **Thread Ref Drop**: Reference counting and cleanup

#### IPC Test
```bash
bazelisk test \
  --test_output=all \
  --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/ipc/user:ipc_test
```

Tests inter-process communication between user-space processes.

### Force Re-run Without Cache

To force a test to re-run even if cached results exist:

```bash
bazelisk test \
  --test_output=all \
  --cache_test_results=no \
  --config=k_qemu_ast1030 \
  //pw_kernel/target/ast1030/thread_termination/kernel:thread_termination_test
```

### AST1030 Platform Details
- **CPU**: ARM Cortex-M4F @ 200 MHz
- **Architecture**: ARMv7-M with PMSAv7
- **Memory**: 640 KB SRAM (RAM-only execution, no XIP)
- **MPU**: 8 regions
- **NVIC**: 64 IRQs
- **Unique Feature**: ROM bootloader copies firmware from flash to RAM

See [pw_kernel/target/ast1030/README.md](pw_kernel/target/ast1030/README.md) for complete AST1030 documentation.

## QEMU RISC-V 32-bit

To run the kernel tests on QEMU RISC-V with full output:

```bash
bazelisk test --test_output=all --cache_test_results=no --config k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/unittest_runner
```

To build all kernel components for QEMU RISC-V:

```bash
bazelisk build --config k_qemu_virt_riscv32 //pw_kernel/...
```

To run all tests for QEMU RISC-V:

```bash
bazelisk test --config k_qemu_virt_riscv32 //pw_kernel/...
```

### Specific Test Targets

Run specific kernel feature tests:

```bash
# Thread tests
bazelisk test --config k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/threads/...

# Interrupt tests
bazelisk test --config k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/interrupts/...

# Thread termination tests
bazelisk test --config k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/thread_termination/...

# IPC tests
bazelisk test --config k_qemu_virt_riscv32 //pw_kernel/target/qemu_virt_riscv32/ipc/...
```

## QEMU Arm Cortex-M33

To run on QEMU emulating an Arm Cortex-M33:

```bash
bazelisk test --config k_qemu_mps2_an505 //pw_kernel/...
```

## Platform Details

### ARMv7-M (Cortex-M3)
- **Target**: QEMU lm3s6965evb (TI Stellaris LM3S6965)
- **Platform**: `//pw_kernel/target/lm3s6965:lm3s6965`
- **Flash**: 0x00000000 - 0x00020000 (~128KB)
- **RAM**: 0x20000000 - 0x20008000 (32KB)
- **MPU**: 8 regions (PMSAv7)

### ARMv7-M (Cortex-M4)
- **Target**: QEMU ast1030-evb (Aspeed AST1030 BMC)
- **Platform**: `//pw_kernel/target/ast1030:ast1030`
- **Memory**: 640 KB SRAM (RAM-only, no XIP)
- **MPU**: 8 regions (PMSAv7)
- **Vector Table**: 264 vectors (1056 bytes)

### ARMv8-M (Cortex-M33)
- **Target**: QEMU mps2-an505
- **Platform**: `//pw_kernel/target/mps2_an505:mps2_an505`
- **MPU**: 16 regions (PMSAv8)

### RISC-V
- **Target**: QEMU virt (RV32IMAC)
- **Platform**: `//pw_kernel/target/qemu_virt_riscv32:qemu_virt_riscv32`

## Common Bazel Flags

**View test output**:
```bash
--test_output=all
```

**Force test re-run** (ignore cache):
```bash
--cache_test_results=no
```

**Build without running tests**:
```bash
bazelisk build --config=k_qemu_lm3s6965 [target]
```

**Increase download/build timeouts**:
```bash
--experimental_scale_timeouts=10.0
--experimental_repository_downloader_retries=10
```

## Notes

- The QEMU runner is handled automatically by the test framework
- You don't need to manually invoke QEMU
- The test runner script is located at [pw_kernel/tooling/qemu_runner.py](pw_kernel/tooling/qemu_runner.py)
- Use `--test_output=all` to see all test output
- Use `--cache_test_results=no` to force re-running tests
- First build may take 30-40 minutes due to toolchain download
- Subsequent builds are much faster (seconds to minutes)

## Running on Host

For quick testing on your development machine:

```bash
bazelisk test --config k_host //pw_kernel/...
```

## Additional Resources

- See [pw_kernel/quickstart.rst](pw_kernel/quickstart.rst) for complete documentation
- See [DESIGN_ARMv7M_Userspace_Support.md](DESIGN_ARMv7M_Userspace_Support.md) for ARMv7-M architecture details
- Kernel config: [pw_kernel/kernel.bazelrc](pw_kernel/kernel.bazelrc)
