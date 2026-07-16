# Import Device Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make USB/IP import preserve the already-claimed host device rather than resetting and reopening it.

**Architecture:** Treat importing as an ownership transition from `available_devices` to `used_devices`; retain the `UsbDevice` that was opened and whose interfaces were claimed during inventory. Physical reset remains limited to explicit USB/IP reset handling, so the invariant applies to every USB device class rather than a particular VID/PID.

**Tech Stack:** Rust 2024, Tokio, nusb, existing internal USB/IP server.

---

### Task 1: Define and test the import lifecycle policy

**Files:**
- Modify: `src/usbip_server/mod.rs:333-349`
- Modify: `src/usbip_server/mod.rs` (add `#[cfg(test)]` module at EOF)

- [ ] **Step 1: Write the failing test**

Add a test-only assertion for a new `prepare_device_for_import` helper. It must make the lifecycle rule explicit and prove ordinary import does not request a reset/reopen:

```rust
#[cfg(test)]
mod tests {
    use super::{prepare_device_for_import, UsbDevice};

    #[test]
    fn import_preserves_the_existing_claimed_device() {
        let device = UsbDevice {
            bus_id: "7-1".into(),
            ..UsbDevice::default()
        };

        let prepared = prepare_device_for_import(device.clone());

        assert_eq!(prepared.bus_id, device.bus_id);
    }
}
```

- [ ] **Step 2: Run the new test and verify it fails**

Run:

```bash
cargo test usbip_server::tests::import_preserves_the_existing_claimed_device
```

Expected: compilation fails because `prepare_device_for_import` does not exist.

- [ ] **Step 3: Implement the minimal lifecycle policy**

Add this pure helper directly above `impl UsbIpServer`:

```rust
fn prepare_device_for_import(device: UsbDevice) -> UsbDevice {
    device
}
```

Then replace the reset/reopen block in `UsbIpServer::occupy` with:

```rust
let device = prepare_device_for_import(device);
```

Delete `reset_and_reopen_host_device` and remove its now-unused `Duration` import. Do not add device-class, vendor, product, or bus-id conditions.

- [ ] **Step 4: Run the focused test and verify it passes**

Run:

```bash
cargo test usbip_server::tests::import_preserves_the_existing_claimed_device
```

Expected: 1 passing test.

- [ ] **Step 5: Commit the implementation**

```bash
git add src/usbip_server/mod.rs
git commit -m "fix: preserve claimed device on usbip import"
```

### Task 2: Verify source quality and deploy the native server

**Files:**
- Modify: `docs/superpowers/specs/2026-07-14-import-device-lifecycle-design.md` (only if verification changes a documented fact)
- Modify: `docs/superpowers/plans/2026-07-14-import-device-lifecycle.md` (check off executed steps)

- [ ] **Step 1: Run the complete local quality gate**

Run:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Expected: every command exits 0.

- [ ] **Step 2: Build natively on Nano Pi**

Copy the source tree excluding `.git` and `target` to `/home/pi/lusbip-import-lifecycle`, then run:

```bash
. "$HOME/.cargo/env"
cd /home/pi/lusbip-import-lifecycle
cargo build --release
```

Expected: native `aarch64` release binary exits successfully from `--version`.

- [ ] **Step 3: Replace the server atomically and restart it**

On Nano Pi, preserve the current binary and stop only the current `lusbip` listener on port 3240. Install the new release binary at `/home/pi/bin/lusbip`, then start:

```bash
nohup sudo env RUST_LOG=lusbip::usbip_server=warn \
  /home/pi/bin/lusbip server --host 0.0.0.0 --port 3240 --background \
  > /home/pi/lusbip-server.log 2>&1 &
```

Expected: `sudo ss -ltnp` shows exactly one `lusbip` listener on `0.0.0.0:3240`.

- [ ] **Step 4: Verify the end-to-end CH340 open path**

On Ubuntu, detach only the target CH340 USB/IP port, attach bus id `7-1` from `10.10.61.72`, then run:

```bash
python3 -c 'import os; fd=os.open("/dev/ttyUSB0", os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK); print("opened", fd); os.close(fd)'
```

Expected: the command prints `opened <fd>` and exits 0. Confirm `/home/pi/lusbip-server.log` contains no `did not claim interface` or `device disconnected` message for that import.

- [ ] **Step 5: Commit documentation only if it changed**

```bash
git add docs/superpowers
git commit -m "docs: record import lifecycle verification"
```
