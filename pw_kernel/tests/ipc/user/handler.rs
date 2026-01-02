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

fn handle_uppercase_ipcs() -> Result<()> {
    pw_log::info!("IPC service starting");
    loop {
        // Wait for an IPC to come in.
        syscall::object_wait(handle::IPC, Signals::READABLE, Instant::MAX)?;

        // Read the payload.
        const RECV_LEN: usize = size_of::<char>();
        let mut buffer = AlignedBuf::<RECV_LEN> { buf: [0u8; RECV_LEN] };
        let len = syscall::channel_read(handle::IPC, 0, buffer.as_bytes_mut())?;
        if len != RECV_LEN {
            return Err(Error::OutOfRange);
        };

        // Convert the payload to a character and make it uppercase.
        let Some(c) = char::from_u32(u32::from_ne_bytes(buffer.as_bytes().try_into().unwrap())) else {
            return Err(Error::InvalidArgument);
        };
        let upper_c = c.to_ascii_uppercase();

        // Respond to the IPC with the uppercase character.
        const RESP_LEN: usize = size_of::<char>() * 2;
        let mut response_buffer = AlignedBuf::<RESP_LEN> { buf: [0u8; RESP_LEN] };
        {
            let buf = response_buffer.as_bytes_mut();
            let (first, second) = buf.split_at_mut(size_of::<char>());
            upper_c.encode_utf8(first);
            c.encode_utf8(second);
        }
        syscall::channel_respond(handle::IPC, response_buffer.as_bytes())?;
    }
}

#[entry]
fn entry() -> ! {
    if let Err(e) = handle_uppercase_ipcs() {
        // On error, log that it occurred and, since this is written as a test,
        // shut down the system with the error code.
        pw_log::error!("IPC service error: {}", e as u32);
        let _ = syscall::debug_shutdown(Err(e));
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
