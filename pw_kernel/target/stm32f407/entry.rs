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

//! Entry point for STM32F407 Discovery board (STM32F407G-DISC1).

#![no_std]
#![no_main]

use arch_arm_cortex_m::Arch;
use core::ptr::{read_volatile, write_volatile};

/// Configure the STM32F407 clock system for 168 MHz operation.
///
/// Clock tree: HSE (8 MHz crystal) -> PLL -> SYSCLK (168 MHz)
///   AHB  (HCLK):  168 MHz (/1)
///   APB1 (PCLK1):  42 MHz (/4)
///   APB2 (PCLK2):  84 MHz (/2)
unsafe fn init_clocks() {
    const RCC_CR: *mut u32 = 0x4002_3800 as *mut u32;
    const RCC_PLLCFGR: *mut u32 = 0x4002_3804 as *mut u32;
    const RCC_CFGR: *mut u32 = 0x4002_3808 as *mut u32;
    const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
    const PWR_CR: *mut u32 = 0x4000_7000 as *mut u32;
    const FLASH_ACR: *mut u32 = 0x4002_3C00 as *mut u32;
    const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;

    unsafe {
        // 1. Enable PWR peripheral clock
        let apb1enr = read_volatile(RCC_APB1ENR);
        write_volatile(RCC_APB1ENR, apb1enr | (1 << 28));

        // 2. Set voltage scaling to Scale 1 (required for 168 MHz)
        let pwr_cr = read_volatile(PWR_CR);
        write_volatile(PWR_CR, pwr_cr | (1 << 14));

        // 3. Enable HSE oscillator (8 MHz crystal on Discovery board)
        let cr = read_volatile(RCC_CR);
        write_volatile(RCC_CR, cr | (1 << 16)); // HSEON
        while read_volatile(RCC_CR) & (1 << 17) == 0 {} // Wait HSERDY

        // 4. Configure PLL: HSE / 8 * 336 / 2 = 168 MHz
        let pllcfgr: u32 = 8      // PLLM = 8   -> VCO input  = 1 MHz
            | (336 << 6)          // PLLN = 336 -> VCO output = 336 MHz
            | (0 << 16)           // PLLP = 0   -> /2, SYSCLK = 168 MHz
            | (1 << 22)           // PLLSRC     -> HSE
            | (7 << 24);          // PLLQ = 7   -> USB = 48 MHz
        write_volatile(RCC_PLLCFGR, pllcfgr);

        // 5. Enable PLL
        let cr = read_volatile(RCC_CR);
        write_volatile(RCC_CR, cr | (1 << 24)); // PLLON
        while read_volatile(RCC_CR) & (1 << 25) == 0 {} // Wait PLLRDY

        // 6. Set Flash latency to 5 WS (required at 168 MHz / 3.3V)
        //    Also enable prefetch, instruction cache, and data cache.
        write_volatile(FLASH_ACR, 5 | (1 << 8) | (1 << 9) | (1 << 10));

        // 7. Configure bus dividers: AHB /1, APB1 /4, APB2 /2
        //    Keep SW = HSI (0b00) for now.
        let cfgr: u32 = (0b0000 << 4) // HPRE  = /1
            | (0b101 << 10)           // PPRE1 = /4
            | (0b100 << 13);          // PPRE2 = /2
        write_volatile(RCC_CFGR, cfgr);

        // 8. Switch system clock to PLL
        let cfgr = read_volatile(RCC_CFGR);
        write_volatile(RCC_CFGR, (cfgr & !0b11) | 0b10); // SW = PLL
        while (read_volatile(RCC_CFGR) >> 2) & 0b11 != 0b10 {} // Wait SWS = PLL

        // 9. Set SysTick to use processor clock (168 MHz) to match SYS_TICK_HZ.
        //    Bit 2 (CLKSOURCE): 0 = external ref (HCLK/8), 1 = processor clock.
        //    systick_early_init preserves CLKSOURCE via read-modify-write.
        let csr = read_volatile(SYST_CSR);
        write_volatile(SYST_CSR, csr | (1 << 2));
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
    // SAFETY: Called once at boot before any kernel code runs.
    // Configures PLL for 168 MHz and sets SysTick clock source.
    unsafe { init_clocks() };

    kernel::static_init_state!(static mut INIT_STATE: InitKernelState<Arch>);

    // SAFETY: `main` is only executed once, so we never generate more than one
    // `&mut` reference to `INIT_STATE`.
    #[allow(static_mut_refs)]
    kernel::main(Arch, unsafe { &mut INIT_STATE });
}
