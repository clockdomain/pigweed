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

//! Static kernel configuration for ASPEED AST1030 target.
//!
//! The AST1030 is a Cortex-M4 based BMC SoC running at 200 MHz with 768KB SRAM.
//! For QEMU emulation, we use 12 MHz (LM3S6965EVB compatible clock) since QEMU's
//! ast1030-evb machine uses the LM3S6965 SysTick implementation.

#![no_std]

pub use kernel_config::{
    CortexMKernelConfigInterface, KernelConfigInterface, NvicConfigInterface,
};

pub struct KernelConfig;

impl CortexMKernelConfigInterface for KernelConfig {
    /// SysTick clock frequency in Hz.
    /// Using 12 MHz for QEMU compatibility (LM3S6965EVB SysTick clock).
    /// Real AST1030 hardware runs at 200 MHz.
    const SYS_TICK_HZ: u32 = 12_000_000;

    /// Number of MPU regions available.
    /// ARM Cortex-M4 with PMSAv7 has 8 regions.
    const NUM_MPU_REGIONS: usize = 8;
}

impl KernelConfigInterface for KernelConfig {
    /// System clock frequency in Hz.
    const SYSTEM_CLOCK_HZ: u64 = KernelConfig::SYS_TICK_HZ as u64;
}

pub struct NvicConfig;

// Uses the default configuration (480 interrupts).
impl NvicConfigInterface for NvicConfig {}
