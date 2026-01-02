**Work order:** new_feature – ARMv7-M Userspace & MPU Support

- [x] **Setup:** ARMv7-M docs directory and design scaffold created.
- [ ] **Planning:** Refine design and expand task list.
- [ ] **Critic:** Deep review of design and risks.
- [ ] **Implementation:** Execute incremental bring-up and fixes.
- [ ] **Verification:** Confirm behavior across targets (QEMU + AST1030).
- [ ] **Work order review (Optional):** Reflect on process and templates.
- [ ] **Integration & Cleanup:** Final docs, CLs, and cleanup completed.

---

### Detailed Tasks (Initial Draft)

Planning phase should refine and extend this list.

1. **Baseline & Branching**
   - [ ] Branch from current upstream `main` for a fresh ARMv7-M effort.
   - [ ] Document branch name and purpose in this file.

2. **Minimal ARMv7-M Kernel Bring-up (QEMU lm3s6965evb)**
   - [ ] Ensure a minimal kernel-only target builds from `main`.
   - [ ] Boot under QEMU and reach a known-good "threads" test.
   - [ ] Capture logs and link them from the design doc.

3. **System Image & Relocation for ARMv7-M**
   - [ ] Specify required system_assembler behavior for ARMv7-M in the design.
   - [ ] Implement/port relocation fixes on top of main.
   - [ ] Add invariants/checks so bad layouts fail at build time.

4. **MPU Configuration & Safe Update Protocol**
   - [ ] Define the MPU region layout for armv7m_minimal.
   - [ ] Implement safe PMSAv7 MPU updates on context switch.
   - [ ] Add focused tests/logging for MPU configuration.

5. **Userspace Entry, ABI, and Alignment**
   - [ ] Re-establish userspace entry glue (assembly + Rust `main_*`) on ARMv7-M.
   - [ ] Document and enforce stack/exception frame invariants.
   - [ ] Address unaligned access/codegen constraints for ARMv7-M.

6. **IPC Initiator/Handler Tests**
   - [x] Establish ARMv8-M baseline on mps2_an505 (QEMU); IPC test passes end-to-end.
   - [ ] Process rule: for every IPC change, first rerun the ARMv8-M mps2_an505 IPC test and confirm it still passes before running the ARMv7-M armv7m_minimal IPC test.
   - [ ] Bring up IPC tests on armv7m_minimal under QEMU.
   - [ ] Ensure armv7m_minimal logs match ARMv8-M baseline ("Sent X, received (Y, Z)", "PASSED").
   - [ ] Add any ARMv7-M specific diagnostics needed for fault triage.
    - **Commands (current workspace):**
       - Build ARMv7-M minimal IPC image:
          - `bazelisk build --platforms=//pw_kernel/target/armv7m_minimal:armv7m_minimal //pw_kernel/target/armv7m_minimal/ipc/user:ipc`

   **6.1 Initiator & Handler Alignment & Codegen Fix Plan**

    **Current status (2026-01-02)**
    - Initiator-side fault has been mapped and refactored; initiator tests now use `AlignedBuf` and logging shims.
   - Kernel `SyscallBuffer::copy_into` uses `ipc_copy_bytes` on ARMv7-M to avoid unsafe memcpy codegen; disassembly confirms a byte-wise `ldrb/strb` loop with no `ldm/stm`.
   - Handler-side memcpy fault in `main_handler_1` has been refactored; static inspection of `main_initiator_0`, `main_handler_1`, and the `compiler_builtins::memcpy` implementations confirms that `memcpy` is no longer used on the IPC send/receive path.
    - Debug/codegen configuration, probe functions, and codegen sanity tests are planned but not yet implemented.

   This plan is intended to be systematic rather than reactive:
   first fix the debug/codegen configuration, then centralize
   all IPC-related copies through a small set of hardened helpers,
   and finally lock behavior in with codegen sanity tests.
   For Rust in particular, prefer explicit aligned/byte-wise copy
   helpers over relying on C toolchain flags such as `-mstrict-align`.

      1. **Get a debuggable, representative ARMv7-M IPC build**
          - [ ] Build the ARMv7-M IPC test target with debug info and predictable codegen:
             - `bazel build -c dbg --config=armv7m_safe --platforms=//pw_kernel/target/armv7m_minimal:armv7m_minimal //pw_kernel/target/armv7m_minimal/ipc/user:ipc_test`.
          - [ ] Ensure the build disables LTO and aggressive inlining for the IPC tests and helpers (via `armv7m_safe` or equivalent), so DWARF is preserved and small probe functions stay intact.
          - [ ] Locate the produced ELF:
             - `find bazel-bin -maxdepth 8 -type f \( -name 'ipc_test' -o -name 'ipc.elf' \) | sort`.
             - Use `file` and `arm-none-eabi-nm -n` to confirm it contains `main_initiator_0` and `main_handler_1` for armv7m_minimal.

      2. **Map fault PCs and LRs to Rust source**
          - [ ] Run addr2line on the debuggable ELF for both initiator and handler faults:
             - `arm-none-eabi-addr2line -f -C -e <elf> 0x00020154 0x000202e9 0x0003012e 0x000302c3`.
          - [ ] Record function names and `file:line` pairs (expected in pw_kernel/tests/ipc/user/initiator.rs, pw_kernel/tests/ipc/user/handler.rs, or shared helpers).
          - [ ] If addr2line still reports `??:0`, inspect ELF sections (`arm-none-eabi-objdump -h <elf> | grep -i debug`) and adjust Bazel/Rust flags or per-crate settings so ipc_test carries DWARF for the IPC paths, then repeat.

      3. **Identify the exact Rust constructs behind risky memcpy/codegen**
          - [ ] Open the mapped source locations and look for:
             - Slice/array copies (e.g. `copy_from_slice`) or struct moves that could lower to a bulk memcpy.
             - Helper calls on aligned buffers that may invoke compiler_builtins::memcpy/memmove.
          - [ ] Cross-check with disassembly around the known offsets, e.g. for initiator:
             - `arm-none-eabi-objdump -d <elf> | sed -n '20120,20180p'`.
             - Match the call at `0x000200f2` and the `ldmia.w r8!, {…}` at `0x00020154` to the Rust operation.
          - [ ] Do the same for handler around `0x0003012e` to understand which response/formatting copy the unrolled `ldmia.w` / `stmia.w` loop corresponds to.

      4. **Centralize IPC-related copies behind hardened helpers**
          - [ ] Treat memcpy-style bulk copies as an implementation detail of a few well-audited helpers rather than ad-hoc `copy_from_slice` calls.
          - [ ] Existing initiator-side and kernel helpers (already implemented):
             - `AlignedBuf` in pw_kernel/tests/ipc/user/initiator.rs:
                - `#[repr(C, align(4))] struct AlignedBuf<const N: usize> { buf: [u8; N] }`.
                - Methods: `as_bytes(&self) -> &[u8]`, `as_bytes_mut(&mut self) -> &mut [u8]`.
                - Used for both send and receive IPC buffers so user slices passed into `channel_transact` are 4-byte aligned on ARMv7-M.
             - Logging shims in pw_kernel/tests/ipc/user/initiator.rs:
                - On `target_arch = "arm"`, `test_log_info!` and `test_log_error!` are no-ops to avoid complex formatting codegen in the initiator.
                - On other targets, they delegate to `pw_log::info!` / `pw_log::error!`.
             - Kernel-side IPC copy helper in pw_kernel/kernel/object/buffer.rs:
                - `SyscallBuffer::copy_into` now calls `ipc_copy_bytes` instead of `NonNull::copy_to`.
                - On `target_arch = "arm"`, `ipc_copy_bytes` performs a byte-wise loop (emits `LDRB/STRB`) to avoid compiler_builtins::memcpy and its multi-word `ldmia/stmia` sequences on possibly unaligned syscall buffers.
                - On other targets, it falls back to the optimized pointer copy primitive.
          - [ ] Extend the same pattern to the handler side:
             - Introduce a handler-side helper (or reuse a shared one) for response-buffer copies, mirroring `ipc_copy_bytes` semantics:
                - On `target_arch = "arm"`, implement `handler_ipc_copy_bytes(dst: *mut u8, src: *const u8, len: usize)` as a byte-wise loop using `core::ptr::read`/`write`.
                - On other targets, delegate to `core::ptr::copy_nonoverlapping` or slice-based `copy_from_slice`.
             - Refactor pw_kernel/tests/ipc/user/handler.rs and any related formatting/response paths to use `AlignedBuf`-backed slices and the handler copy helper instead of ad-hoc memcpy-style constructs.
          - [ ] For aligned regions where performance matters, consider an explicit `u32`-based copy path (loads/stores on `*const u32` / `*mut u32` plus trailing-byte handling) guarded by assertions on alignment, so codegen is controlled and auditable.

      5. **Verify and iterate on ARMv7-M runtime behavior**
          - [ ] Rebuild for ARMv7-M with the hardened helpers in place:
             - `bazelisk build --config=armv7m_safe --platforms=//pw_kernel/target/armv7m_minimal:armv7m_minimal //pw_kernel/target/armv7m_minimal/ipc/user:ipc`.
          - [ ] Run under QEMU:
             - `timeout 30 qemu-system-arm -M lm3s6965evb -nographic -semihosting -kernel bazel-bin/pw_kernel/target/armv7m_minimal/ipc/user/ipc.elf | tee /tmp/armv7m_ipc_fixed.log`.
          - [ ] Detokenize logs:
             - `python -m pw_tokenizer.detokenize base64 bazel-bin/pw_kernel/target/armv7m_minimal/ipc/user/ipc.elf < /tmp/armv7m_ipc_fixed.log`.
          - [ ] Confirm there are no HardFault/UsageFault/MemManage logs and that the IPC test prints the full "Sent X, received (Y, Z)" sequence and a clear PASS indication, matching the ARMv8-M baseline.

      6. **Add small probe functions and codegen sanity tests**
          - [ ] Create a small tooling crate/module (e.g. pw_kernel/tooling/alignment_sanity) with minimal Rust functions mirroring initiator/handler IPC buffer access patterns and the hardened helpers.
          - [ ] Add Bazel targets that build these functions for `armv7m_minimal` (and `mps2_an505` as a control) and emit disassembly via `arm-none-eabi-objdump`.
          - [ ] Write a checker (e.g. Python script) that scans the disassembly for risky `ldm`/`stm` usage in the probe functions and fails if unaligned multi-word loads/stores reappear.
          - [ ] Wrap the checker as a `bazel test` target and document the command both here and in the alignment/codegen design doc, so regressions show up as test failures rather than HardFaults under QEMU.

    **6.2 Checkpoint: Debug & ARMv7-M Expert Recommendations (2026-01-02)**

    At this checkpoint, we engaged two “virtual experts” – a generic
    debugging specialist and an ARMv7-M/Cortex-M3 specialist – to review
    the current IPC bring-up state and propose next steps. Their advice
    is captured here as concrete, ordered debugging work items.

         1. **Prove whether user entries run at all (QEMU + GDB)**
             - Attach `arm-none-eabi-gdb` to the ARMv7-M IPC ELF under
                QEMU (`-S -gdb`).
             - Set breakpoints on the Rust user-thread entry functions in
                [pw_kernel/tests/ipc/user/initiator.rs](pw_kernel/tests/ipc/user/initiator.rs)
                and [pw_kernel/tests/ipc/user/handler.rs](pw_kernel/tests/ipc/user/handler.rs)
                (use `nm` / `info functions` / `addr2line` to locate them).
             - Run from reset and observe whether these breakpoints fire.
             - If they never fire, focus on ARMv7-M context-switch and
                return-to-user-mode glue (EXC_RETURN, CONTROL.SPSEL, etc.).

         2. **Check for silent faults and where the core “parks”**
             - In the same GDB session, set breakpoints on the lm3s6965evb
                fault handlers (HardFault, MemManage, UsageFault) and
                optionally any central kernel fault handler.
             - Run until the apparent stall or break manually near the
                timeout, then:
                - If you’re in a fault handler, use `bt` and
                   `arm-none-eabi-addr2line` on the stacked PC to map back to
                   source and decide which IPC/user/kernel path is still
                   faulting.
                - If you’re in idle/WFI or a kernel wait function
                   (e.g. `object_wait`), treat this as a potential IPC
                   deadlock / missing wakeup issue.

         3. **Static ISA scan for remaining risky accesses in IPC paths**
             - Use `arm-none-eabi-nm` on the ARMv7-M IPC ELF to locate
                symbols for `ipc_copy_bytes`, `SyscallBuffer::copy_into`,
                and the initiator/handler IPC helpers.
             - Run `arm-none-eabi-objdump -d -C` on these ranges and scan
                for `LDM`, `STM`, `LDRD`, `STRD`, or suspicious word/halfword
                accesses that could touch unaligned pointers.
             - Also inspect any linked `memcpy`/`memmove` implementations
                still reachable from IPC paths.
             - If multi-word or doubleword sequences remain on potentially
                unaligned addresses, further constrain the Rust/C code
                (pure `u8` loops, stronger alignment types) or provide a
                v7-M-specific safe copy helper and route all IPC copies
                through it.

         4. **Instrument ultra-cheap “heartbeats” in user code**
             - Add minimal ARMv7-M-only markers (simple `pw_log::info!` or
                even a global RAM flag) at the very start of initiator and
                handler `entry`, and just before the first IPC syscall.
             - Re-run ARMv7-M and detokenize logs:
                - If no heartbeat appears, user threads never actually run;
                   return to step 1 and focus on user entry glue.
                - If entry heartbeats appear but not the “before IPC” ones,
                   the stall is between entry and the first syscall.
                - If both appear, the stall is in or after the first IPC
                   operation; proceed to the IPC tracing step.

         5. **Trace the first IPC transaction end-to-end**
             - In GDB on ARMv7-M, set breakpoints on
                `userspace::syscall::channel_transact`, `object_wait`,
                `channel_read`, and `channel_respond` (use `nm` to find the
                exact symbol names).
             - Run from reset and watch:
                - Does the initiator hit `channel_transact`? Does it return?
                   With what `StatusCode`?
                - Does the handler ever hit `object_wait` / `channel_read` /
                   `channel_respond`?
             - Use this to classify the failure as “initiator never starts
                IPC”, “handler never wakes”, or “response never reaches
                initiator”, and target fixes accordingly.

         6. **Validate MPU layout for user stacks and IPC buffers**
             - Dump ARMv7-M MPU regions (RBAR/RASR) and SCB fault registers
                after boot but before the IPC test, then correlate with
                runtime addresses of user stacks and IPC buffers
                (via `nm` / `objdump`).
             - Ensure these lie entirely within RW user regions and do not
                straddle disabled subregions or boundaries.
             - If mismatches are found, fix the armv7m_minimal MPU
                configuration so user IPC memory is cleanly covered.

         7. **Check for “stall without fault” due to WFI / missing interrupts**
             - When the stall is reproduced under QEMU, attach GDB and
                inspect whether PC is in a WFI or idle loop and whether the
                relevant NVIC/SysTick/PendSV configuration for lm3s6965evb
                matches the working ARMv8-M target.
             - If the CPU is stuck in WFI with no pending interrupts,
                investigate and fix the interrupt configuration needed for
                IPC/scheduling on ARMv7-M.

         8. **(Optional) Reduce to a minimal IPC ping test**
             - In a small experiment, reduce the userspace IPC test to a
                single fixed send/echo round-trip plus `debug_shutdown` on
                both sides.
             - If this passes on ARMv7-M, the remaining bug lies in the
                more complex “char loop + dual-char response” behavior.
             - If it still hangs, the issue is in the basic IPC wiring for
                armv7m_minimal and should be debugged with the above
                instrumentation in place.

7. **AST1030 Target Integration**
   - [ ] Adapt design for non-XIP execution on AST1030.
   - [ ] Validate MPU layout and system image behavior on hardware.
   - [ ] Capture and link test logs.

8. **Finalization**
   - [ ] Update [docs/armv7m/index.md](index.md) with final document links/status.
   - [ ] Summarize outcomes and remaining TODOs in the design doc.
