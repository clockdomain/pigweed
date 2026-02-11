#!/bin/bash
# Flash IPC test to STM32F407G-DISC1 board
#
# Usage: ./flash.sh [elf_file]
#
# If no ELF file is specified, uses the default IPC test binary.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIGWEED_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Default ELF file
ELF_FILE="${1:-$PIGWEED_ROOT/bazel-bin/pw_kernel/target/stm32f407/ipc/user/ipc.elf}"

if [[ ! -f "$ELF_FILE" ]]; then
    echo "Error: ELF file not found: $ELF_FILE"
    echo ""
    echo "Build it first with:"
    echo "  bazelisk build //pw_kernel/target/stm32f407/ipc/user:ipc --config=k_stm32f407"
    exit 1
fi

echo "Flashing: $ELF_FILE"

# Use OpenOCD to flash
openocd -f "$SCRIPT_DIR/openocd.cfg" \
    -c "program $ELF_FILE verify reset exit"

echo "Flash complete!"
