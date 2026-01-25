# Next GDB Session Plan - 2026-01-24 Session A

## Objective

Trace the frame pointer through the syscall path to find where the 0x20 byte offset occurs.

## Hypothesis to Test

PendSV fires during blocking syscall and saves the wrong frame address.

## Session Setup

```gdb
# Start fresh
delete

# Load symbols
file bazel-bin/pw_kernel/target/ast1030/ipc/user/ipc.elf

# Connect
target remote :1234
```

## Step 1: Track Frame Pointer Through SVCall

```gdb
# Break at SVCall entry (before frame is built)
# Use *SVCall for naked assembly function
break *SVCall
commands
  silent
  printf "\n=== SVCall Entry ===\n"
  printf "MSP before push: 0x%08x\n", $msp
  continue
end

# Break after frame is built (at handle_svc call)
# handle_svc is at 0x11c8 (from debug.gdb output)
break *handle_svc
commands
  silent
  printf "\n=== handle_svc Entry ===\n"
  printf "r0 (frame_ptr): 0x%08x\n", $r0
  printf "MSP: 0x%08x\n", $msp
  # Dump the frame
  printf "Frame contents:\n"
  x/12wx $r0
  continue
end
```

## Step 2: Track PendSV

```gdb
# Enable PendSV breakpoint
break PendSV
commands
  silent
  printf "\n=== PendSV Entry ===\n"
  printf "MSP: 0x%08x\n", $msp
  printf "r0 (frame): 0x%08x\n", $r0
  continue
end
```

## Step 3: Track svc_return

```gdb
# Break at svc_return entry
break svc_return
commands
  silent
  printf "\n=== svc_return Entry ===\n"
  printf "r0 (frame to restore): 0x%08x\n", $r0
  printf "Frame contents:\n"
  x/12wx $r0
  # Validate frame
  printf "Expected: psp at +0x20, control at +0x24, return at +0x28\n"
  printf "psp: 0x%08x\n", *(unsigned int*)($r0 + 0x20)
  printf "control: 0x%08x\n", *(unsigned int*)($r0 + 0x24)
  printf "return: 0x%08x\n", *(unsigned int*)($r0 + 0x28)
  continue
end
```

## Step 4: Catch the Fault

```gdb
break MemoryManagement
```

## Step 5: Run and Analyze

```gdb
continue
```

## What to Look For

1. **At SVCall entry**: Note MSP value
2. **At handle_svc**: Verify r0 matches MSP after frame push
3. **At PendSV** (if it fires): Does it save the correct frame address?
4. **At svc_return**: Is r0 the same as it was at handle_svc?
5. **At fault**: Compare addresses from above

## Key Questions

| Checkpoint | Expected | If Wrong |
|------------|----------|----------|
| handle_svc r0 | Valid frame addr | SVCall frame build bug |
| PendSV frame | Same as handle_svc r0 | Context switch bug |
| svc_return r0 | Same as handle_svc r0 | Return value corruption |
| svc_return r0 | 0x20 less than fault MSP | **CONFIRMS 0x20 OFFSET** |

## Alternative: Minimal Breakpoints

If too many breakpoints change timing, use only:

```gdb
delete
break MemoryManagement
break svc_return
continue
```

Then when svc_return hits, manually inspect:
```gdb
printf "Frame at r0: 0x%08x\n", $r0
x/12wx $r0
```

## Quick Commands Reference

```gdb
# Check frame validity
x/12wx $r0

# Check what's 0x20 below
x/12wx ($r0 - 0x20)

# Validate control/return fields
p/x *(unsigned int*)($r0 + 0x24)  # control (should be 0-3)
p/x *(unsigned int*)($r0 + 0x28)  # return (should be 0xFFFFFFxx)
```
