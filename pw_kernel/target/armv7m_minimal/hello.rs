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

//! Minimal ARMv7-M test process - just proves the system boots

#![no_std]
#![no_main]

use userspace as kernel;

#[no_mangle]
fn main() -> ! {
    // Simple loop to prove we booted
    let mut counter: u32 = 0;
    loop {
        counter = counter.wrapping_add(1);
        
        // Exit after a few iterations
        if counter > 1000 {
            unsafe {
                kernel::shutdown(0);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        kernel::shutdown(1);
    }
}
