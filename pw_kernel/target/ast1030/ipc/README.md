HOWTO: Run IPC Test on ASPEED AST1030

## PURPOSE

This guide explains how to build and run the Inter-Process Communication (IPC)
test for the ASPEED AST1030 target using QEMU emulation. The IPC test 
demonstrates communication between two userspace applications running under the
Pigweed kernel.

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

## SYSTEM CONFIGURATION

The IPC test uses the following memory layout on AST1030:

  Memory Region                  | Start Addr  | Size
  -------------------------------|-------------|--------
  Vector Table                   | 0x00000000  | 1KB
  Kernel Code (in RAM)           | 0x00000420  | 256KB
  Kernel Data RAM                | 0x00040420  | 384KB
  Initiator App Code             | (kernel+)   | 128KB
  Initiator App RAM              | (kernel+)   | 16KB
  Handler App Code               | (kernel+)   | 128KB
  Handler App RAM                | (kernel+)   | 16KB

Total RAM Usage: ~640KB (fits within AST1030's available SRAM)

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
   Pigweed uses QEMU 8.2+ which includes AST1030 support.

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
    --platforms=//pw_kernel/target/ast1030:ast1030 \
    //pw_kernel/target/ast1030/ipc/user:ipc

This command:
  • Configures for ARM Cortex-M4 (AST1030)
  • Builds the kernel with the target configuration
  • Compiles both initiator and handler apps
  • Links everything into a single ELF image
  • Outputs: bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

Build Time: ~30-90 seconds (depending on system)

Expected Output:
  INFO: Build completed successfully

Step 2: Verify the Binary
--------------------------

Check that the ELF file was created:

  ls -lh bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
  
  Expected: ~500KB - 1MB ELF file

Optional: Inspect the binary:

  file bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf
  
  Expected output:
  ELF 32-bit LSB executable, ARM, EABI5 version 1 (SYSV), statically linked

## RUNNING THE IPC TEST IN QEMU

Command to Run:
---------------

  qemu-system-arm \
    -M ast1030-evb \
    -nographic \
    -semihosting \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

Command Breakdown:
------------------
  -M ast1030-evb      Use ASPEED AST1030 evaluation board machine
  -nographic          Run without graphical window (console only)
  -semihosting        Enable ARM semihosting for console output
  -kernel <file>      Load ELF kernel image into emulated memory

Running the Test:
-----------------

  cd <pigweed-workspace>
  
  qemu-system-arm -M ast1030-evb -nographic -semihosting \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

Expected Output:
----------------

  [INFO ] <pw_kernel::kernel::kern_task> Kernel starting
  [INFO ] <pw_kernel::subsys::console::semihosting> Console initialized (semihosting)
  [INFO ] <pw_kernel::kernel::kern_task> Starting scheduler
  [INFO ] <app_handler> IPC service starting
  [INFO ] <app_initiator> Ipc test starting
  [INFO ] <app_initiator> Ipc test complete
  [INFO ] <pw_kernel::kernel::kern_task> All processes complete

Test Success Indicators:
  ✓ "Ipc test starting" appears
  ✓ "Ipc test complete" appears
  ✓ No errors or panics
  ✓ "All processes complete" at the end

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

Problem: Build fails with "No such target"
-------------------------------------------
Solution: Verify you're in the correct directory
  • Ensure you're in your Pigweed workspace root
  • Check the target path exists:
    ls pw_kernel/target/ast1030/ipc/user/BUILD.bazel

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
  • Check for "ERROR" or "panic" in output
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
1. Bazel uses the AST1030 platform configuration
2. Compiles Rust kernel for ARM Cortex-M4
3. Builds initiator and handler userspace apps
4. Uses system.json5 to configure memory layout
5. Generates linker script from template
6. Links kernel + apps into single ELF image
7. Vector table placed at 0x00000000
8. Kernel code starts at 0x00000420

Runtime Execution:
------------------
1. QEMU loads ELF into emulated AST1030 memory
2. CPU starts at reset vector (0x00000004)
3. Entry point initializes hardware
4. Kernel starts and creates two processes
5. Scheduler begins multitasking
6. Handler thread waits on IPC channel
7. Initiator thread sends 26 messages
8. Each message triggers context switch to handler
9. Handler processes and responds
10. Initiator validates all responses
11. Both processes complete
12. Kernel reports completion

Memory Protection:
------------------
  • Each app runs in separate process
  • Memory isolation enforced by kernel
  • IPC is only way to communicate
  • Stack overflow protection
  • No direct memory sharing

## NEXT STEPS

1. Run on Real Hardware
   ---------------------
   • Flash the ELF to real AST1030 board
   • Use OpenOCD for debugging
   • Connect UART for console output

2. Modify the Test
   ----------------
   • Edit: pw_kernel/tests/ipc/user/initiator.rs
   • Edit: pw_kernel/tests/ipc/user/handler.rs
   • Rebuild and rerun

3. Add More Apps
   --------------
   • Copy IPC test structure
   • Add new apps to system.json5
   • Define additional IPC channels

4. Performance Analysis
   ---------------------
   • Add timing measurements
   • Count context switches
   • Measure IPC latency

5. Port to AST1060
   ----------------
   • Similar process, different memory layout
   • More RAM available (1MB+)
   • May need different QEMU machine type

## RELATED DOCUMENTATION

  • HOWTO: Build Pigweed for ASPEED AST1030.md
  • pw_kernel/target/ast1030/README.md (if exists)
  • pw_kernel/tests/ipc/README.md (if exists)
  • QEMU AST1030 documentation

## SUMMARY: QUICK REFERENCE

Build:
  bazelisk build --platforms=//pw_kernel/target/ast1030:ast1030 \
    //pw_kernel/target/ast1030/ipc/user:ipc

Run:
  qemu-system-arm -M ast1030-evb -nographic -semihosting \
    -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

Stop:
  Ctrl-A, then X
  (or: pkill qemu)

Success:
  Look for "Ipc test complete" in output
