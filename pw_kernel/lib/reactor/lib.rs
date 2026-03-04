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

//! I/O reactor for pw_kernel userspace.
//!
//! Provides a [`Reactor`] that multiplexes waits across kernel objects using
//! a WaitGroup, and I/O futures that register with it. When no tasks are
//! ready, the executor calls [`Reactor::wait_for_events`] which blocks on
//! the WaitGroup — waking only when a registered object becomes ready.
//!
//! # Setup
//!
//! ```rust,ignore
//! // Initialize the reactor with your app's WaitGroup handle:
//! reactor::REACTOR.init(handle::WAIT_GROUP);
//!
//! // Pass reactor as the executor's idle strategy:
//! EXECUTOR.run(
//!     |spawner| { spawner.spawn(main_task(spawner)).unwrap(); },
//!     || reactor::REACTOR.wait_for_events(),
//! );
//! ```
//!
//! # Usage in tasks
//!
//! ```rust,ignore
//! let wr = reactor::object_wait(handle::IPC, Signals::READABLE).await?;
//! let signals = reactor::wait_interrupt(handle::IRQ, signals::MY_IRQ).await?;
//! ```

#![no_std]

use core::cell::{Cell, UnsafeCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use pw_status::{Error, Result};
use userspace::syscall::{self, Signals, WaitReturn};
use userspace::time::Instant;

// --- Reactor ----------------------------------------------------------------

/// Maximum number of kernel objects that can be registered with the reactor
/// simultaneously.
pub const MAX_REACTOR_SLOTS: usize = 16;

/// Global reactor instance. Initialize with [`Reactor::init`] before starting
/// the executor.
pub static REACTOR: Reactor = Reactor::new();

/// I/O reactor that multiplexes waits across kernel objects via WaitGroup.
///
/// Futures register their kernel object handle with the reactor when they
/// return `Pending`. The executor's idle closure calls [`wait_for_events`]
/// which blocks on the WaitGroup until any registered object becomes ready,
/// then wakes the corresponding task.
///
/// # Safety model
///
/// Interior mutability via `UnsafeCell` is sound because:
/// - The executor is single-threaded and `!Send`
/// - `poll()` is never called reentrantly
/// - `wait_for_events()` is only called when no tasks are being polled
pub struct Reactor {
    wg_handle: Cell<u32>,
    // UnsafeCell because Waker isn't Copy (can't use Cell).
    wakers: [UnsafeCell<Option<Waker>>; MAX_REACTOR_SLOTS],
    used: Cell<u16>,
}

// SAFETY: Reactor is used from a single-threaded, !Send executor.
// The global static requires Sync, but all access is non-concurrent.
unsafe impl Sync for Reactor {}

impl Reactor {
    /// Create an uninitialized reactor. Call [`init`] before use.
    pub const fn new() -> Self {
        // SAFETY: None is a valid initial state for UnsafeCell<Option<Waker>>.
        const EMPTY_WAKER: UnsafeCell<Option<Waker>> = UnsafeCell::new(None);
        Self {
            wg_handle: Cell::new(0),
            wakers: [EMPTY_WAKER; MAX_REACTOR_SLOTS],
            used: Cell::new(0),
        }
    }

    /// Set the WaitGroup handle. Must be called before starting the executor.
    pub fn init(&self, wg_handle: u32) {
        self.wg_handle.set(wg_handle);
    }

    /// Register a kernel object handle for async readiness notification.
    ///
    /// Adds the object to the WaitGroup and stores the waker. Returns the
    /// slot index (used to update or deregister later).
    ///
    /// # Errors
    ///
    /// Returns `Error::ResourceExhausted` if all slots are occupied, or
    /// propagates errors from `wait_group_add`.
    pub fn register(
        &self,
        handle: u32,
        signals: Signals,
        waker: &Waker,
    ) -> Result<usize> {
        let used = self.used.get();
        let slot = find_free_slot(used).ok_or(Error::ResourceExhausted)?;

        syscall::wait_group_add(self.wg_handle.get(), handle, signals, slot)?;

        // SAFETY: single-threaded, non-reentrant — no concurrent access.
        unsafe { *self.wakers[slot].get() = Some(waker.clone()) };
        self.used.set(used | (1 << slot));

        Ok(slot)
    }

    /// Update the waker for an existing registration.
    ///
    /// Must be called on each `poll()` because the waker may change between
    /// polls (e.g., if the task is moved to a different executor slot).
    pub fn update_waker(&self, slot: usize, waker: &Waker) {
        // SAFETY: single-threaded, non-reentrant.
        unsafe { *self.wakers[slot].get() = Some(waker.clone()) };
    }

    /// Deregister a kernel object handle and free the slot.
    pub fn deregister(&self, slot: usize, handle: u32) {
        let _ = syscall::wait_group_remove(self.wg_handle.get(), handle);
        // SAFETY: single-threaded, non-reentrant.
        unsafe { *self.wakers[slot].get() = None };
        self.used.set(self.used.get() & !(1 << slot));
    }

    /// Block until any registered object becomes ready, then wake the
    /// corresponding task.
    ///
    /// Call this from the executor's idle closure. If no objects are
    /// registered, falls back to a spin-loop hint.
    pub fn wait_for_events(&self) {
        if self.used.get() == 0 {
            core::hint::spin_loop();
            return;
        }

        match syscall::object_wait(self.wg_handle.get(), Signals::READABLE, Instant::MAX) {
            Ok(wait_return) => {
                let slot = wait_return.user_data;
                if slot < MAX_REACTOR_SLOTS {
                    // SAFETY: single-threaded, non-reentrant.
                    if let Some(waker) = unsafe { &*self.wakers[slot].get() } {
                        waker.wake_by_ref();
                    }
                }
            }
            Err(_) => {
                // Timeout or error — wake all registered tasks so they can
                // re-check their conditions.
                for i in 0..MAX_REACTOR_SLOTS {
                    if self.used.get() & (1 << i) != 0 {
                        // SAFETY: single-threaded, non-reentrant.
                        if let Some(waker) = unsafe { &*self.wakers[i].get() } {
                            waker.wake_by_ref();
                        }
                    }
                }
            }
        }
    }
}

/// Find the lowest free slot in the bitmask.
fn find_free_slot(used: u16) -> Option<usize> {
    if used == u16::MAX {
        return None;
    }
    // Find lowest clear bit.
    Some((!used).trailing_zeros() as usize)
}

// --- I/O Futures ------------------------------------------------------------

/// Future that waits for signals on a kernel object.
///
/// On first `Pending`, registers the object with the global [`REACTOR`].
/// The reactor's WaitGroup will wake this task when the object's signals
/// match. Automatically deregisters on completion or drop.
pub struct ObjectWaitFuture {
    handle: u32,
    signal_mask: Signals,
    slot: Option<usize>,
}

impl ObjectWaitFuture {
    fn deregister_if_needed(&mut self) {
        if let Some(slot) = self.slot.take() {
            REACTOR.deregister(slot, self.handle);
        }
    }
}

impl Future for ObjectWaitFuture {
    type Output = Result<WaitReturn>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match syscall::object_wait(self.handle, self.signal_mask, Instant::MIN) {
            Ok(wait_return) => {
                self.deregister_if_needed();
                Poll::Ready(Ok(wait_return))
            }
            Err(Error::DeadlineExceeded) => {
                // Not ready — register or update waker.
                match self.slot {
                    Some(slot) => REACTOR.update_waker(slot, cx.waker()),
                    None => match REACTOR.register(self.handle, self.signal_mask, cx.waker()) {
                        Ok(slot) => self.slot = Some(slot),
                        Err(e) => return Poll::Ready(Err(e)),
                    },
                }
                Poll::Pending
            }
            Err(e) => {
                self.deregister_if_needed();
                Poll::Ready(Err(e))
            }
        }
    }
}

impl Drop for ObjectWaitFuture {
    fn drop(&mut self) {
        self.deregister_if_needed();
    }
}

/// Wait for signals on a kernel object handle.
///
/// Returns when any signal in `signal_mask` becomes pending on the object.
/// Registers with the global [`REACTOR`] for efficient WaitGroup-based
/// blocking.
pub fn object_wait(handle: u32, signal_mask: Signals) -> ObjectWaitFuture {
    ObjectWaitFuture {
        handle,
        signal_mask,
        slot: None,
    }
}

/// Future that waits for an interrupt signal and acknowledges it.
///
/// On first `Pending`, registers with the global [`REACTOR`]. When the
/// interrupt fires, automatically calls `interrupt_ack` to clear and
/// re-enable it. Deregisters on completion or drop.
pub struct InterruptFuture {
    handle: u32,
    signal_mask: Signals,
    slot: Option<usize>,
}

impl InterruptFuture {
    fn deregister_if_needed(&mut self) {
        if let Some(slot) = self.slot.take() {
            REACTOR.deregister(slot, self.handle);
        }
    }
}

impl Future for InterruptFuture {
    type Output = Result<Signals>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match syscall::object_wait(self.handle, self.signal_mask, Instant::MIN) {
            Ok(wait_return) => {
                self.deregister_if_needed();
                let _ = syscall::interrupt_ack(self.handle, wait_return.pending_signals);
                Poll::Ready(Ok(wait_return.pending_signals))
            }
            Err(Error::DeadlineExceeded) => {
                match self.slot {
                    Some(slot) => REACTOR.update_waker(slot, cx.waker()),
                    None => match REACTOR.register(self.handle, self.signal_mask, cx.waker()) {
                        Ok(slot) => self.slot = Some(slot),
                        Err(e) => return Poll::Ready(Err(e)),
                    },
                }
                Poll::Pending
            }
            Err(e) => {
                self.deregister_if_needed();
                Poll::Ready(Err(e))
            }
        }
    }
}

impl Drop for InterruptFuture {
    fn drop(&mut self) {
        self.deregister_if_needed();
    }
}

/// Wait for an interrupt signal and auto-acknowledge it.
///
/// Returns the pending signals when the interrupt fires. The interrupt is
/// acknowledged automatically so it can fire again. Registers with the
/// global [`REACTOR`] for efficient WaitGroup-based blocking.
pub fn wait_interrupt(handle: u32, signal_mask: Signals) -> InterruptFuture {
    InterruptFuture {
        handle,
        signal_mask,
        slot: None,
    }
}
