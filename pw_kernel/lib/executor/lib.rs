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

//! Async executor for pw_kernel userspace.
//!
//! Wraps Embassy's [`raw::Executor`] to provide waker-based task scheduling
//! with dynamic spawning via [`Spawner`]. Tasks are only re-polled when their
//! waker is invoked, avoiding unnecessary work.
//!
//! The executor accepts a caller-provided `idle` closure that runs when no
//! tasks are ready. This allows plugging in the appropriate blocking strategy:
//!
//! ```rust,ignore
//! // Spin-wait (no kernel dependency):
//! executor.run(init, || core::hint::spin_loop());
//!
//! // Block via kernel object_wait:
//! executor.run(init, || {
//!     let _ = syscall::object_wait(wake_handle, Signals::USER, Instant::MAX);
//! });
//! ```

#![no_std]

use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll};

pub use embassy_executor::Spawner;
use embassy_executor::raw;

/// Global flag set by the pender when any task becomes ready.
///
/// Embassy calls [`__pender`] whenever a waker fires. The executor checks
/// this flag each iteration to decide whether to poll or spin.
static SIGNAL_WORK: AtomicBool = AtomicBool::new(false);

/// Pender callback invoked by Embassy when a task is woken.
///
/// Can be called from any context (interrupt, another thread, etc.).
/// Must NOT call `poll()` — just sets the flag for the main loop.
#[export_name = "__pender"]
fn __pender(_context: *mut ()) {
    SIGNAL_WORK.store(true, Ordering::Release);
}

/// Async executor backed by Embassy's raw executor.
///
/// Provides dynamic task spawning via [`Spawner`] and waker-based scheduling.
/// Create with [`Executor::new()`], then call [`Executor::run()`] with an
/// init closure that spawns your tasks.
pub struct Executor {
    inner: raw::Executor,
    not_send: PhantomData<*mut ()>,
}

impl Executor {
    /// Create a new executor.
    pub const fn new() -> Self {
        Self {
            inner: raw::Executor::new(core::ptr::null_mut()),
            not_send: PhantomData,
        }
    }

    /// Run the executor forever.
    ///
    /// The `init` closure receives a [`Spawner`] to spawn the initial tasks.
    /// After `init` returns, the executor polls tasks in a loop. Tasks can
    /// spawn additional tasks by holding a copy of the `Spawner`.
    ///
    /// The `idle` closure is called when no tasks are ready. Use it to block
    /// efficiently (e.g. via `object_wait`) or spin-wait.
    ///
    /// This function never returns.
    pub fn run(&'static self, init: impl FnOnce(Spawner), idle: impl Fn()) -> ! {
        init(self.inner.spawner());

        loop {
            // SAFETY: we are the only caller of poll() and we don't call it
            // reentrantly (the pender only sets a flag).
            unsafe { self.inner.poll() };

            if !SIGNAL_WORK.swap(false, Ordering::AcqRel) {
                idle();
            }
        }
    }
}

// --- Utility futures --------------------------------------------------------

/// A future that yields once (returning [`Poll::Pending`]), then completes.
///
/// Properly wakes itself before returning `Pending` so the executor knows
/// to re-poll on the next iteration.
pub struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Create a future that yields execution once, then completes.
pub fn yield_once() -> YieldOnce {
    YieldOnce { yielded: false }
}
