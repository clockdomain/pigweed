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

//! STM32F407 Discovery LED Demo Target
//!
//! Runs 4 kernel threads, each blinking one of the Discovery board LEDs
//! at a different rate.

#![no_std]
#![no_main]

use arch_arm_cortex_m::Arch;
use target_common::{TargetInterface, declare_target};
use {console_backend as _, entry as _};

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "STM32F407 LED Demo";

    fn console_init() {
        console_backend::init();
    }

    fn main() -> ! {
        static mut APP_STATE: led_demo::AppState<Arch> = led_demo::AppState::new(Arch);
        // SAFETY: `main` is only executed once, so we never generate more
        // than one `&mut` reference to `APP_STATE`.
        #[expect(static_mut_refs)]
        let _ = led_demo::main(Arch, unsafe { &mut APP_STATE });
        // LED demo runs forever, but if it somehow returns:
        #[expect(clippy::empty_loop)]
        loop {}
    }
}

declare_target!(Target);
