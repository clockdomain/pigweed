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

use kernel_config::{ExceptionMode, KernelConfig, RiscVKernelConfigInterface};
use log_if::debug_if;
#[cfg(feature = "exceptions_reload_pmp")]
use memory_config::MemoryConfig as _;
use pw_log::info;
#[cfg(not(feature = "user_space"))]
pub(crate) use riscv_macro::kernel_only_exception as exception;
#[cfg(feature = "user_space")]
pub(crate) use riscv_macro::user_space_exception as exception;

#[cfg(feature = "exceptions_reload_pmp")]
use crate::MemoryConfig;
#[cfg(not(feature = "veer_pic"))]
use crate::plic as pic;
use crate::regs::{
    Cause, Exception, Interrupt, MCause, MCauseVal, MStatus, MStatusVal, MtVal, MtVec, MtVecMode,
    PrivilegeLevel,
};
use crate::timer;
#[cfg(feature = "veer_pic")]
use crate::veer_pic as pic;

const LOG_EXCEPTIONS: bool = false;

/// Set for the duration of dispatching a hardware interrupt (not a
/// syscall/exception) through `interrupt_handler`, below.
///
/// Read by `Arch::context_switch` (`threads.rs`) to decide whether a wake
/// triggered from inside this interrupt must defer its context switch
/// rather than perform it immediately -- see
/// `threads::complete_deferred_context_switch` for why.
static IN_HW_INTERRUPT: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

pub(crate) fn in_hw_interrupt() -> bool {
    IN_HW_INTERRUPT.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn early_init() {
    // Explicitly set up MTVEC to point to the kernel's handler to ensure
    // that it is set to the correct mode.
    let (base, mode) = match KernelConfig::get_exception_mode() {
        ExceptionMode::Direct => (_start_trap as *const () as usize, MtVecMode::Direct),
        ExceptionMode::Vectored(vec_table) => (vec_table, MtVecMode::Vectored),
    };
    MtVec::write(MtVec::read().with_base(base).with_mode(mode));
}

/// Type of all exception handlers.
#[allow(dead_code)]
pub type ExceptionHandler =
    unsafe extern "C" fn(mcause: MCauseVal, mepc: usize, frame: &mut TrapFrame);

#[inline(never)]
fn dump_exception_frame(frame: &TrapFrame) {
    info!(
        "Exception frame {:#08x}:",
        core::ptr::addr_of!(frame) as usize
    );

    info!(
        "ra  {:#010x} t0 {:#010x} t1  {:#010x} t2  {:#010x}",
        frame.ra as usize, frame.t0 as usize, frame.t1 as usize, frame.t2 as usize
    );
    info!(
        "t3  {:#010x} t4 {:#010x} t5  {:#010x} t6  {:#010x}",
        frame.t3 as usize, frame.t4 as usize, frame.t5 as usize, frame.t6 as usize
    );
    info!(
        "a0  {:#010x} a1 {:#010x} a2  {:#010x} a3  {:#010x}",
        frame.a0 as usize, frame.a1 as usize, frame.a2 as usize, frame.a3 as usize
    );
    info!(
        "a4  {:#010x} a5 {:#010x} a6  {:#010x} a7  {:#010x}",
        frame.a4 as usize, frame.a5 as usize, frame.a6 as usize, frame.a7 as usize
    );
    info!(
        "tp  {:#010x} gp {:#010x} sp  {:#010x}",
        frame.tp as usize, frame.gp as usize, frame.sp as usize
    );
    info!("mstatus {:#010x}", frame.status as usize);
    info!("mcause {:#010x}", MCause::read().0 as usize);
    info!("mtval {:#010x}", MtVal::read().0 as usize);
    info!("epc {:#010x}", frame.epc as usize);
}

fn is_exception_from_kernel(frame: &TrapFrame) -> bool {
    MStatusVal(frame.status).mpp() != PrivilegeLevel::User
}

fn exception_handler(exception: Exception, mepc: usize, frame: &mut TrapFrame) {
    // For now, always dump the exception we've received and halt.
    debug_if!(
        LOG_EXCEPTIONS,
        "Exception: exception_number={:#010x}, mepc={:#010x}, mstatus={:#010x}",
        exception as usize,
        mepc as usize,
        MStatus::read().0 as usize,
    );

    match exception {
        Exception::EnvironmentCallFromUMode | Exception::EnvironmentCallFromMMode => {
            #[cfg(feature = "user_space")]
            crate::syscall::handle_ecall(frame);
            #[cfg(not(feature = "user_space"))]
            {
                dump_exception_frame(frame);
                let guard = kernel::interrupt_controller::InterruptGuard::new(
                    super::Arch,
                    !is_exception_from_kernel(frame),
                );
                let _ = kernel::scheduler::handle_terminal_exception(super::Arch, guard);
            }
        }
        Exception::Breakpoint => {
            // Breakpoint is currently use to handle kernel panics.  Don't call
            // back into `handle_terminal_exception` and instead terminate the
            // kernel.
            dump_exception_frame(frame);
            kernel::shutdown(1);
        }
        _ => {
            dump_exception_frame(frame);
            let guard = kernel::interrupt_controller::InterruptGuard::new(
                super::Arch,
                !is_exception_from_kernel(frame),
            );
            let _ = kernel::scheduler::handle_terminal_exception(super::Arch, guard);
        }
    };
}

// Default interrupt handler
unsafe fn interrupt_handler(interrupt: Interrupt, mepc: usize, frame: &TrapFrame) {
    debug_if!(
        LOG_EXCEPTIONS,
        "Interrupt: interrupt_number={:#010x}, mepc={:#010x}, frame={:#010x}, mstatus={:#010x}",
        interrupt as usize,
        mepc as usize,
        core::ptr::addr_of!(frame) as usize,
        frame.status as usize,
    );

    let from_userspace = !is_exception_from_kernel(frame);

    match interrupt {
        Interrupt::MachineTimer => {
            timer::mtimer_tick(from_userspace);
        }
        Interrupt::MachineExternal => {
            pic::interrupt(from_userspace);
        }
        _ => {
            pw_assert::panic!(
                "Unhandled interrupt: interrupt_number={:#010x}",
                interrupt as usize
            );
        }
    }
}

#[exception(exception = "_start_trap")]
#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler(mcause: MCauseVal, mepc: usize, frame: &mut TrapFrame) {
    #[cfg(feature = "exceptions_reload_pmp")]
    unsafe {
        // Before we do anything, configure ePMP for kernel access.
        MemoryConfig::KERNEL_THREAD_MEMORY_CONFIG.write();
    }

    match mcause.cause() {
        Cause::Interrupt(interrupt) => {
            IN_HW_INTERRUPT.store(true, core::sync::atomic::Ordering::Relaxed);
            unsafe { interrupt_handler(interrupt, mepc, frame) };
            IN_HW_INTERRUPT.store(false, core::sync::atomic::Ordering::Relaxed);
        }
        Cause::Exception(exception) => exception_handler(exception, mepc, frame),
    }

    // Complete any context switch that `Arch::context_switch` deferred while
    // the hardware interrupt above was being dispatched. Must run after
    // `IN_HW_INTERRUPT` is cleared: by this point every RAII guard taken
    // while handling the interrupt (object lock, wait-queue lock, scheduler
    // lock) has unwound normally, so it's safe to perform the switch here.
    // No-op if no switch was deferred.
    crate::threads::complete_deferred_context_switch();

    #[cfg(feature = "exceptions_reload_pmp")]
    {
        use kernel::Kernel;
        // When we exit the trap, get the current thread and load its memory
        // config into ePMP.
        let mut scheduler = crate::Arch.get_scheduler().lock(crate::Arch);
        unsafe {
            let tstate = scheduler.get_current_arch_thread_state();
            if tstate != core::ptr::null_mut() {
                (*(*tstate).memory_config).write();
            }
        }
    }
}

#[repr(C)]
pub struct TrapFrame {
    // Note: offsets are for 32bit sized usize
    pub epc: usize,    // 0x00
    pub status: usize, // 0x04
    pub ra: usize,     // 0x08

    // SAFETY: the `a()` accessor requires these to be in order.
    pub a0: usize, // 0x0c
    pub a1: usize, // 0x10
    pub a2: usize, // 0x14
    pub a3: usize, // 0x18
    pub a4: usize, // 0x1c
    pub a5: usize, // 0x20
    pub a6: usize, // 0x24
    pub a7: usize, // 0x28

    pub t0: usize, // 0x2c
    pub t1: usize, // 0x30
    pub t2: usize, // 0x34
    pub t3: usize, // 0x38
    pub t4: usize, // 0x3c
    pub t5: usize, // 0x40
    pub t6: usize, // 0x44

    // These are only set when handling a trap from user space.
    pub tp: usize, // 0x48
    pub gp: usize, // 0x4c
    pub sp: usize, // 0x50

    // Per RISC-V calling convention, stacks must always be 16 byte aligned.
    pub pad: [usize; 3], // 0x54-0x5f
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 0x60);
