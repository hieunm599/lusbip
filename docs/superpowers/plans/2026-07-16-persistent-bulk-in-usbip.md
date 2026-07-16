# Persistent Bulk-IN USB/IP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve low-latency UART traffic by keeping one physical bulk-IN transfer pending per serial endpoint and completing the matching USB/IP URB only when data arrives or it is unlinked.

**Architecture:** Split a USB/IP connection into a command reader and serialized response writer. Maintain endpoint workers that own their `nusb::Endpoint<Bulk, In>` state; an endpoint worker submits a physical read once, waits asynchronously for it, and maps a completion to the oldest pending USB/IP sequence number. `USBIP_CMD_UNLINK` removes the matching pending request and cancels an idle physical read when no client requests remain.

**Tech Stack:** Rust, Tokio, `nusb`, existing internal USB/IP protocol module.

---

### Task 1: Model pending submit and unlink bookkeeping

**Files:**
- Modify: `src/usbip_server/mod.rs`
- Test: `src/usbip_server/mod.rs`

- [ ] **Step 1: Write failing unit tests for pure pending-request helpers**

Add tests proving a FIFO queue returns the oldest sequence number for a completion and that an unlink removes only its target sequence number:

```rust
#[test]
fn pending_bulk_in_completes_in_submission_order() {
    let mut pending = PendingBulkIn::default();
    pending.push(10);
    pending.push(20);
    assert_eq!(pending.complete_next(), Some(10));
    assert_eq!(pending.complete_next(), Some(20));
}

#[test]
fn pending_bulk_in_unlink_removes_only_target_request() {
    let mut pending = PendingBulkIn::default();
    pending.push(10);
    pending.push(20);
    assert!(pending.unlink(10));
    assert_eq!(pending.complete_next(), Some(20));
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test pending_bulk_in`

Expected: compile failure because `PendingBulkIn` does not exist.

- [ ] **Step 3: Implement the pure queue helper**

Implement `PendingBulkIn` with `VecDeque<u32>` and methods `push`, `complete_next`, `unlink`, and `is_empty`. Do not put I/O in this helper.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test pending_bulk_in`

Expected: both tests pass.

### Task 2: Add a persistent physical bulk-IN worker

**Files:**
- Modify: `src/usbip_server/host.rs`
- Modify: `src/usbip_server/mod.rs`
- Test: `src/usbip_server/host.rs`

- [ ] **Step 1: Write a failing lifecycle test for worker state**

Test that an endpoint is marked active after the first request and is not submitted again while active:

```rust
#[test]
fn bulk_in_worker_submits_once_while_a_read_is_pending() {
    let mut state = BulkInState::default();
    assert!(state.start_if_idle());
    assert!(!state.start_if_idle());
    state.complete();
    assert!(state.start_if_idle());
}
```

- [ ] **Step 2: Run the test and confirm failure**

Run: `cargo test bulk_in_worker_submits_once`

Expected: compile failure because `BulkInState` does not exist.

- [ ] **Step 3: Implement an endpoint-owned worker**

Create a worker that owns `nusb::Endpoint<Bulk, In>`, calls `submit(Buffer::new(requested_len))` once, then awaits `next_complete()` without `transfer_blocking`. Reuse the endpoint and completion buffer. It must never issue a second transfer while `pending() > 0`.

- [ ] **Step 4: Run focused tests**

Run: `cargo test bulk_in_worker_submits_once`

Expected: pass.

### Task 3: Wire worker completions and unlink replies into the USB/IP connection

**Files:**
- Modify: `src/usbip_server/mod.rs`
- Test: `src/usbip_server/mod.rs`

- [ ] **Step 1: Add a failing response-routing test**

Test that an endpoint completion produces `USBIP_RET_SUBMIT` with the stored sequence number and exact `actual_length`, and an unlink produces `USBIP_RET_UNLINK` without completing another request.

- [ ] **Step 2: Run the focused test and confirm failure**

Run: `cargo test persistent_bulk_in_routes`

Expected: fail before routing is implemented.

- [ ] **Step 3: Split reader and writer responsibilities**

Use Tokio channels: the reader accepts `USBIP_CMD_SUBMIT` and `USBIP_CMD_UNLINK`; endpoint workers send completed responses to one writer task. Keep endpoint ownership single-threaded, so `nusb` never reports `endpoint already in use`.

- [ ] **Step 4: Implement unlink semantics**

On `USBIP_CMD_UNLINK`, remove the requested seqnum from only the matching endpoint queue, send `USBIP_RET_UNLINK`, and cancel the physical transfer only when that endpoint has no remaining pending requests.

- [ ] **Step 5: Run focused tests**

Run: `cargo test persistent_bulk_in_routes`

Expected: pass.

### Task 4: Verify in the LAN lab

**Files:**
- Modify: `docs/QUALITY.md` only if an acceptance command changes.

- [ ] **Step 1: Run local quality gate**

Run:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Expected: all pass.

- [ ] **Step 2: Deploy the native aarch64 binary to NanoPi**

Build in `/home/pi/lusbip`, install `/home/pi/bin/lusbip`, and restart only `lusbip.service`.

- [ ] **Step 3: CH340 E2E**

On Ubuntu, attach `5-1`, run `espflash monitor --port /dev/ttyUSB0`, and verify that the boot log already observed on NanoPi reaches the client without a USB/IP disconnect.

- [ ] **Step 4: ESP USB-JTAG/serial E2E**

Attach the ESP native ACM device and run `espflash flash --before no-reset --after no-reset --port /dev/ttyACM0` against the P4 image. Verify completion before claiming flashing works.

