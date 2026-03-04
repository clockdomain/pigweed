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

//! Async executor test for pw_kernel userspace.
//!
//! Demonstrates a minimal round-robin executor running in a pw_kernel
//! userspace thread. The executor polls pinned futures without allocation,
//! using a noop waker (all tasks are re-polled each iteration).
//!
//! Three async tasks are spawned:
//!   1. Immediate completion — verifies basic future mechanics.
//!   2. Multi-yield accumulator — yields N times, summing values.
//!   3. Syscall from async context — proves kernel syscalls work inside futures.

#![no_main]
#![no_std]

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use pw_status::{Error, Result, StatusCode};
use userspace::{entry, syscall};

// ---------------------------------------------------------------------------
// YieldOnce: a future that returns Pending exactly once, then Ready.
// ---------------------------------------------------------------------------

struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            Poll::Pending
        }
    }
}

fn yield_once() -> YieldOnce {
    YieldOnce { yielded: false }
}

// ---------------------------------------------------------------------------
// Round-robin poll loop executor with noop waker.
// ---------------------------------------------------------------------------

/// Run the executor test.  Spawns three async tasks, polls them round-robin,
/// and verifies the results.
fn test_executor() -> Result<()> {
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);

    // ---- Task 1: immediate completion ----
    let mut result1: u32 = 0;
    let mut task1_done = false;
    let mut task1 = core::pin::pin!(async { 42u32 });

    // ---- Task 2: yields 5 times, accumulating 1+2+3+4+5 = 15 ----
    let mut result2: u32 = 0;
    let mut task2_done = false;
    let mut task2 = core::pin::pin!(async {
        let mut sum = 0u32;
        for i in 1..=5u32 {
            sum += i;
            yield_once().await;
        }
        sum
    });

    // ---- Task 3: exercises syscalls from async context ----
    let mut result3: u32 = 0;
    let mut task3_done = false;
    let mut task3 = core::pin::pin!(async {
        // Prove that kernel syscalls work inside a future.
        if syscall::debug_nop().is_err() {
            return 0u32;
        }
        yield_once().await;
        if syscall::debug_nop().is_err() {
            return 0u32;
        }
        1u32
    });

    // ---- Executor loop: poll all tasks round-robin ----
    let mut iterations: u32 = 0;

    loop {
        iterations += 1;
        let mut any_pending = false;

        if !task1_done {
            match task1.as_mut().poll(&mut cx) {
                Poll::Ready(v) => {
                    result1 = v;
                    task1_done = true;
                }
                Poll::Pending => any_pending = true,
            }
        }

        if !task2_done {
            match task2.as_mut().poll(&mut cx) {
                Poll::Ready(v) => {
                    result2 = v;
                    task2_done = true;
                }
                Poll::Pending => any_pending = true,
            }
        }

        if !task3_done {
            match task3.as_mut().poll(&mut cx) {
                Poll::Ready(v) => {
                    result3 = v;
                    task3_done = true;
                }
                Poll::Pending => any_pending = true,
            }
        }

        if !any_pending {
            break;
        }
    }

    // ---- Verify results ----

    pw_log::info!("Task 1 result: {} (expected 42)", result1 as u32);
    if result1 != 42 {
        pw_log::error!("Task 1 returned wrong value");
        return Err(Error::Internal);
    }

    pw_log::info!("Task 2 result: {} (expected 15)", result2 as u32);
    if result2 != 15 {
        pw_log::error!("Task 2 returned wrong value");
        return Err(Error::Internal);
    }

    pw_log::info!("Task 3 result: {} (expected 1)", result3 as u32);
    if result3 != 1 {
        pw_log::error!("Task 3 returned wrong value (syscall failure?)");
        return Err(Error::Internal);
    }

    // Verify iteration count:
    //   Iteration 1: Task1→Ready, Task2→Pending(yield 1), Task3→Pending(yield 1)
    //   Iteration 2: Task2→Pending(yield 2), Task3→Ready
    //   Iteration 3: Task2→Pending(yield 3)
    //   Iteration 4: Task2→Pending(yield 4)
    //   Iteration 5: Task2→Pending(yield 5)
    //   Iteration 6: Task2→Ready
    pw_log::info!("Executor iterations: {} (expected 6)", iterations as u32);
    if iterations != 6 {
        pw_log::error!("Unexpected iteration count");
        return Err(Error::Internal);
    }

    Ok(())
}

#[entry]
fn entry() -> ! {
    pw_log::info!("🔄 RUNNING: async executor test");
    let ret = test_executor();

    if ret.is_err() {
        pw_log::error!("❌ FAILED: {}", ret.status_code() as u32);
    } else {
        pw_log::info!("✅ PASSED");
    }

    let _ = syscall::debug_shutdown(ret);
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
