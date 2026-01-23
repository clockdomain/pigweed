# Hello User Mode - Minimal Test App

**Purpose:** Create the simplest possible user-mode app to isolate the CONTROL register corruption bug on AST1030.

---

## Why This Test?

The IPC test has two user processes (initiator + handler) which complicates debugging. A single "hello world" user app will help determine:

1. Does the fault happen on **first entry** to user mode?
2. Does the fault happen only after **syscalls**?
3. Is the bug in the **context switch** code or **syscall handler**?

---

## Directory Structure

```
pw_kernel/target/ast1030/hello_user/
├── BUILD.bazel
├── system.json5
├── target.rs
└── (uses pw_kernel/tests/hello_user/hello.rs)

pw_kernel/tests/hello_user/
├── BUILD.bazel
└── hello.rs
```

---

## File: `pw_kernel/tests/hello_user/hello.rs`

```rust
// Copyright 2025 The Pigweed Authors
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License. You may obtain a copy of
// the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations under
// the License.
#![no_main]
#![no_std]

use userspace::entry;

/// Minimal user-mode entry point.
/// This tests if we can successfully enter user mode at all.
#[entry]
fn main() -> ! {
    // If we get here, user mode entry succeeded!
    pw_log::info!("Hello from user mode!");
    
    // Simple loop - no syscalls yet
    let mut counter: u32 = 0;
    loop {
        counter = counter.wrapping_add(1);
        if counter % 1_000_000 == 0 {
            // This will trigger a syscall (logging)
            pw_log::info!("User mode still running: counter = {}", counter as u32);
        }
    }
}
```

---

## File: `pw_kernel/tests/hello_user/BUILD.bazel`

```python
# Copyright 2025 The Pigweed Authors
#
# Licensed under the Apache License, Version 2.0 (the "License"); you may not
# use this file except in compliance with the License. You may obtain a copy of
# the License at
#
#     https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
# WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
# License for the specific language governing permissions and limitations under
# the License.
load("@rules_rust//rust:defs.bzl", "rust_binary")
load("//pw_kernel/tooling:app_package.bzl", "app_package")

rust_binary(
    name = "hello",
    srcs = [
        "hello.rs",
    ],
    edition = "2024",
    tags = ["kernel"],
    visibility = ["//visibility:public"],
    deps = [
        ":app_hello",
        "//pw_kernel/userspace",
        "//pw_log/rust:pw_log",
    ],
)

app_package(
    name = "app_hello",
    app_name = "hello",
    edition = "2024",
    system_config = "//pw_kernel/target:system_config_file",
    tags = ["kernel"],
)
```

---

## File: `pw_kernel/target/ast1030/hello_user/system.json5`

```json5
// Copyright 2025 The Pigweed Authors
//
// Minimal single-app configuration for AST1030 hello user mode test
{
    arch: {
        type: "armv7m",
        vector_table_start_address: 0x00000000,
        vector_table_size_bytes: 1056,  // 0x420
    },
    kernel: {
        flash_start_address: 0x00000420,  // After vector table
        flash_size_bytes: 130016,         // ~126KB (ends at 0x00020000)
        ram_start_address: 0x00060000,    // After flash regions
        ram_size_bytes: 131072,           // 128KB
    },
    apps: [
        {
            name: "hello",
            flash_size_bytes: 131072,     // 128KB for app code
            ram_size_bytes: 65536,        // 64KB RAM for app
        },
    ],
}
```

---

## File: `pw_kernel/target/ast1030/hello_user/target.rs`

```rust
// Copyright 2025 The Pigweed Authors
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License. You may obtain a copy of
// the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations under
// the License.
#![no_std]
#![no_main]

use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit};
use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "AST1030 Hello User Mode";

    fn main() -> ! {
        codegen::start();
        #[expect(clippy::empty_loop)]
        loop {}
    }

    fn shutdown(code: u32) -> ! {
        pw_log::info!("Shutting down with code {}", code as u32);
        let status = match code {
            0 => EXIT_SUCCESS,
            _ => EXIT_FAILURE,
        };
        exit(status);
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
```

---

## File: `pw_kernel/target/ast1030/hello_user/BUILD.bazel`

```python
# Copyright 2025 The Pigweed Authors
#
# Licensed under the Apache License, Version 2.0 (the "License"); you may not
# use this file except in compliance with the License. You may obtain a copy of
# the License at
#
#     https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
# WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
# License for the specific language governing permissions and limitations under
# the License.

load("@rules_rust//rust:defs.bzl", "rust_binary")
load("//pw_kernel/target/ast1030:defs.bzl", "TARGET_COMPATIBLE_WITH")
load("//pw_kernel/tooling:system_image.bzl", "system_image", "system_image_test")
load("//pw_kernel/tooling:target_codegen.bzl", "target_codegen")
load("//pw_kernel/tooling:target_linker_script.bzl", "target_linker_script")

system_image(
    name = "hello_user",
    apps = [
        "//pw_kernel/tests/hello_user:hello",
    ],
    kernel = ":target",
    platform = "//pw_kernel/target/ast1030",
    system_config = ":system_config",
    tags = ["kernel"],
    visibility = ["//visibility:public"],
)

system_image_test(
    name = "hello_user_test",
    image = ":hello_user",
    target_compatible_with = TARGET_COMPATIBLE_WITH,
)

filegroup(
    name = "system_config",
    srcs = ["system.json5"],
)

target_codegen(
    name = "codegen",
    arch = "//pw_kernel/arch/arm_cortex_m:arch_arm_cortex_m",
    system_config = ":system_config",
    target_compatible_with = TARGET_COMPATIBLE_WITH,
)

target_linker_script(
    name = "linker_script",
    system_config = ":system_config",
    tags = ["kernel"],
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    template = "//pw_kernel/target/ast1030:linker_script_template",
)

rust_binary(
    name = "target",
    srcs = [
        "target.rs",
    ],
    edition = "2024",
    tags = ["kernel"],
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        ":codegen",
        ":linker_script",
        "//pw_kernel/arch/arm_cortex_m:arch_arm_cortex_m",
        "//pw_kernel/kernel",
        "//pw_kernel/subsys/console:console_backend",
        "//pw_kernel/target:target_common",
        "//pw_kernel/target/ast1030:entry",
        "//pw_kernel/userspace",
        "//pw_log/rust:pw_log",
        "@rust_crates//:cortex-m-semihosting",
    ],
)
```

---

## Build and Test Commands

```bash
# Build
bazelisk build //pw_kernel/target/ast1030/hello_user:hello_user --config=k_qemu_ast1030

# Test
bazelisk test //pw_kernel/target/ast1030/hello_user:hello_user_test --config=k_qemu_ast1030 --test_output=streamed

# Debug with GDB
qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic \
  -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf \
  -semihosting-config enable=on,target=native -S -gdb tcp::3333 &

gdb-multiarch bazel-bin/pw_kernel/target/ast1030/hello_user/hello_user.elf \
  -ex "target remote :3333"
```

---

## Expected Outcomes

### If Fault on First Entry
```
[INF] Starting thread 'hello' (0x...)
[INF] HardFault exception triggered...
```
**Conclusion:** Bug is in initial user mode entry, not syscall path.

### If Hello Prints Then Faults on Syscall
```
[INF] Starting thread 'hello' (0x...)
[INF] Hello from user mode!
[INF] HardFault exception triggered...
```
**Conclusion:** Initial entry works, bug is in syscall handling/return.

### If Runs Successfully
```
[INF] Starting thread 'hello' (0x...)
[INF] Hello from user mode!
[INF] User mode still running: counter = 1000000
[INF] User mode still running: counter = 2000000
...
```
**Conclusion:** Single-app works! Bug may be in multi-process IPC interaction.

---

## Debugging Notes

### Key Breakpoints
```gdb
break *0x5da           # PendSV entry
break pendsv_swap_sp   # Context switch function
break MemoryManagement # Catch the fault
```

### Check CONTROL at Key Points
```gdb
# At PendSV entry
print/x $control
print/x $psp
print/x $lr

# At user mode entry
# Should see: control=0x3, psp=<user_stack>, lr=0xFFFFFFFD
```
