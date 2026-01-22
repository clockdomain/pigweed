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
#![no_std]

#[cfg(feature = "arch_arm_cortex_m")]
use cortex_m_semihosting::hio::hstdout;
use pw_status::{Error, Result};
#[cfg(feature = "arch_riscv")]
use riscv_semihosting::hio::hstdout;

/// Check if interrupts are disabled (PRIMASK=1 on ARM Cortex-M).
///
/// Semihosting requires interrupts to be enabled to complete properly.
/// When called with interrupts disabled, semihosting blocks indefinitely.
#[cfg(feature = "arch_arm_cortex_m")]
#[inline]
fn interrupts_disabled() -> bool {
    // Note: is_active() means "exceptions are active" (interrupts ENABLED),
    // not "PRIMASK is active". We need is_inactive() to detect when
    // interrupts are disabled (PRIMASK bit set).
    cortex_m::register::primask::read().is_inactive()
}

/// RISC-V implementation - check machine interrupt enable bit.
/// TODO: Implement proper RISC-V interrupt state check if needed.
#[cfg(feature = "arch_riscv")]
#[inline]
fn interrupts_disabled() -> bool {
    // For now, always allow logging on RISC-V.
    // If RISC-V semihosting has similar issues, this can be updated
    // to check the mstatus.MIE bit.
    false
}

#[unsafe(no_mangle)]
pub fn console_backend_write_all(buf: &[u8]) -> Result<()> {
    // Skip semihosting if interrupts are disabled to prevent blocking.
    // Semihosting requires debugger/QEMU interaction which may depend on
    // interrupts being enabled. Logging with PRIMASK=1 causes hangs.
    if interrupts_disabled() {
        return Ok(());
    }

    let mut stdout = hstdout().map_err(|_| Error::Unavailable)?;
    stdout.write_all(buf).map_err(|_| Error::DataLoss)?;
    Ok(())
}
