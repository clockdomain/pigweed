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
#![no_main]
#![no_std]

use app_handler::handle;
use pw_status::{Error, Result};
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

/// 4-byte aligned byte buffer to keep ARMv7-M from faulting on
/// compiler-generated multi-word loads (e.g. LDMIA) over IPC buffers.
#[repr(C, align(4))]
struct AlignedBuf<const N: usize> {
    buf: [u8; N],
}

impl<const N: usize> AlignedBuf<N> {
    fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

// Simple logging shims: on ARMv7-M we can disable verbose pw_log! usage
// in this test to avoid exercising complex formatter/codegen paths that
// currently generate unaligned multi-word loads.
#[cfg(target_arch = "arm")]
macro_rules! test_log_info {
    ($($arg:tt)*) => {};
}

#[cfg(not(target_arch = "arm"))]
macro_rules! test_log_info {
    ($($arg:tt)*) => {
        pw_log::info!($($arg)*);
    };
}

#[cfg(target_arch = "arm")]
macro_rules! test_log_error {
    ($($arg:tt)*) => {};
}

#[cfg(not(target_arch = "arm"))]
macro_rules! test_log_error {
    ($($arg:tt)*) => {
        pw_log::error!($($arg)*);
    };
}

fn handle_uppercase_ipcs() -> Result<()> {
    // Emit a simple service-start marker on ARMv7-M so we can
    // confirm the handler thread is running in detokenized logs.
    #[cfg(target_arch = "arm")]
    pw_log::info!("IPC service starting");

    test_log_info!("IPC service starting");
    loop {
        // Wait for an IPC to come in.
        syscall::object_wait(handle::IPC, Signals::READABLE, Instant::MAX)?;

        // Read the payload. The initiator currently sends a single ASCII
        // character encoded via `encode_utf8` into a 4-byte `char` slot,
        // so only the low byte is meaningful.
        const RECV_LEN: usize = core::mem::size_of::<char>();
        let mut buffer = AlignedBuf::<RECV_LEN> { buf: [0; RECV_LEN] };
        let len = syscall::channel_read(handle::IPC, 0, buffer.as_bytes_mut())?;
        if len != RECV_LEN {
            return Err(Error::OutOfRange);
        };

        // Interpret the payload as a 32-bit word whose low byte holds
        // the ASCII character; avoid char/UTF-8 helpers to keep the
        // codegen simple and predictable on ARMv7-M.
        let word = u32::from_ne_bytes(buffer.as_bytes().try_into().unwrap());
        let b = (word & 0xFF) as u8;
        if !b.is_ascii_lowercase() {
            return Err(Error::InvalidArgument);
        }
        let upper_b = b.to_ascii_uppercase();
        let upper_word = (word & !0xFF) | u32::from(upper_b);

        // Respond to the IPC with two 4-byte words: the uppercased
        // character (first) and the original (second).
        const RESP_LEN: usize = core::mem::size_of::<char>() * 2;
        let mut response_buffer = AlignedBuf::<RESP_LEN> { buf: [0; RESP_LEN] };
        {
            let buf = response_buffer.as_bytes_mut();
            let upper_bytes = upper_word.to_ne_bytes();
            let orig_bytes = word.to_ne_bytes();
            // Manual per-byte copies to avoid slice-based memcpy.
            for i in 0..4 {
                buf[i] = upper_bytes[i];
                buf[4 + i] = orig_bytes[i];
            }
        }
        syscall::channel_respond(handle::IPC, response_buffer.as_bytes())?;
    }
}

#[entry]
fn entry() -> ! {
    if let Err(e) = handle_uppercase_ipcs() {
        // On error, log that it occurred and, since this is written as a test,
        // shut down the system with the error code.
        test_log_error!("IPC service error: {}", e as u32);
        let _ = syscall::debug_shutdown(Err(e));
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
