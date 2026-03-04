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
//! Demonstrates the Embassy-based executor running in a pw_kernel userspace
//! thread. Tasks are spawned dynamically via [`Spawner`] and scheduled using
//! waker-based readiness tracking.
//!
//! Three async tasks are spawned:
//!   1. Immediate completion — verifies basic future mechanics.
//!   2. Multi-yield accumulator — yields N times, summing values.
//!   3. Syscall from async context — proves kernel syscalls work inside futures.

#![no_main]
#![no_std]

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_executor::Spawner;
use executor::yield_once;
use pw_status::{Error, Result, StatusCode};
use userspace::{entry, syscall};

static EXECUTOR: executor::Executor = executor::Executor::new();

// Task results stored in atomics so spawned tasks can communicate results
// back to the main verification task.
static RESULT1: AtomicU32 = AtomicU32::new(0);
static RESULT2: AtomicU32 = AtomicU32::new(0);
static RESULT3: AtomicU32 = AtomicU32::new(0);

// Counters track task completion. The main task waits for all three.
static DONE_COUNT: AtomicU32 = AtomicU32::new(0);

#[embassy_executor::task]
async fn task_immediate() {
    RESULT1.store(42, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

#[embassy_executor::task]
async fn task_accumulator() {
    let mut sum = 0u32;
    for i in 1..=5u32 {
        sum += i;
        yield_once().await;
    }
    RESULT2.store(sum, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

#[embassy_executor::task]
async fn task_syscall() {
    let result = if syscall::debug_nop().is_err() {
        0u32
    } else {
        yield_once().await;
        if syscall::debug_nop().is_err() { 0u32 } else { 1u32 }
    };
    RESULT3.store(result, Ordering::Release);
    DONE_COUNT.fetch_add(1, Ordering::Release);
}

#[embassy_executor::task]
async fn task_main(spawner: Spawner) {
    // Spawn the three test tasks.
    spawner.spawn(task_immediate()).unwrap();
    spawner.spawn(task_accumulator()).unwrap();
    spawner.spawn(task_syscall()).unwrap();

    // Wait for all three tasks to complete.
    while DONE_COUNT.load(Ordering::Acquire) < 3 {
        yield_once().await;
    }

    // Verify results.
    let ret = verify_results();

    if ret.is_err() {
        pw_log::error!("❌ FAILED: {}", ret.status_code() as u32);
    } else {
        pw_log::info!("✅ PASSED");
    }

    let _ = syscall::debug_shutdown(ret);
}

fn verify_results() -> Result<()> {
    let r1 = RESULT1.load(Ordering::Acquire);
    pw_log::info!("Task 1 result: {} (expected 42)", r1);
    if r1 != 42 {
        pw_log::error!("Task 1 returned wrong value");
        return Err(Error::Internal);
    }

    let r2 = RESULT2.load(Ordering::Acquire);
    pw_log::info!("Task 2 result: {} (expected 15)", r2);
    if r2 != 15 {
        pw_log::error!("Task 2 returned wrong value");
        return Err(Error::Internal);
    }

    let r3 = RESULT3.load(Ordering::Acquire);
    pw_log::info!("Task 3 result: {} (expected 1)", r3);
    if r3 != 1 {
        pw_log::error!("Task 3 returned wrong value (syscall failure?)");
        return Err(Error::Internal);
    }

    Ok(())
}

#[entry]
fn entry() -> ! {
    pw_log::info!("🔄 RUNNING: async executor test");
    EXECUTOR.run(
        |spawner| { spawner.spawn(task_main(spawner)).unwrap(); },
        || core::hint::spin_loop(),
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
