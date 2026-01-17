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

//! MSR (Move to Special Register) definitions
//!
//! This module contains CONTROL register definitions, conditionally
//! including architecture-specific variants:
//! - ARMv7-M: Bits 0-2 only (nPRIV, SPSEL, FPCA)
//! - ARMv8-M: Bits 0-7 (adds SFPA, BTI, PAC fields)

/// Stack-pointer selection (shared between architectures)
#[allow(dead_code)]
#[repr(u32)]
pub enum Spsel {
    Main = 0,
    Process = 1,
}

// Architecture-specific register definitions
#[cfg(feature = "mpu_v7")]
mod msr_v7;
#[cfg(feature = "mpu_v7")]
pub use msr_v7::*;

#[cfg(feature = "mpu_v8")]
mod msr_v8;
#[cfg(feature = "mpu_v8")]
pub use msr_v8::*;
