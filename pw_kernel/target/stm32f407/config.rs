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

//! Static kernel configuration for STM32F407 Discovery board (STM32F407G-DISC1).
//!
//! The STM32F407VGT6 is a Cortex-M4 based microcontroller with FPU running at
//! up to 168 MHz with 1MB Flash and 192KB SRAM.
//!
//! Key characteristics:
//! - Architecture: ARMv7E-M (Cortex-M4 with FPU)
//! - MPU: PMSAv7 (8 regions)
//! - FPU: Single-precision (not used by kernel)
//! - Flash: 1MB at 0x08000000
//! - SRAM: 192KB total (112KB + 16KB + 64KB CCM)
//! - Debug: On-board ST-Link/V2 with SWO trace

#![no_std]

pub use kernel_config::{
    CortexMKernelConfigInterface, KernelConfigInterface, NvicConfigInterface,
};

pub struct KernelConfig;

impl CortexMKernelConfigInterface for KernelConfig {
    /// SysTick clock frequency in Hz.
    /// STM32F407 with HSE (8 MHz crystal) and PLL configured for 168 MHz.
    /// SysTick uses the processor clock (AHB) which is 168 MHz.
    /// Can also use external clock (AHB/8 = 21 MHz) but we use core clock.
    const SYS_TICK_HZ: u32 = 168_000_000;

    /// Number of MPU regions available.
    /// ARM Cortex-M4 with PMSAv7 has 8 regions.
    const NUM_MPU_REGIONS: usize = 8;
}

impl KernelConfigInterface for KernelConfig {
    /// Scheduler tick rate: 10,000 Hz (100µs tick) for stress testing.
    /// Default is 100 Hz. This gives 100x more context switches to
    /// increase the chance of hitting timing-sensitive races.
    const SCHEDULER_TICK_HZ: u32 = 10_000;

    /// System clock frequency in Hz.
    const SYSTEM_CLOCK_HZ: u64 = KernelConfig::SYS_TICK_HZ as u64;
}

pub struct NvicConfig;

impl NvicConfigInterface for NvicConfig {
    /// STM32F407 has 82 maskable interrupt channels.
    const MAX_IRQS: u32 = 82;
}
