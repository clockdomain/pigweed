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

//! Beta process for context switch stress test.
//!
//! This process performs syscall loops to stress test the scheduler
//! and user-mode context switching. Each syscall gives an opportunity
//! for the scheduler to switch to the alpha process.

#![no_main]
#![no_std]

use pw_status::Error;
use userspace::{entry, syscall};

const ITERATIONS: u32 = 100;

#[entry]
fn entry() -> ! {
    pw_log::info!("🅱️ Beta process starting");
    pw_log::info!("Beta: Will perform {} syscall iterations", ITERATIONS as u32);

    let mut completed = 0u32;

    for i in 0..ITERATIONS {
        // Log progress periodically
        if i % 20 == 0 {
            pw_log::info!("🅱️ Beta iteration {}/{}", i as u32, ITERATIONS as u32);
        }

        // Do multiple syscalls to trigger context switches
        for _ in 0..10 {
            let _ = syscall::debug_nop();
        }

        completed += 1;
    }

    pw_log::info!("🅱️ Beta completed all {} iterations!", completed as u32);
    pw_log::info!("✅ Beta: PASSED");

    // Beta enters idle loop after completing - alpha will shutdown the system
    loop {
        let _ = syscall::debug_nop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    pw_log::error!("🅱️ Beta PANIC!");
    let _ = syscall::debug_shutdown(Err(Error::Internal));
    loop {}
}
