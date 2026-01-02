# ARMv7-M Alignment & Codegen Safety Design

## 1. Goals

- Prevent ARMv7-M (Cortex-M3/M4) from taking HardFault/UsageFault/MemManage faults due to misaligned multi-word loads/stores (e.g. `LDMIA`, `STMIA`).
- Make any alignment assumptions **explicit and enforced by types/APIs**, not accidentally inferred by LLVM.
- Share as much of the policy as possible with ARMv8-M so that regressions are caught early.

Initial application scope (concrete targets):

- IPC userspace tests:
  - `pw_kernel/tests/ipc/user/initiator.rs` send/receive buffers.
  - Matching handler-side buffers in the IPC test.
- Any shared buffers that cross the kernel/userspace boundary in the IPC path (to be enumerated during implementation).

## 2. Bug Class / Threat Model

- ARMv7-M faults on certain unaligned accesses, especially multi-word load/store instructions.
- LLVM may legally emit word or `LDM/STM` sequences if it believes a pointer is 4-byte aligned, even if user code later computes misaligned derived pointers.
- Example pattern:
  - Base buffer is word-aligned.
  - Code takes `&buf[1]` or `base.add(1)` and then performs operations that LLVM optimizes into multi-word loads.
  - Hardware sees `LDM` from an address `% 4 != 0` and raises a fault.

We want to eliminate this pattern by construction in IPC/user code and any other hot paths on ARMv7-M.

## 3. Strategy Overview

1. **Align by construction:** Use `#[repr(align(4))]` (or stronger) for key buffer types so their *base* addresses are always 4-byte aligned.
2. **Constrain access patterns:** Design APIs so that callers operate on aligned units (u32 / 4-byte chunks) rather than arbitrary byte offsets when alignment is required.
3. **Mark truly unaligned accesses as such:** Where odd-byte operations are required, use operations that do not invite multi-word loads (byte loops, `read_unaligned`, etc.).
4. **Codegen sanity tests:** Add small, focused tests that build representative Rust snippets for ARMv7-M and ARMv8-M, disassemble, and assert on the generated instructions.

## 4. Alignment by Construction

### 4.1 Buffer Types

- Introduce newtypes or wrappers for buffers that participate in IPC and similar protocols, for example:
  - Message payload buffers.
  - Kernel<->user shared memory regions.
  - Per-thread/process stacks, if accessed via typed views.
- Example (sketch only):

```rust
#[repr(C, align(4))]
pub struct AlignedBuf<const N: usize> {
    buf: [u8; N],
}
```

- Invariants:
  - `AlignedBuf` instances are always 4-byte aligned in memory.
  - Public APIs must not allow callers to create misaligned `*const T` / `*mut T` from `buf` without going through explicitly unaligned-safe operations (e.g. `read_unaligned`).
  - Word-based views (e.g. `as_u32_slice`) only expose 4-byte aligned views.

### 4.2 Where to Apply

- **Userspace IPC initiator/handler tests:** Ensure send/receive buffers are aligned wrappers, not raw `[u8; N]` from arbitrary stack locations.
- **Syscall boundaries:** Any buffer pointers crossing the kernel/userspace boundary should either be:
  - Typed aligned buffers, or
  - Explicitly treated as unaligned and only accessed with safe patterns.

## 5. Safe Access Patterns

### 5.1 Aligned Operations

- Provide APIs that make the intended alignment obvious:
  - A canonical trait or helper (e.g. `trait AlignedWords`) providing:
    - `fn as_u32_words(&self) -> &[u32]` on aligned buffers.
    - `fn as_u32_words_mut(&mut self) -> &mut [u32]`.
- Code using these APIs must:
  - Avoid manual pointer arithmetic that breaks alignment (e.g., `&words[0] as *const u32 as *const u8` then adding 1).
  - Prefer iteration over `u32` elements rather than slicing arbitrary bytes.

### 5.2 Unaligned Operations

- When byte-level operations are required (e.g. protocol fields that are not 4-byte aligned):
  - Use standard library helpers that are explicitly unaligned-safe, such as `u32::from_le_bytes` on a `[u8; 4]`, **but** ensure that the underlying storage is really a `[u8; 4]` value, not an arbitrary sub-slice into a larger aligned buffer at an odd offset.
  - Where a pointer may be truly unaligned, use `core::ptr::read_unaligned` / `write_unaligned` or simple byte-copy loops.
- Design rule:
  - If a slice or pointer **might** be unaligned, we never call into an API that allows LLVM to assume alignment (e.g., treating it as `*const u32` without `read_unaligned`).

## 6. Codegen Sanity Tests

### 6.1 Test Scope

- Add a small test crate or module containing representative patterns from IPC and other critical paths, for example:
  - Message parsing that uses `u32::from_ne_bytes`.
  - Buffer copy/transform loops.
  - Any pattern previously observed to generate `LDM/STM` on ARMv7-M.

### 6.2 Build & Disassemble

- For each pattern, build for:
  - ARMv7-M (e.g. `armv7m_minimal` config).
  - ARMv8-M (e.g. `mps2_an505`).
- Use `arm-none-eabi-objdump` (or equivalent) to disassemble relevant functions.
- Script (Python or Rust) parses the disassembly and checks, for a small, named set of functions:
  - No `LDM*`/`STM*` sequences are emitted for code paths that are supposed to be alignment-agnostic.
  - Where multi-word instructions exist, they are only used on code paths with guaranteed 4-byte alignment (documented and enforced by types).
  - We do **not** attempt to ban `LDM/STM` globally; only to enforce policy on selected functions.

### 6.3 Integration

- Wrap the script as a Bazel test target under `pw_kernel` tooling.
- Place it under a dedicated path (e.g. `pw_kernel/tooling/alignment_sanity/`).
- Make it part of the kernel test suite so changes in Rust code or toolchain behavior that reintroduce risky patterns are caught early, without running on every trivial unit test.

## 7. Interaction with ARMv8-M

- ARMv8-M hardware is more tolerant of unaligned accesses, but we still want shared invariants:
  - The same aligned buffer types and APIs should be usable on both architectures.
  - Codegen tests can run on both, giving early warning when compiler behavior changes.
- We do **not** rely on enabling `UNALIGN_TRP` at runtime on ARMv8-M; instead we:
  - Treat ARMv7-M as the runtime truth source for alignment faults.
  - Use static/codegen checks to keep both architectures in a safe subset of behavior.

## 8. Open Questions / Follow-ups

- Exact list of buffer types to wrap in aligned newtypes (IPC only vs broader).
- Whether 4-byte alignment is sufficient everywhere, or if some paths benefit from 8-byte alignment.
- How aggressively to enforce the policy in generic library code vs only in kernel/userspace boundaries.
- Where to host the codegen test crate and scripts (likely under `pw_kernel/tooling`).

## 9. Acceptance Criteria

- IPC userspace tests on `armv7m_minimal` run without alignment-related HardFault/UsageFault/MemManage faults.
- The same IPC tests on `mps2_an505` continue to pass unchanged.
- Alignment/codegen sanity tests pass for both ARMv7-M and ARMv8-M toolchains/configs we support.

## 10. Verification Notes (Current Status)

- ARMv8-M baseline (mps2_an505, QEMU):
  - Built IPC image with:
    - `bazelisk build --config=k_qemu_mps2_an505 //pw_kernel/target/mps2_an505/ipc/user:ipc`
  - Ran under QEMU as documented in
    - [pw_kernel/target/mps2_an505/ipc/README.md](../pw_kernel/target/mps2_an505/ipc/README.md):
    - `qemu-system-arm -M mps2-an505 -nographic -semihosting -kernel bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf`
  - Detokenized log (using `python -m pw_tokenizer.detokenize base64 <elf> < <log>`):
    - Shows clean kernel/userspace bring-up.
    - Initiator/handler IPC traffic from 'a' through 'z'.
    - Final lines: `Ipc test complete`, `✅ PASSED`, `Shutting down with code 0`.
- ARMv7-M (armv7m_minimal):
  - TODO: repeat the same IPC scenario once alignment fixes and MPU/system-image work are in place, and record results here.
