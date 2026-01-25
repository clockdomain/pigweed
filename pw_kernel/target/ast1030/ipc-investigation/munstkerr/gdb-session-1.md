# Task : Compare syscall #33 (works) vs #34 (fails) to see exactly when/where the frame gets corrupted.

The key steps:

1. Track MSP drift from syscall #30 onwards
2. Stop at syscall #33 (the last working one) and dump the frame
3. Compare with #34 (the failing one) to see what changes
4. Set watchpoint on the psp field to catch the corruption in action

The main question: Is MSP drifting between syscalls (stack exhaustion) or is the frame pointer calculation just wrong?

tmux new-session -s ast1030-debug \; \
  send-keys 'qemu-system-arm -machine ast1030-evb -cpu cortex-m4 -bios none -nographic -serial mon:stdio -kernel bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -semihosting-config enable=on,target=native -S -s 2>&1 | python3 -m pw_tokenizer.detokenize base64 bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf' C-m \; \
  split-window -h \; \
  send-keys 'sleep 2 && gdb-multiarch bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf -x pw_kernel/target/ast1030/ipc-investigation/debug.gdb -ex "target remote :1234"' C-m


delete
set $svc_count = 0
break SVCall if ++$svc_count == 33
continue