HOWTO: Run IPC Test on ARM MPS2-AN505

## PURPOSE

This guide explains how to build and run the Inter-Process Communication (IPC)
test for the ARM MPS2-AN505 target using QEMU emulation. The IPC test 
demonstrates communication between two userspace applications running under the
Pigweed kernel on ARMv8-M architecture with PMSAv8 MPU.

## WHAT IS THE IPC TEST?

The IPC test consists of two userspace applications:

1. **Initiator Application** (`initiator`)
   - Sends lowercase letters 'a' through 'z' to the handler
   - Uses synchronous IPC (channel_transact) to send and receive
   - Validates that responses contain uppercase versions

2. **Handler Application** (`handler`)
   - Listens for incoming IPC messages
   - Converts received lowercase characters to uppercase
   - Responds back with both uppercase and original character

Test Flow:
----------
  Initiator                    Handler
     |                             |
     |--- send 'a' --------------->|
     |                      Convert to 'A'
     |<-- respond 'A' + 'a' -------|
     |                             |
     |--- send 'b' --------------->|
     |                      Convert to 'B'
     |<-- respond 'B' + 'b' -------|
     |                             |
    ... (continues for 'c' through 'z')

Success Criteria:
-----------------
  ✓ All 26 letters successfully sent and received
  ✓ Each uppercase conversion validated
  ✓ IPC channels work correctly between processes
  ✓ Test outputs "Ipc test complete"

## TARGET PLATFORM

Platform: ARM MPS2-AN505
CPU: ARM Cortex-M33 (ARMv8-M Mainline)
MPU: PMSAv8 (ARMv8-M Protected Memory System Architecture)
Memory: 4MB PSRAM, 16MB SSRAM1, 16MB SSRAM2, 16MB SSRAM3
Emulator: QEMU mps2-an505 machine

Key Features:
- ARMv8-M architecture with TrustZone support
- PMSAv8 MPU with 16 regions
- Cortex-M33 with FPU and DSP extensions
- Memory Protection Unit with granular access control

## SYSTEM CONFIGURATION

The IPC test uses the following memory layout on MPS2-AN505:

  Memory Region                  | Start Addr  | Size
  -------------------------------|-------------|--------
  Vector Table                   | 0x00000000  | 1KB
  Kernel Code (Flash/ROM)        | 0x00000400  | 256KB
  Kernel Data RAM                | 0x20000000  | 384KB
  Initiator App Code             | (kernel+)   | 128KB
  Initiator App RAM              | (kernel+)   | 16KB
  Handler App Code               | (kernel+)   | 128KB
  Handler App RAM                | (kernel+)   | 16KB

Process Configuration:
----------------------
  - Initiator Process:
    * 1 thread with 2KB stack
    * 1 channel_initiator object ("IPC")
    * Connected to handler's "IPC" object

  - Handler Process:
    * 1 thread with 2KB stack
    * 1 channel_handler object ("IPC")
    * Waits for incoming messages

## PREREQUISITES

1. Pigweed Environment
   ---------------------
   Ensure your Pigweed environment is bootstrapped:
   
   cd <pigweed-workspace>
   source ./bootstrap.sh

2. QEMU (Automatic Download)
   --------------------------
   QEMU is automatically downloaded by Bazel as a prebuilt binary from the
   Fuchsia CIPD repository. No manual installation required!
   
   The download happens automatically when you first run a test that needs QEMU.
   Pigweed uses QEMU 8.2+ which includes MPS2-AN505 support.

3. Bazel/Bazelisk
   ---------------
   Verify Bazel is available:
   
   bazelisk --version

## BUILDING THE IPC TEST

Step 1: Build the IPC Test Binary
----------------------------------

From the Pigweed workspace root:

  cd <pigweed-workspace>
  
  bazelisk build \
    --config=k_qemu_mps2_an505 \
    //pw_kernel/target/mps2_an505/ipc/user:ipc

IMPORTANT: You must use --config=k_qemu_mps2_an505 (not --platforms)
The config flag sets up the correct platform and configuration.

This command:
  • Configures for ARM Cortex-M33 (ARMv8-M)
  • Builds the kernel with PMSAv8 MPU support
  • Compiles both initiator and handler apps
  • Links everything into a single ELF image
  • Outputs: bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf

Build Time: ~30-90 seconds (depending on system)

Expected Output:
  INFO: Build completed successfully, 104 total actions

Step 2: Verify the Binary
--------------------------

Check that the ELF file was created:

  ls -lh bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf
  
  Expected: ~500KB - 1MB ELF file

Optional: Inspect the binary:

  file bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf
  
  Expected output:
  ELF 32-bit LSB executable, ARM, EABI5 version 1 (SYSV), statically linked

## RUNNING THE IPC TEST IN QEMU

Command to Run:
---------------

  qemu-system-arm \
    -M mps2-an505 \
    -nographic \
    -semihosting \
    -kernel bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf

Command Breakdown:
------------------
  -M mps2-an505       Use ARM MPS2-AN505 development board machine
  -nographic          Run without graphical window (console only)
  -semihosting        Enable ARM semihosting for console output
  -kernel <file>      Load ELF kernel image into emulated memory

Running the Test:
-----------------

  cd <pigweed-workspace>
  
  qemu-system-arm -M mps2-an505 -nographic -semihosting \
    -kernel bazel-bin/pw_kernel/target/mps2_an505/ipc/user/ipc.elf

Expected Output:
----------------

  [INFO ] <pw_kernel::kernel::kern_task> Kernel starting
  [INFO ] <pw_kernel::subsys::console::semihosting> Console initialized (semihosting)
  [INFO ] <pw_kernel::kernel::kern_task> Starting scheduler
  [INFO ] <app_handler> IPC service starting
  [INFO ] <app_initiator> Ipc test starting
  [INFO ] <app_initiator> Ipc test complete
  [INFO ] <pw_kernel::kernel::kern_task> All processes complete

NOTE: The output will be tokenized (base64-encoded strings). This is normal.
To see readable output, use the Pigweed detokenizer tool.

Test Success Indicators:
  ✓ "Ipc test starting" appears (in tokenized form)
  ✓ Series of character exchanges (repeated patterns)
  ✓ "Ipc test complete" appears (in tokenized form)
  ✓ No crashes or lockups
  ✓ Clean exit via semihosting

Test Duration: ~1-5 seconds

Stopping QEMU:
--------------
  
  Press: Ctrl-A, then X
  
  Or from another terminal:
  pkill qemu

## TROUBLESHOOTING

Problem: "qemu-system-arm: command not found" or QEMU not available
--------------------------------------------------------------------
Solution: Bazel downloads QEMU automatically when needed
  • This should happen automatically on first test run
  • If it fails, check your network connection
  • Verify Bazel can access CIPD: MODULE.bazel defines the QEMU download

Problem: Build fails with "Found 0 targets"
-------------------------------------------
Solution: You must use the config flag, not --platforms
  • Correct:   bazelisk build --config=k_qemu_mps2_an505 //...
  • Incorrect: bazelisk build --platforms=//pw_kernel/target/mps2_an505:mps2_an505 //...
  
  The config flag properly sets up the build configuration.

Problem: Build fails with "No such target"
-------------------------------------------
Solution: Verify you're in the correct directory
  • Ensure you're in your Pigweed workspace root
  • Check the target path exists:
    ls pw_kernel/target/mps2_an505/ipc/user/BUILD.bazel

Problem: QEMU hangs with no output
-----------------------------------
Solution: Check semihosting support
  • Verify -semihosting flag is present
  • Try adding: -d guest_errors to debug
  • Check that the ELF file is the correct architecture:
    arm-none-eabi-objdump -f <elf-file>

Problem: "Ipc test complete" doesn't appear
--------------------------------------------
Solution: IPC test failed
  • Look for error messages before hang
  • Output is tokenized - use detokenizer to read
  • Check for crash patterns (repeated errors)
  • Verify both apps are built correctly
  • Try rebuilding from clean: bazelisk clean

Problem: Cannot kill QEMU with Ctrl-A X
----------------------------------------
Solution: Alternative methods
  • Force quit: Ctrl-C (may not work in semihosting mode)
  • From another terminal: pkill -9 qemu
  • Find process: ps aux | grep qemu
    Then: kill -9 <pid>

## UNDER THE HOOD: HOW IT WORKS

Build Process:
--------------
1. Bazel uses the MPS2-AN505 platform configuration
2. Compiles Rust kernel for ARM Cortex-M33
3. Enables ARMv8-M features and PMSAv8 MPU
4. Builds initiator and handler userspace apps
5. Uses system.json5 to configure memory layout
6. Generates linker script from template
7. Links kernel + apps into single ELF image
8. Vector table placed at 0x00000000
9. Kernel code starts at 0x00000400

Runtime Execution:
------------------
1. QEMU loads ELF into emulated MPS2-AN505 memory
2. CPU starts at reset vector (0x00000004)
3. Entry point initializes hardware and PMSAv8 MPU
4. Kernel starts and creates two processes
5. PMSAv8 configures memory protection regions
6. Scheduler begins multitasking
7. Handler thread waits on IPC channel
8. Initiator sends characters 'a' through 'z'
9. Handler responds with uppercase + original
10. Test completes and exits via semihosting

PMSAv8 Memory Protection:
--------------------------
The ARMv8-M PMSAv8 MPU provides:
- 16 memory protection regions (vs 8 in PMSAv7)
- Byte-level granularity (vs power-of-2 in PMSAv7)
- Overlapping region support with priority
- Separate privileged/unprivileged access controls
- Enhanced security features for TrustZone

## COMPARISON WITH ARMv7-M

Feature                    | ARMv8-M (MPS2-AN505)    | ARMv7-M (AST1030)
---------------------------|-------------------------|-------------------
CPU                        | Cortex-M33              | Cortex-M4
Architecture               | ARMv8-M Mainline        | ARMv7-M
MPU Version                | PMSAv8                  | PMSAv7
MPU Regions                | 16                      | 8
Region Granularity         | 32 bytes                | 32 bytes (with subregions)
Region Alignment           | Any address             | Power-of-2 size aligned
TrustZone Support          | Yes                     | No
Build Config               | --config=k_qemu_mps2_an505 | --platforms=//...

## RELATED DOCUMENTATION

- ../README.md - MPS2-AN505 platform overview
- ../../ast1030/ipc/README.md - ARMv7-M IPC test guide
- BUILDING.md - Complete build instructions for all targets
- ARMV7M_INTEGRATION_STATUS.md - ARMv7-M vs ARMv8-M comparison

## EXIT QEMU

To exit QEMU: Press `Ctrl+A` then `X`
