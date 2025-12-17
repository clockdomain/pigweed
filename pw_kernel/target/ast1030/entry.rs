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

use arch_arm_cortex_m::Arch;
use core::ptr::{read_volatile, write_volatile};

/// AST1030-specific hardware initialization.
/// This runs before the Rust runtime setup and the main kernel entry.
/// Based on aspeed-rust's pre_init implementation.
///
/// Key setup:
/// 1. JTAG pinmux configuration for debug access
/// 2. AST1030 cache controller initialization (NOT Cortex-M4 ACTLR cache)
#[cortex_m_rt::pre_init]
unsafe fn pre_init() {
    // SAFETY: This function is called once during boot before any other code runs.
    // The register accesses are to valid hardware registers on AST1030.
    unsafe {
        // Configure JTAG pinmux
        // Register offset: 0x7e6e2000 (SCU base) + 0x41c
        const JTAG_PINMUX_REG: u32 = 0x7e6e_241c;
        let mut reg = read_volatile(JTAG_PINMUX_REG as *const u32);
        reg |= 0x1f << 25;  // Enable JTAG pins
        write_volatile(JTAG_PINMUX_REG as *mut u32, reg);

        // AST1030 Cache Controller Configuration
        // Note: AST1030 has its own cache controller, NOT the standard ARM Cortex-M4 cache
        
        // Disable cache before configuration
        const CACHE_CTRL: u32 = 0x7e6e_2a58;
        write_volatile(CACHE_CTRL as *mut u32, 0);

        // Configure cache area (full range)
        const CACHE_AREA: u32 = 0x7e6e_2a50;
        write_volatile(CACHE_AREA as *mut u32, 0x000f_ffff);

        // Invalidate cache
        const CACHE_INVAL: u32 = 0x7e6e_2a54;
        write_volatile(CACHE_INVAL as *mut u32, 0x8660_0000);

        // Enable cache
        write_volatile(CACHE_CTRL as *mut u32, 1);
    }
}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "C" fn pw_assert_HandleFailure() -> ! {
    use kernel::Arch as _;
    Arch::panic()
}

#[cortex_m_rt::entry]
fn main() -> ! {
    kernel::static_init_state!(static mut INIT_STATE: InitKernelState<Arch>);

    // SAFETY: `main` is only executed once, so we never generate more than one
    // `&mut` reference to `INIT_STATE`.
    #[allow(static_mut_refs)]
    kernel::main(Arch, unsafe { &mut INIT_STATE });
}
