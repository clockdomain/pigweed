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

//! Minimal user-mode test application.
//!
//! This is the simplest possible user-mode app to test if user mode entry
//! works correctly. It helps isolate CONTROL register corruption bugs.

#![no_main]
#![no_std]

use userspace::{entry, syscall};

#[entry]
fn entry() -> ! {
    // If we get here, user mode entry succeeded!
    pw_log::info!("🎉 Hello from user mode!");
    pw_log::info!("User mode entry successful - no fault on initial entry");

    // Now test a simple syscall (logging uses syscalls)
    pw_log::info!("Testing syscalls via logging...");

    // Stress test: Do many syscalls in a loop to trigger potential
    // context switch issues. Each debug_nop() is a syscall that could
    // trigger PendSV if there's a higher-priority thread ready.
    pw_log::info!("Stress testing with 100 nop syscalls...");
    for i in 0..100 {
        let _ = syscall::debug_nop();
        if i % 25 == 0 {
            pw_log::info!("Completed {} syscalls", i as u32);
        }
    }
    pw_log::info!("All 100 syscalls completed successfully!");

    // Signal test passed and exit
    pw_log::info!("✅ PASSED: User mode works correctly!");
    let _ = syscall::debug_shutdown(Ok(()));
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

