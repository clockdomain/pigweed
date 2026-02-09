# STM32F407 IPC User-Space Test

Runs the pw_kernel IPC test on an STM32F4-Discovery board (STM32F407VG).
Two user-space processes (initiator and handler) exchange messages via
kernel IPC to verify memory protection and inter-process communication
on real ARMv7E-M hardware.

These instructions assume a WSL2 (Windows Subsystem for Linux 2)
environment with USB passthrough to access the ST-Link debugger.

## Prerequisites

- STM32F4-Discovery board connected to the Windows host via USB
- ST-Link USB forwarded to WSL2 (from Windows PowerShell as admin):
  ```powershell
  usbipd list                              # find the ST-Link bus ID
  usbipd attach --wsl --busid <BUS_ID>     # attach to WSL2
  ```
- OpenOCD installed in WSL2 (`sudo apt install openocd`)
- GDB: `gdb-multiarch` (not `arm-none-eabi-gdb`)
- Pigweed environment activated: `source activate.sh`

## Build

```sh
source activate.sh
bazelisk build //pw_kernel/target/stm32f407/ipc/user:ipc --config=k_stm32f407
```

## Flash and Run with Semihosting

The default console backend is semihosting, which requires a debugger
connection to capture output. Without a debugger, the board will
HardFault on the first log call (BKPT with no debug handler).

### Command Line

Flash, enable semihosting, and capture output:

```sh
openocd -f pw_kernel/target/stm32f407/openocd.cfg \
  -c "init" \
  -c "arm semihosting enable" \
  -c "program bazel-bin/pw_kernel/target/stm32f407/ipc/user/ipc.elf verify reset"
```

Semihosting output (tokenized) appears on stdout. Detokenize with:

```sh
cat semihosting_output.log | python3 -m pw_tokenizer.detokenize base64 \
  bazel-bin/pw_kernel/target/stm32f407/ipc/user/ipc.elf
```

### VSCode (Cortex-Debug)

A launch configuration is provided in `.vscode/launch.json` as
"STM32F407 IPC Test". It uses `gdb-multiarch` and enables semihosting
via `postLaunchCommands`. Hit F5 to flash, then Continue (F5 again)
to run. Output appears in the Debug Console.

## Expected Output

```
INF] Welcome to Maize on STM32F407 User IPC!
INF] Cortex-M early initialization
...
INF] Allocating non-privileged process 'initiator process'
INF] Allocating non-privileged thread 'initiator thread'
INF] Allocating non-privileged process 'handler process'
INF] Allocating non-privileged thread 'handler thread'
INF] IPC service starting
INF] PASSED
FTL] Target shutdown: code=0
```

## Console Backend

The platform console backend is configured in
`pw_kernel/target/stm32f407/BUILD.bazel` via the `console_backend`
label flag. To switch to UART (standalone, no debugger required):

1. Change the flag to `:console` (USART2 TX on PA2, 115200 baud)
2. Add `console_backend::init()` to `console_init()` in target.rs
3. Connect a USB-to-serial adapter to PA2 (TX) and GND
