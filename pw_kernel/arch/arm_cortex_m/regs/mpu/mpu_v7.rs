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

//! PMSAv7 (ARMv7-M) MPU register definitions
//!
//! This module contains register definitions specific to the PMSAv7
//! memory protection architecture used in ARMv7-M processors
//! (Cortex-M3, Cortex-M4, Cortex-M7).

#![allow(dead_code)]

use regs::*;

/// PMSAv7 Region Base Address Register value
#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub struct RbarVal(pub u32);

impl RbarVal {
    pub const fn const_default() -> Self {
        Self(0)
    }

    rw_bool_field!(u32, valid, 4, "MPU region number valid");

    /// Extract region field (used when VALID bit is set).
    pub const fn region(&self) -> u8 {
        #[expect(clippy::cast_possible_truncation)]
        (ops::get_u32(self.0, 0, 3) as u8)
    }

    /// Update region field.
    pub const fn with_region(self, val: u8) -> Self {
        Self(ops::set_u32(self.0, 0, 3, val as u32))
    }

    rw_masked_field!(addr, 0xffff_ffe0, u32, "region base address");
}

rw_reg!(
    Rbar,
    RbarVal,
    u32,
    0xe000ed9c,
    "MPU Region Base Address Register (PMSAv7)"
);

/// PMSAv7 access permissions
#[repr(u8)]
pub enum RasrAp {
    NoAccess = 0b000,
    RwPrivileged = 0b001,
    RoPrivileged = 0b010,
    RwAny = 0b011,
    Reserved1 = 0b100,
    RoPrivileged2 = 0b101,
    RoAny = 0b110,
    RoAny2 = 0b111,
}

/// PMSAv7 TEX/S/C/B memory attribute combinations
#[repr(u8)]
pub enum RasrTexScb {
    /// Strongly-ordered, shareable
    StronglyOrdered = 0b00000,
    /// Device, shareable
    Device = 0b00001,
    /// Normal, write-through, no write allocate
    NormalWriteThrough = 0b00010,
    /// Normal, write-back, no write allocate
    NormalWriteBack = 0b00011,
    /// Normal, non-cacheable
    NormalNonCacheable = 0b01000,
    /// Normal, write-back, write and read allocate
    NormalWriteBackAllocate = 0b01011,
    /// Device, not shareable
    DeviceNonShareable = 0b10000,
}

/// PMSAv7 Region Attribute and Size Register value
#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub struct RasrVal(pub u32);

impl RasrVal {
    pub const fn const_default() -> Self {
        Self(0)
    }

    rw_bool_field!(u32, enable, 0, "region enable");

    /// Extract region size field (SIZE).
    /// Region size is 2^(SIZE+1) bytes, so SIZE=4 means 32 bytes, SIZE=31 means 4GB.
    pub const fn size(&self) -> u8 {
        #[expect(clippy::cast_possible_truncation)]
        (ops::get_u32(self.0, 1, 5) as u8)
    }

    /// Update region size field.
    pub const fn with_size(self, val: u8) -> Self {
        Self(ops::set_u32(self.0, 1, 5, val as u32))
    }

    /// Extract sub-region disable field (SRD).
    pub const fn srd(&self) -> u8 {
        #[expect(clippy::cast_possible_truncation)]
        (ops::get_u32(self.0, 8, 15) as u8)
    }

    /// Update sub-region disable field.
    pub const fn with_srd(self, val: u8) -> Self {
        Self(ops::set_u32(self.0, 8, 15, val as u32))
    }

    rw_bool_field!(u32, b, 16, "bufferable");
    rw_bool_field!(u32, c, 17, "cacheable");
    rw_bool_field!(u32, s, 18, "shareable");

    /// Extract TEX (Type Extension) field.
    pub const fn tex(&self) -> u8 {
        #[expect(clippy::cast_possible_truncation)]
        (ops::get_u32(self.0, 19, 21) as u8)
    }

    /// Update TEX field.
    pub const fn with_tex(self, val: u8) -> Self {
        Self(ops::set_u32(self.0, 19, 21, val as u32))
    }

    /// Extract access permissions field.
    pub const fn ap(&self) -> RasrAp {
        // Safety: Value is masked to only contain valid enum values.
        #[expect(clippy::cast_possible_truncation)]
        unsafe {
            core::mem::transmute(ops::get_u32(self.0, 24, 26) as u8)
        }
    }

    /// Update access permissions field.
    pub const fn with_ap(self, val: RasrAp) -> Self {
        Self(ops::set_u32(self.0, 24, 26, val as u32))
    }

    rw_bool_field!(u32, xn, 28, "execute-never");
}

rw_reg!(
    Rasr,
    RasrVal,
    u32,
    0xe000eda0,
    "MPU Region Attribute and Size Register (PMSAv7)"
);
