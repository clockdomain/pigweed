# pw_kernel Quickstart Guide

A practical guide to get started with Pigweed's experimental Rust-based RTOS kernel.

## Overview

`pw_kernel` is an experimental, security-focused RTOS written in Rust. Key features:

- **Rust-powered core** with memory safety guarantees
- **Hardware protection** via MPU (ARM) and PMP (RISC-V)
- **User-space support** with kernel/user privilege separation
- **Preemptive scheduler** with threads and processes
- **IPC mechanisms** for inter-process communication

## Supported Targets

| Config | Platform | Architecture | Description |
|--------|----------|--------------|-------------|
| `k_host` | Linux/macOS | x86_64 | Host simulation for testing |
| `k_qemu_mps2_an505` | QEMU | ARMv8-M (Cortex-M33) | MPS2-AN505 board emulation |
| `k_qemu_ast1030` | QEMU | ARMv7-M (Cortex-M4) | ASPEED AST1030 emulation |
| `k_qemu_virt_riscv32` | QEMU | RISC-V 32-bit | QEMU virt machine |
| `k_rp2350` | Hardware | ARM Cortex-M33 | Raspberry Pi RP2350 |

## Prerequisites

### Linux (Recommended)

Only minimal system packages are required - Pigweed's bootstrap handles everything else:

```bash
# System prerequisites (one-time)
sudo apt install git build-essential
```

### Clone and Bootstrap

```bash
git clone https://pigweed.googlesource.com/pigweed/pigweed
cd pigweed

# First-time setup (downloads all toolchains automatically)
source bootstrap.sh
```

Bootstrap automatically provides:
- Bazelisk (Bazel launcher)
- Rust toolchain
- ARM GCC cross-compiler
- RISC-V GCC cross-compiler  
- QEMU emulators
- GDB debuggers
- Python environment
- All other required tools

### Subsequent Sessions

After the initial bootstrap, just re-activate the environment:

```bash
cd pigweed
source activate.sh    # Fast - no downloads
```

## Building pw_kernel

### Build for a Specific Target

```bash
# Build all pw_kernel components for a target
bazelisk build --config=<CONFIG> //pw_kernel/...

# Examples:
bazelisk build --config=k_host //pw_kernel/...
bazelisk build --config=k_qemu_mps2_an505 //pw_kernel/...
bazelisk build --config=k_qemu_ast1030 //pw_kernel/...
```

### Build a Specific Application

```bash
# Build IPC test for MPS2-AN505 (ARMv8-M)
bazelisk build --config=k_qemu_mps2_an505 //pw_kernel/target/mps2_an505/ipc/user:ipc

# Build IPC test for AST1030 (ARMv7-M)
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc
```

## Running Tests

### Run All Tests

```bash
# Run tests on host
bazelisk test --config=k_host //pw_kernel/...

# Run tests on QEMU (ARM Cortex-M33)
bazelisk test --config=k_qemu_mps2_an505 //pw_kernel/...

# Run tests on QEMU (RISC-V)
bazelisk test --config=k_qemu_virt_riscv32 //pw_kernel/...
```

### Run with Output

```bash
# Stream test output
bazelisk test --config=k_qemu_mps2_an505 \
  --test_output=streamed \
  //pw_kernel/target/mps2_an505/ipc/user:ipc_test

# Show all output and skip cache
bazelisk test --config=k_qemu_mps2_an505 \
  --test_output=all \
  --cache_test_results=no \
  //pw_kernel/target/mps2_an505/ipc/user:ipc_test
```

### Run Interactively with QEMU

```bash
# Launch QEMU directly (useful for debugging)
bazelisk run --config=k_qemu_mps2_an505 //pw_kernel/target/mps2_an505/ipc/user:ipc_qemu
```

## Project Structure

```
pw_kernel/
├── arch/                    # Architecture-specific code
│   ├── arm_cortex_m/        # ARM Cortex-M (ARMv7-M, ARMv8-M)
│   ├── host/                # Host simulation
│   └── riscv/               # RISC-V
├── kernel/                  # Core kernel (scheduler, sync primitives)
├── syscall/                 # System call interface
├── target/                  # Target-specific configurations
│   ├── ast1030/             # ASPEED AST1030 (Cortex-M4)
│   ├── mps2_an505/          # ARM MPS2-AN505 (Cortex-M33)
│   ├── pw_rp2350/           # Raspberry Pi RP2350
│   └── qemu_virt_riscv32/   # RISC-V QEMU
├── subsys/                  # Subsystems (console, etc.)
├── userspace/               # User-space libraries
└── macros/                  # Proc macros for kernel
```

## Writing a Simple Kernel Application

### 1. Create a Kernel Thread

```rust
use kernel::scheduler::{self, thread::ThreadBuilder};
use pw_log::info;

fn main_thread(arg0: usize, _arg1: usize, _arg2: usize) {
    info!("Hello from kernel thread! arg0={}", arg0);
    
    // Your application logic here
    
    scheduler::exit_thread(Arch);
}

fn kernel_main() {
    // Create and start a thread
    let thread = ThreadBuilder::new("my_thread")
        .with_entry(main_thread)
        .with_args((42, 0, 0))
        .build()
        .expect("Failed to create thread");
    
    scheduler::start_thread(thread);
}
```

### 2. Create a User-Space Process

```rust
// User application (runs in unprivileged mode)
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Make system calls to interact with kernel
    syscall_user::debug_log("Hello from user space!");
    
    loop {
        // Application logic
    }
}
```

## VS Code Setup

### Generate rust-project.json

```bash
bazelisk run @rules_rust//tools/rust_analyzer:gen_rust_project -- \
  --config=k_qemu_mps2_an505 //pw_kernel/...
```

### Configure rust-analyzer

Add to `.vscode/settings.json`:

```json
{
  "rust-analyzer.linkedProjects": ["rust-project.json"],
  "rust-analyzer.check.overrideCommand": [
    "bazelisk",
    "build",
    "--config=k_lint",
    "--config=k_qemu_mps2_an505",
    "--@rules_rust//:error_format=json",
    "//pw_kernel/..."
  ]
}
```

## Debugging with GDB

### Start QEMU with GDB Server

```bash
# Build the target
bazelisk build --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc

# Run QEMU with GDB server (paused at start)
bazelisk run --config=k_qemu_ast1030 //pw_kernel/target/ast1030/ipc/user:ipc_qemu -- -S -s
```

### Connect GDB

```bash
# In another terminal
arm-none-eabi-gdb bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

(gdb) target remote :1234
(gdb) break main
(gdb) continue
```

## Hardware Deployment (RP2350)

### Build

```bash
bazelisk build --config=k_rp2350 //pw_kernel/target/pw_rp2350/ipc/user:ipc
```

### Flash with probe-rs

```bash
# Install probe-rs: https://probe.rs/docs/getting-started/installation/
probe-rs download bazel-bin/pw_kernel/target/pw_rp2350/ipc/user/ipc.elf
probe-rs reset --chip rp2350
```

### View Console Output

```bash
bazelisk run --config=k_rp2350 //pw_kernel/target/pw_rp2350/ipc/user:ipc -- -d /dev/ttyACM0
```

## Key Concepts

### Memory Protection Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| **Protected** | Hardware MPU/PMP enforced isolation | Security-critical applications |
| **Lightweight** | No memory protection | Resource-constrained systems |

### Synchronization Primitives

- **SpinLock**: Interrupt-disabling lock for short critical sections
- **Mutex**: Blocking mutex with priority inheritance
- **Event**: Binary event signaling

### IPC (Inter-Process Communication)

- **Channels**: Typed message passing between processes
- **Shared Memory**: Controlled shared regions with MPU protection

## Troubleshooting

### Build Errors

```bash
# Clean build cache
bazelisk clean --expunge

# Rebuild
bazelisk build --config=<CONFIG> //pw_kernel/...
```

### QEMU Hangs

If tests hang on QEMU:
1. Check if semihosting is being used with interrupts disabled (known issue)
2. Try running with `--test_timeout=120` for longer timeout
3. Use GDB to debug: `bazelisk run ... -- -S -s`

### Missing Toolchain

Bazel automatically downloads toolchains. If issues persist:

```bash
# Force toolchain refresh
bazelisk sync
```

## Further Reading

- [Design Document](design.rst) - Architecture and design philosophy
- [Guides](guides.rst) - In-depth guides for specific features
- [Roadmap](roadmap.rst) - Future development plans
- [Pigweed Documentation](https://pigweed.dev/) - Main Pigweed docs

---

*pw_kernel is experimental. APIs may change.*
