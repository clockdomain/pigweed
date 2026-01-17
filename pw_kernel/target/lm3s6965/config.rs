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

//! Static kernel configuration for TI Stellaris LM3S6965 target.
//!
//! The LM3S6965 is a Cortex-M3 based microcontroller running at 50 MHz with
//! 256KB Flash and 64KB SRAM. QEMU provides excellent emulation via the
//! `lm3s6965evb` machine type.
//!
//! Key characteristics:
//! - Architecture: ARMv7-M (Cortex-M3)
//! - MPU: PMSAv7 (8 regions)
//! - FPU: None (Cortex-M3 has no FPU)
//! - QEMU Machine: `lm3s6965evb`

#![no_std]

pub use kernel_config::{
    CortexMKernelConfigInterface, KernelConfigInterface, NvicConfigInterface,
};

pub struct KernelConfig;

impl CortexMKernelConfigInterface for KernelConfig {
    /// SysTick clock frequency in Hz.
    /// The LM3S6965 runs at 50 MHz with default QEMU settings.
    /// However, QEMU's lm3s6965evb uses 12 MHz for the SysTick.
    const SYS_TICK_HZ: u32 = 12_000_000;

    /// Number of MPU regions available.
    /// ARM Cortex-M3 with PMSAv7 has 8 regions.
    const NUM_MPU_REGIONS: usize = 8;
}

impl KernelConfigInterface for KernelConfig {
    /// System clock frequency in Hz.
    const SYSTEM_CLOCK_HZ: u64 = KernelConfig::SYS_TICK_HZ as u64;
}

pub struct NvicConfig;

// Uses the default configuration (480 interrupts).
impl NvicConfigInterface for NvicConfig {}
