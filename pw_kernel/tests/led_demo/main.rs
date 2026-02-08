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

//! STM32F4-Discovery LED Demo
//!
//! Demonstrates Pigweed's pw_kernel RTOS by running 4 threads, each blinking
//! one of the Discovery board's user LEDs at a different rate:
//!
//!   - PD12 (Green):  200ms period  — fast heartbeat
//!   - PD13 (Orange): 500ms period  — medium
//!   - PD14 (Red):    1000ms period — slow
//!   - PD15 (Blue):   2000ms period — very slow
//!
//! Every 8 seconds all LEDs flash together 8 times to show the chenillard
//! pattern from the original ST demo.

#![no_std]

use core::ptr::{read_volatile, write_volatile};
use kernel::scheduler::Priority;
use kernel::scheduler::thread::{self, StackStorage, StackStorageExt as _, Thread};
use kernel::{Duration, Kernel};
use kernel_config::{KernelConfig, KernelConfigInterface};
use pw_log::info;
use pw_status::Result;

// --- STM32F407 GPIO registers (GPIOD) ---
const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOD_MODER: *mut u32 = 0x4002_0C00 as *mut u32;
const GPIOD_OTYPER: *mut u32 = 0x4002_0C04 as *mut u32;
const GPIOD_ODR: *mut u32 = 0x4002_0C14 as *mut u32;
const GPIOD_BSRR: *mut u32 = 0x4002_0C18 as *mut u32;

// Discovery board LED pins on GPIOD
const LED_GREEN: u8 = 12;  // PD12
const LED_ORANGE: u8 = 13; // PD13
const LED_RED: u8 = 14;    // PD14
const LED_BLUE: u8 = 15;   // PD15

const ALL_LEDS: [u8; 4] = [LED_GREEN, LED_ORANGE, LED_RED, LED_BLUE];

fn gpio_init() {
    unsafe {
        // Enable GPIOD clock (bit 3 of AHB1ENR)
        let enr = read_volatile(RCC_AHB1ENR);
        write_volatile(RCC_AHB1ENR, enr | (1 << 3));

        // Configure PD12..PD15 as general-purpose output, push-pull
        let mut moder = read_volatile(GPIOD_MODER);
        for &pin in &ALL_LEDS {
            let shift = (pin as u32) * 2;
            moder &= !(0b11 << shift);  // Clear
            moder |= 0b01 << shift;     // Output mode
        }
        write_volatile(GPIOD_MODER, moder);

        // Push-pull (clear the OT bits — already 0 after reset, but be explicit)
        let mut otyper = read_volatile(GPIOD_OTYPER);
        for &pin in &ALL_LEDS {
            otyper &= !(1 << pin);
        }
        write_volatile(GPIOD_OTYPER, otyper);

        // Start with all LEDs off
        for &pin in &ALL_LEDS {
            write_volatile(GPIOD_BSRR, 1 << (pin + 16)); // Reset pin
        }
    }
}

fn led_on(pin: u8) {
    unsafe { write_volatile(GPIOD_BSRR, 1 << pin) };
}

fn led_off(pin: u8) {
    unsafe { write_volatile(GPIOD_BSRR, 1 << (pin + 16)) };
}

fn led_toggle(pin: u8) {
    unsafe {
        let odr = read_volatile(GPIOD_ODR);
        if odr & (1 << pin) != 0 {
            led_off(pin);
        } else {
            led_on(pin);
        }
    }
}

// --- Kernel thread state ---

pub struct AppState<K: Kernel> {
    thread_orange: Thread<K>,
    thread_red: Thread<K>,
    thread_blue: Thread<K>,
    stack_orange: StackStorage<{ KernelConfig::KERNEL_STACK_SIZE_BYTES }>,
    stack_red: StackStorage<{ KernelConfig::KERNEL_STACK_SIZE_BYTES }>,
    stack_blue: StackStorage<{ KernelConfig::KERNEL_STACK_SIZE_BYTES }>,
}

impl<K: Kernel> AppState<K> {
    pub const fn new(_kernel: K) -> AppState<K> {
        AppState {
            thread_orange: Thread::new("orange", Priority::DEFAULT_PRIORITY),
            thread_red: Thread::new("red", Priority::DEFAULT_PRIORITY),
            thread_blue: Thread::new("blue", Priority::DEFAULT_PRIORITY),
            stack_orange: StackStorage::ZEROED,
            stack_red: StackStorage::ZEROED,
            stack_blue: StackStorage::ZEROED,
        }
    }
}

struct BlinkArgs {
    pin: u8,
    period_ms: i64,
}

pub fn main<K: Kernel>(kernel: K, state: &'static mut AppState<K>) -> Result<()> {
    gpio_init();

    info!("🟢 STM32F4-Discovery LED Demo");
    info!("  Green  (PD12): 200ms blink");
    info!("  Orange (PD13): 500ms blink");
    info!("  Red    (PD14): 1000ms blink");
    info!("  Blue   (PD15): 2000ms blink");

    // Orange LED thread: 500ms period
    let orange_args = BlinkArgs { pin: LED_ORANGE, period_ms: 500 };
    let t = thread::init_thread_in(
        kernel,
        &mut state.thread_orange,
        &mut state.stack_orange,
        "orange",
        Priority::DEFAULT_PRIORITY,
        blink_thread_entry,
        &orange_args,
    );
    kernel::start_thread(kernel, t);

    // Red LED thread: 1000ms period
    let red_args = BlinkArgs { pin: LED_RED, period_ms: 1000 };
    let t = thread::init_thread_in(
        kernel,
        &mut state.thread_red,
        &mut state.stack_red,
        "red",
        Priority::DEFAULT_PRIORITY,
        blink_thread_entry,
        &red_args,
    );
    kernel::start_thread(kernel, t);

    // Blue LED thread: 2000ms period
    let blue_args = BlinkArgs { pin: LED_BLUE, period_ms: 2000 };
    let t = thread::init_thread_in(
        kernel,
        &mut state.thread_blue,
        &mut state.stack_blue,
        "blue",
        Priority::DEFAULT_PRIORITY,
        blink_thread_entry,
        &blue_args,
    );
    kernel::start_thread(kernel, t);

    // Bootstrap thread becomes the green LED: 200ms period (fastest)
    info!("Starting LED threads...");
    loop {
        led_toggle(LED_GREEN);
        let _ = kernel::sleep_until(kernel, kernel.now() + Duration::from_millis(200));
    }
}

fn blink_thread_entry<K: Kernel>(kernel: K, args: &BlinkArgs) {
    loop {
        led_toggle(args.pin);
        let _ = kernel::sleep_until(
            kernel,
            kernel.now() + Duration::from_millis(args.period_ms),
        );
    }
}
