# CH341 Large Write URB Experiment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build, safely load, and benchmark an experimental Linux `ch341` module that uses 256-byte bulk-OUT URBs over USB/IP.

**Architecture:** Keep the packaged kernel module untouched. A repo-local preparation script downloads the pinned upstream Linux v7.0 `ch341.c`, verifies its SHA-256, applies a one-line driver patch, and builds an out-of-tree module against the running kernel. Runtime instructions detach only the CH340 VHCI port, load the experimental module transiently, verify serial control, run one authorized forced flash, and rollback on any failure.

**Tech Stack:** Linux kernel module build system, Bash, upstream Linux v7.0 `ch341` driver, USB/IP, `espflash`.

---

## File Structure

- `tools/ch341-usbip/Makefile`: out-of-tree kernel module target.
- `tools/ch341-usbip/ch341-bulk-out-size.patch`: isolated 256-byte buffer change.
- `tools/ch341-usbip/prepare.sh`: pinned source download, checksum verification, and patch application.
- `tools/ch341-usbip/verify.sh`: source/patch/build preflight without loading a module.
- `tools/ch341-usbip/README.md`: load, attach, smoke-test, benchmark, and rollback commands.
- `docs/QUALITY.md`: route reviewers to the experiment's evidence requirements.

### Task 1: Add Reproducible Driver Preparation

**Files:**
- Create: `tools/ch341-usbip/Makefile`
- Create: `tools/ch341-usbip/ch341-bulk-out-size.patch`
- Create: `tools/ch341-usbip/prepare.sh`
- Create: `tools/ch341-usbip/verify.sh`

- [ ] **Step 1: Write the failing verification script**

Create `tools/ch341-usbip/verify.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
test "$(uname -r)" = "7.0.0-27-generic"
test -d "/lib/modules/$(uname -r)/build"
"$root/prepare.sh"
rg -n '^\s*\.bulk_out_size\s*=\s*256,' "$root/build/ch341.c"
make -C "/lib/modules/$(uname -r)/build" M="$root/build" modules
test "$(modinfo -F vermagic "$root/build/ch341.ko" | awk '{print $1}')" = "$(uname -r)"
```

- [ ] **Step 2: Run verification to prove it fails before implementation**

Run: `bash tools/ch341-usbip/verify.sh`

Expected: FAIL because `tools/ch341-usbip/prepare.sh` does not exist.

- [ ] **Step 3: Add the kernel module Makefile**

Create `tools/ch341-usbip/Makefile`:

```make
obj-m += ch341.o
```

The preparation script copies this file into `build/` before invoking Kbuild.

- [ ] **Step 4: Add the isolated driver patch**

Create `tools/ch341-usbip/ch341-bulk-out-size.patch`:

```diff
--- a/ch341.c
+++ b/ch341.c
@@ -871,6 +871,7 @@ static struct usb_serial_driver ch341_device = {
 	},
 	.id_table          = id_table,
 	.num_ports         = 1,
+	.bulk_out_size     = 256,
 	.open              = ch341_open,
 	.dtr_rts	   = ch341_dtr_rts,
 	.carrier_raised	   = ch341_carrier_raised,
```

- [ ] **Step 5: Add pinned source preparation**

Create `tools/ch341-usbip/prepare.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
build="$root/build"
url=https://raw.githubusercontent.com/torvalds/linux/v7.0/drivers/usb/serial/ch341.c
expected=42dfb2e94a8e8a82cedc464a71e41f721caa36917d31d81dd792cb5ea4c03f2f

mkdir -p "$build"
curl --fail --silent --show-error --location "$url" --output "$build/ch341.c.upstream"
printf '%s  %s\n' "$expected" "$build/ch341.c.upstream" | sha256sum --check --status
cp "$build/ch341.c.upstream" "$build/ch341.c"
cp "$root/Makefile" "$build/Makefile"
patch --silent --directory "$build" --input "$root/ch341-bulk-out-size.patch"
```

Run: `chmod +x tools/ch341-usbip/prepare.sh tools/ch341-usbip/verify.sh`

- [ ] **Step 6: Run source and build verification**

Run: `bash tools/ch341-usbip/verify.sh`

Expected: the patched line is printed, Kbuild exits zero, and the built module vermagic equals `7.0.0-27-generic`.

- [ ] **Step 7: Commit the reproducible module experiment**

```bash
git add tools/ch341-usbip
git commit -m "experiment: build CH341 with larger write URBs"
```

### Task 2: Document Safe Runtime And Rollback

**Files:**
- Create: `tools/ch341-usbip/README.md`
- Modify: `docs/QUALITY.md`

- [ ] **Step 1: Write the runtime checklist**

Create `tools/ch341-usbip/README.md` with these exact phases:

```markdown
# CH341 USB/IP Large-URB Experiment

This experiment is only for Ubuntu `7.0.0-27-generic`, CH340 `1a86:7523`,
NanoPi `10.10.61.72`, and remote bus `5-1`.

## Build

\`\`\`bash
bash tools/ch341-usbip/verify.sh
modinfo tools/ch341-usbip/build/ch341.ko
\`\`\`

## Preflight

\`\`\`bash
sudo -v
test -z "$(lsof -t /dev/ttyUSB0 2>/dev/null)"
usbip port
\`\`\`

Detach only the single attached port whose VID:PID is `1a86:7523`:

\`\`\`bash
mapfile -t ports < <(usbip port | awk '/^Port [0-9]+:/ {p=$2; sub(":", "", p)} /1a86:7523/ {print p}')
test "${#ports[@]}" -eq 1
port=${ports[0]}
sudo usbip detach -p "$port"
sudo modprobe -r ch341
sudo insmod tools/ch341-usbip/build/ch341.ko
test "$(cat /sys/module/ch341/srcversion)" = "$(modinfo -F srcversion tools/ch341-usbip/build/ch341.ko)"
\`\`\`

Attach the same remote target and smoke-test control transfers:

\`\`\`bash
sudo usbip --tcp-port 3240 attach -r 10.10.61.72 -b 5-1
ls -l /dev/ttyUSB0
espflash board-info --port /dev/ttyUSB0
\`\`\`

## Rollback

Identify the single CH340 port again; do not assume its port number:

\`\`\`bash
usbip port
mapfile -t ports < <(usbip port | awk '/^Port [0-9]+:/ {p=$2; sub(":", "", p)} /1a86:7523/ {print p}')
test "${#ports[@]}" -eq 1
port=${ports[0]}
sudo usbip detach -p "$port"
sudo modprobe -r ch341
sudo modprobe ch341
modinfo -F filename ch341
\`\`\`

The final `modinfo` path must be under `/lib/modules/$(uname -r)/`.
```

- [ ] **Step 2: Add the quality routing note**

Append to `docs/QUALITY.md`:

```markdown
## CH341 Large-URB Experiment

The optional host-specific CH341 performance experiment is documented in
`tools/ch341-usbip/README.md`. A passing experiment requires matching vermagic,
successful serial-control smoke test, a complete forced flash with hash
verification, before/after elapsed times, and an explicit rollback check.
Compilation or USB enumeration alone is not acceptance evidence.
```

- [ ] **Step 3: Check documentation and shell syntax**

Run:

```bash
bash -n tools/ch341-usbip/prepare.sh tools/ch341-usbip/verify.sh
git diff --check
```

Expected: both commands exit zero.

- [ ] **Step 4: Commit runtime documentation**

```bash
git add tools/ch341-usbip/README.md docs/QUALITY.md
git commit -m "docs: define CH341 module benchmark and rollback"
```

### Task 3: Load And Smoke-Test The Experimental Module

**Files:**
- No tracked files modified.

- [ ] **Step 1: Capture stock state**

Run:

```bash
uname -r
modinfo -F filename ch341
modinfo -F vermagic ch341
usbip port
lsof /dev/ttyUSB0
```

Expected: kernel is `7.0.0-27-generic`, stock module is under `/lib/modules`, and no process holds the serial device.

- [ ] **Step 2: Obtain sudo authorization in the active terminal**

Run: `sudo -v`

Expected: exit zero. Stop here if sudo authorization is unavailable.

- [ ] **Step 3: Detach only CH340 and load the experimental module**

Resolve exactly one CH340 port from `usbip port`, then run:

```bash
mapfile -t ports < <(usbip port | awk '/^Port [0-9]+:/ {p=$2; sub(":", "", p)} /1a86:7523/ {print p}')
test "${#ports[@]}" -eq 1
port=${ports[0]}
sudo usbip detach -p "$port"
sudo modprobe -r ch341
sudo insmod "$PWD/tools/ch341-usbip/build/ch341.ko"
```

Expected: every command exits zero, `lsmod` shows `ch341`, and
`cat /sys/module/ch341/srcversion` matches
`modinfo -F srcversion tools/ch341-usbip/build/ch341.ko`.

- [ ] **Step 4: Reattach and smoke-test without flashing**

Run:

```bash
sudo usbip --tcp-port 3240 attach -r 10.10.61.72 -b 5-1
udevadm settle
ls -l /dev/ttyUSB0
espflash board-info --port /dev/ttyUSB0
journalctl -k --since '-2 minutes' --no-pager | rg 'ch341|vhci|timeout|error'
```

Expected: CH340 enumerates, `board-info` identifies ESP32-P4, and kernel logs contain no control timeout or disconnect.

- [ ] **Step 5: Roll back immediately on smoke-test failure**

Resolve the current single CH340 port from `usbip port`, then run:

```bash
mapfile -t ports < <(usbip port | awk '/^Port [0-9]+:/ {p=$2; sub(":", "", p)} /1a86:7523/ {print p}')
test "${#ports[@]}" -eq 1
port=${ports[0]}
sudo usbip detach -p "$port"
sudo modprobe -r ch341
sudo modprobe ch341
```

Expected: `modinfo -F filename ch341` returns the packaged module path.

### Task 4: Run One Authorized Forced Flash Benchmark

**Files:**
- Create runtime evidence under `/home/hieunm/workspace/poc/p4mipi-demo/evidence/hardware-flash/`; do not commit it from this repo.

- [ ] **Step 1: Confirm the experimental module and target immediately before flash**

Run:

```bash
modinfo -F filename ch341
usbip port
ls -l /dev/ttyUSB0
```

Expected: `cat /sys/module/ch341/srcversion` matches the experimental module,
and the attached target is CH340 `1a86:7523` from NanoPi bus `5-1`.

- [ ] **Step 2: Run the forced benchmark with elapsed-time evidence**

Run from `/home/hieunm/workspace/poc/p4mipi-demo`:

```bash
set -o pipefail
/usr/bin/time -f 'elapsed=%e seconds' \
  make flash PORT=/dev/ttyUSB0 CONFIRM_FLASH=1 CONFIRM_PORT=/dev/ttyUSB0 \
  BAUD=921600 ESPFLASH_FLASH_FLAGS=--no-skip \
  2>&1 | tee evidence/hardware-flash/ch341-256-urb-921600.log
```

Expected: `espflash` reports successful write and hash verification; elapsed time is materially below the 522.6-second baseline.

- [ ] **Step 3: Capture post-benchmark kernel and network evidence**

Run:

```bash
usbip port
ss -tin dst 10.10.61.72:3240
journalctl -k --since '-15 minutes' --no-pager | rg 'ch341|vhci|timeout|error'
```

Expected: attachment remains present and there are no new disconnect or control-timeout errors.

- [ ] **Step 4: Roll back to the packaged driver**

Resolve exactly one CH340 port from the fresh `usbip port` output, then run:

```bash
mapfile -t ports < <(usbip port | awk '/^Port [0-9]+:/ {p=$2; sub(":", "", p)} /1a86:7523/ {print p}')
test "${#ports[@]}" -eq 1
port=${ports[0]}
sudo usbip detach -p "$port"
sudo modprobe -r ch341
sudo modprobe ch341
modinfo -F filename ch341
```

Expected: the final filename is under `/lib/modules/7.0.0-27-generic/`.

### Task 5: Repository Quality Gate And Result Report

**Files:**
- Modify the design or runtime README only if measured behavior changes an invariant.

- [ ] **Step 1: Run repository quality gates**

Run:

```bash
cargo fmt --check
cargo clippy
cargo test
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 2: Report measured status without overclaiming**

Report the module vermagic, smoke-test result, exact flash result, elapsed time,
throughput, relevant kernel errors, rollback result, and comparison with 522.6
seconds. If forced flash did not complete and verify, report the experiment as
failed even if early throughput appeared faster.
