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

//! I/O futures for pw_kernel userspace.
//!
//! Bridges kernel `object_wait` into Rust async by providing futures that
//! poll kernel object signals via non-blocking `object_wait` calls.
//!
//! ```rust,ignore
//! // Wait for a channel to become readable:
//! let wr = reactor::object_wait(handle::IPC, Signals::READABLE).await?;
//!
//! // Wait for an interrupt and auto-acknowledge:
//! let signals = reactor::wait_interrupt(handle::IRQ, signals::MY_IRQ).await?;
//! ```

#![no_std]

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use pw_status::{Error, Result};
use userspace::syscall::{self, Signals, WaitReturn};
use userspace::time::Instant;

/// Future that waits for signals on a kernel object.
///
/// Polls the object with a non-blocking `object_wait` (deadline = `Instant::MIN`).
/// Completes when any signal in the mask becomes pending.
pub struct ObjectWaitFuture {
    handle: u32,
    signal_mask: Signals,
}

impl Future for ObjectWaitFuture {
    type Output = Result<WaitReturn>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match syscall::object_wait(self.handle, self.signal_mask, Instant::MIN) {
            Ok(wait_return) => Poll::Ready(Ok(wait_return)),
            Err(Error::DeadlineExceeded) => {
                // Not ready yet — re-poll on next executor iteration.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Wait for signals on a kernel object handle.
///
/// Returns when any signal in `signal_mask` becomes pending on the object.
pub fn object_wait(handle: u32, signal_mask: Signals) -> ObjectWaitFuture {
    ObjectWaitFuture {
        handle,
        signal_mask,
    }
}

/// Future that waits for an interrupt signal and acknowledges it.
///
/// Polls the interrupt object with a non-blocking `object_wait`. When the
/// signal fires, automatically calls `interrupt_ack` to clear and re-enable
/// the interrupt at the hardware level.
pub struct InterruptFuture {
    handle: u32,
    signal_mask: Signals,
}

impl Future for InterruptFuture {
    type Output = Result<Signals>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match syscall::object_wait(self.handle, self.signal_mask, Instant::MIN) {
            Ok(wait_return) => {
                let _ = syscall::interrupt_ack(self.handle, wait_return.pending_signals);
                Poll::Ready(Ok(wait_return.pending_signals))
            }
            Err(Error::DeadlineExceeded) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Wait for an interrupt signal and auto-acknowledge it.
///
/// Returns the pending signals when the interrupt fires. The interrupt is
/// acknowledged automatically so it can fire again.
pub fn wait_interrupt(handle: u32, signal_mask: Signals) -> InterruptFuture {
    InterruptFuture {
        handle,
        signal_mask,
    }
}
