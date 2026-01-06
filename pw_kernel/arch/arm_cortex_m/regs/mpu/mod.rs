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

//! MPU (Memory Protection Unit) register definitions
//!
//! This module contains shared MPU register definitions and conditionally
//! includes architecture-specific registers:
//! - PMSAv7 (ARMv7-M): Cortex-M3, Cortex-M4, Cortex-M7
//! - PMSAv8 (ARMv8-M): Cortex-M23, Cortex-M33, Cortex-M55

#![allow(dead_code)]

use regs::*;

// Architecture-specific register definitions
#[cfg(feature = "mpu_v7")]
mod mpu_v7;
#[cfg(feature = "mpu_v7")]
pub use mpu_v7::*;

#[cfg(feature = "mpu_v8")]
mod mpu_v8;
#[cfg(feature = "mpu_v8")]
pub use mpu_v8::*;

/// Memory Protection Unit register bank
pub struct Mpu {
    /// Type Register
    pub _type: Type,

    /// Control Register
    pub ctrl: Ctrl,

    /// Region Number Register
    pub rnr: Rnr,

    /// Region Base Address Register
    pub rbar: Rbar,

    /// Region Limit Address Register (PMSAv8 only)
    #[cfg(feature = "mpu_v8")]
    pub rlar: Rlar,

    /// Region Attribute and Size Register (PMSAv7 only)
    #[cfg(feature = "mpu_v7")]
    pub rasr: Rasr,

    /// Memory Attribute Indirection Register 0 (PMSAv8 only)
    #[cfg(feature = "mpu_v8")]
    pub mair0: Mair0,

    /// Memory Attribute Indirection Register 1 (PMSAv8 only)
    #[cfg(feature = "mpu_v8")]
    pub mair1: Mair1,
}

impl Mpu {
    pub(super) const fn new() -> Self {
        Self {
            _type: Type,
            ctrl: Ctrl,
            rnr: Rnr,
            rbar: Rbar,
            #[cfg(feature = "mpu_v8")]
            rlar: Rlar,
            #[cfg(feature = "mpu_v7")]
            rasr: Rasr,
            #[cfg(feature = "mpu_v8")]
            mair0: Mair0,
            #[cfg(feature = "mpu_v8")]
            mair1: Mair1,
        }
    }
}

// ============================================================================
// Shared registers (identical on PMSAv7 and PMSAv8)
// ============================================================================

/// MPU Type Register value
#[repr(transparent)]
pub struct TypeVal(u32);
impl TypeVal {
    ro_bool_field!(u32, separate, 0, "separate instruction and data regions");
    ro_int_field!(u32, dregion, 8, 15, u8, "number of data regions");
}
ro_reg!(Type, TypeVal, u32, 0xe000ed90, "MPU Type Register");

/// MPU Control Register value
#[repr(transparent)]
pub struct CtrlVal(u32);
impl CtrlVal {
    rw_bool_field!(u32, enable, 0, "enable");
    rw_bool_field!(u32, hfnmiena, 1, "HardFault, NMI enable");
    rw_bool_field!(u32, privdefena, 2, "Privileged default enable");
}
rw_reg!(Ctrl, CtrlVal, u32, 0xe000ed94, "MPU Control Register");

/// MPU Region Number Register value
#[derive(Default)]
#[repr(transparent)]
pub struct RnrVal(u32);
impl RnrVal {
    rw_int_field!(u32, region, 0, 7, u8, "region number");
}
rw_reg!(Rnr, RnrVal, u32, 0xe000ed98, "MPU Region Number Register");
