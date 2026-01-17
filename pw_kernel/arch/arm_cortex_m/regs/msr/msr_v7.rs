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

//! ARMv7-M CONTROL register definition
//!
//! Per ARMv7-M ARM (DDI 0403E.e) Section B1.4.4, the CONTROL register
//! on ARMv7-M only defines bits 0-2. Bits 31:3 are reserved RAZ/WI.

use pw_cast::CastFrom as _;
use regs::*;

use super::Spsel;

macro_rules! rw_msr_reg {
    ($name:ident, $val_type:ident, $reg_name:ident, $doc:literal) => {
        #[doc=$doc]
        pub struct $name;
        impl $name {
            #[inline]
            pub fn read() -> $val_type {
                let mut val: usize;
                unsafe {
                    core::arch::asm!(concat!("mrs {0}, ", stringify!($reg_name)), out(reg) val)
                };
                $val_type(u32::cast_from(val))
            }

            #[allow(dead_code)]
            #[inline]
            pub fn write(val: $val_type) {
                unsafe {
                    core::arch::asm!(
                        concat!("msr ", stringify!($reg_name), ", {0}"),
                        in(reg) val.0)
                };
            }
        }
    };
}

#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub struct ControlVal(pub u32);

impl ControlVal {
    rw_bool_field!(u32, npriv, 0, "non privileged");
    rw_enum_field!(u32, spsel, 1, 1, Spsel, "stack-pointer select");
    rw_bool_field!(u32, fpca, 2, "floating-point context active");
    // Bits 31:3 are reserved on ARMv7-M (RAZ/WI)
}

rw_msr_reg!(Control, ControlVal, control, "Control Register");
