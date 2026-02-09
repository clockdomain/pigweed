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

//! UART console backend for STM32F407 Discovery board.
//!
//! Uses USART2 TX on PA2 (AF7) at 115200 baud. This replaces semihosting
//! to avoid BKPT-related crashes and to allow standalone operation without
//! a debugger.
//!
//! Connect a USB-to-serial adapter to PA2 (TX) and GND. Output appears
//! as tokenized base64 log lines at 115200 8N1.
//!
//! Based on the standalone Rust UART driver for STM32F407 using raw MMIO
//! register access (no PAC crate dependency).

#![no_std]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, Ordering};
use pw_status::Result;

// --- RCC registers ---
const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

// --- GPIOA registers ---
const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_OTYPER: *mut u32 = 0x4002_0004 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;

// --- USART2 registers ---
const USART2_SR: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_DR: *mut u32 = 0x4000_4404 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_440C as *mut u32;

// SR bit masks
const SR_TXE: u32 = 1 << 7; // Transmit data register empty

// CR1 bit masks
const CR1_UE: u32 = 1 << 13; // USART enable
const CR1_TE: u32 = 1 << 3; // Transmitter enable

// APB1 clock after PLL init: 168 MHz / 4 = 42 MHz
const APB1_CLK_HZ: u32 = 42_000_000;
const BAUD: u32 = 115_200;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize USART2 on PA2 (AF7) for TX-only at 115200 baud.
///
/// Safe to call multiple times; only the first call configures hardware.
pub fn init() {
    if INITIALIZED.swap(true, Ordering::Relaxed) {
        return;
    }

    unsafe {
        // Enable GPIOA clock (bit 0 of AHB1ENR)
        let enr = read_volatile(RCC_AHB1ENR);
        write_volatile(RCC_AHB1ENR, enr | (1 << 0));

        // Enable USART2 clock (bit 17 of APB1ENR)
        let enr = read_volatile(RCC_APB1ENR);
        write_volatile(RCC_APB1ENR, enr | (1 << 17));

        // Configure PA2 as alternate function (MODER2 = 0b10)
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 4)) | (0b10 << 4));

        // PA2 push-pull (clear OT2)
        let otyper = read_volatile(GPIOA_OTYPER);
        write_volatile(GPIOA_OTYPER, otyper & !(1 << 2));

        // PA2 → AF7 (USART2_TX): AFRL bits [11:8] = 0x7
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (7 << 8));

        // Set baud rate: BRR = APB1_CLK / BAUD
        // With 16x oversampling, this gives the correct mantissa+fraction.
        let brr = APB1_CLK_HZ / BAUD;
        write_volatile(USART2_BRR, brr);

        // Enable USART2: 8N1, TX only
        write_volatile(USART2_CR1, CR1_UE | CR1_TE);
    }
}

/// Write a single byte, blocking until the TX register is empty.
fn write_byte(byte: u8) {
    unsafe {
        while read_volatile(USART2_SR) & SR_TXE == 0 {}
        write_volatile(USART2_DR, byte as u32);
    }
}

/// Console backend entry point called by the pw_log tokenized logger.
#[unsafe(no_mangle)]
pub fn console_backend_write_all(buf: &[u8]) -> Result<()> {
    if !INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }
    for &byte in buf {
        write_byte(byte);
    }
    Ok(())
}
