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

use core::mem::size_of;

use app_initiator::handle;
use pw_status::{Error, Result};
use userspace::time::Instant;
use userspace::{entry, syscall};

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

// Simple logging shims: on ARMv7-M we disable verbose pw_log! usage
// in this test to avoid exercising complex formatter/codegen paths
// that currently generate unaligned multi-word loads.
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

fn test_uppercase_ipcs() -> Result<()> {
    test_log_info!("Ipc test starting");
    for c in 'a'..='z' {
        const SEND_BUF_LEN: usize = size_of::<char>();
        const RECV_BUF_LEN: usize = size_of::<char>() * 2;

        let mut send_buf = AlignedBuf::<SEND_BUF_LEN> { buf: [0u8; SEND_BUF_LEN] };
        let mut recv_buf = AlignedBuf::<RECV_BUF_LEN> { buf: [0u8; RECV_BUF_LEN] };

        // Encode the character into `send_buf` and send it over to the handler.
        c.encode_utf8(send_buf.as_bytes_mut());
        let len = syscall::channel_transact(
            handle::IPC,
            send_buf.as_bytes(),
            recv_buf.as_bytes_mut(),
            Instant::MAX,
        )?;

        // The handler side always sends 8 bytes to make up two full Rust `char`s.
        if len != RECV_BUF_LEN {
            test_log_error!(
                "Received {} bytes, {} expected",
                len,
                RECV_BUF_LEN,
            );
            return Err(Error::OutOfRange);
        }

        let (char0_bytes, char1_bytes) = recv_buf
            .as_bytes()
            .split_at(size_of::<char>());

        // Decode first char.
        let Ok(char0) = u32::from_ne_bytes(char0_bytes.try_into().unwrap()).try_into() else {
            return Err(Error::InvalidArgument);
        };
        let char0: char = char0;

        // Decode second char.
        let Ok(char1) = u32::from_ne_bytes(char1_bytes.try_into().unwrap()).try_into() else {
            return Err(Error::InvalidArgument);
        };
        let char1: char = char1;

        // Log the response character.
        test_log_info!(
            "Sent {}, received ({},{})",
            c,
            char0,
            char1,
        );

        // Verify that the remote side made the first character uppercase.
        if char0 != c.to_ascii_uppercase() {
            return Err(Error::Unknown);
        }

        // Verify that the remote side left the second character lowercase.
        if char1 != c {
            return Err(Error::Unknown);
        }
    }

    Ok(())
}

#[entry]
fn entry() -> ! {
    test_log_info!("🔄 RUNNING");

    let ret = test_uppercase_ipcs();

    // Log that an error occurred so that the app that caused the shutdown is logged.
    if ret.is_err() {
        test_log_error!("❌ FAILED: {}", ret.status_code() as u32);
    } else {
        test_log_info!("✅ PASSED");
    }

    // Since this is written as a test, shut down with the return status from `main()`.
    let _ = syscall::debug_shutdown(ret);
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
