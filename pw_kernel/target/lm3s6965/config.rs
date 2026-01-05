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

//! Static kernel configuration for ARMv7-M minimal target.

#![no_std]

pub use kernel_config::{CortexMKernelConfigInterface, KernelConfigInterface, NvicConfigInterface};

/// Static kernel configuration.
///
/// This provides compile-time constants for the kernel, independent of the
/// system-specific configuration generated from system.json5.
pub struct KernelConfig;

impl CortexMKernelConfigInterface for KernelConfig {
    /// System tick frequency in Hz.
    /// Using 12 MHz for QEMU LM3S6965 compatibility.
    const SYS_TICK_HZ: u32 = 12_000_000;

    /// Number of MPU regions available.
    /// ARMv7-M (PMSAv7) has 8 regions.
    const NUM_MPU_REGIONS: usize = 8;
}

impl KernelConfigInterface for KernelConfig {
    /// System clock frequency in Hz.
    const SYSTEM_CLOCK_HZ: u64 = KernelConfig::SYS_TICK_HZ as u64;
}

pub struct NvicConfig;

impl NvicConfigInterface for NvicConfig {
    const MAX_IRQS: u32 = 64;
}
