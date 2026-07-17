# USB/IP Bulk OUT Worker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove per-URB synchronous blocking from serial bulk-OUT forwarding so CH340 flashing over USB/IP can sustain practical throughput.

**Architecture:** Add one persistent bulk-OUT worker for each imported bulk-OUT endpoint. The connection handler routes bulk-OUT submit commands to that worker, which performs host transfers sequentially and emits USB/IP completion responses back to the single socket writer. Control and interrupt URBs retain their existing synchronous path; this limits the behavior change to bulk-OUT traffic and preserves endpoint byte order.

**Tech Stack:** Rust, Tokio `mpsc`/`JoinSet`, `nusb` bulk endpoint API, existing USB/IP protocol types.

---

### Task 1: Cover bulk-OUT completion routing with a regression test

**Files:**
- Modify: `src/usbip_server/mod.rs`

- [ ] **Step 1: Write the failing test**

Add a unit test for a new helper that records an accepted bulk-OUT `seqnum` and returns `true` exactly once when the worker completion arrives. The test must prove that duplicate completions are ignored.

```rust
#[test]
fn bulk_out_completion_is_written_once() {
    let mut pending = PendingBulkOut::default();
    pending.push(7);

    assert!(pending.complete(7));
    assert!(!pending.complete(7));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test bulk_out_completion_is_written_once`

Expected: compilation failure because `PendingBulkOut` does not exist.

- [ ] **Step 3: Implement the smallest state helper**

Add `PendingBulkOut` beside `PendingBulkIn`, backed by `HashSet<u32>`, with `push` and `complete` methods.

- [ ] **Step 4: Run the focused test**

Run: `cargo test bulk_out_completion_is_written_once`

Expected: PASS.

### Task 2: Add the persistent worker and route only bulk-OUT URBs through it

**Files:**
- Modify: `src/usbip_server/mod.rs`

- [ ] **Step 1: Write the failing worker-routing test**

Add a pure routing test for a helper that reports `true` only for an endpoint whose attributes are bulk and whose USB/IP direction is OUT.

```rust
#[test]
fn only_bulk_out_urbs_use_the_persistent_worker() {
    assert!(uses_bulk_out_worker(EndpointAttributes::Bulk as u8, 0));
    assert!(!uses_bulk_out_worker(EndpointAttributes::Bulk as u8, 1));
    assert!(!uses_bulk_out_worker(EndpointAttributes::Interrupt as u8, 0));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test only_bulk_out_urbs_use_the_persistent_worker`

Expected: compilation failure because `uses_bulk_out_worker` does not exist.

- [ ] **Step 3: Implement worker lifecycle and response path**

Create `BulkOutRequest`, `BulkOutCommand`, and `BulkOutEvent`; start one worker per bulk-OUT endpoint after import; stop it with existing worker cleanup. The worker opens `nusb::Endpoint<Bulk, Out>`, submits each request in receive order, awaits completion, and emits a `USBIP_RET_SUBMIT` success/failure response. In `handler`, enqueue matching submit URBs and let only the event branch write their completion responses.

- [ ] **Step 4: Preserve existing paths**

Leave all control, interrupt, non-bulk, and bulk-IN handling on their existing code paths. Preserve response headers through the existing `handle_usbip_cmd_submit` conventions.

- [ ] **Step 5: Run focused tests**

Run: `cargo test bulk_out_completion_is_written_once && cargo test only_bulk_out_urbs_use_the_persistent_worker`

Expected: PASS.

### Task 3: Verify, review, and deploy

**Files:**
- Modify: `src/usbip_server/mod.rs`
- Modify: `docs/superpowers/plans/2026-07-17-bulk-out-worker.md`

- [ ] **Step 1: Run the Rust quality gate**

Run:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Expected: all exit successfully.

- [ ] **Step 2: Review the implementation independently**

Review the diff for endpoint ordering, duplicate completion prevention, disconnect cleanup, and the unchanged control/interrupt paths. Address any critical or important finding before deploy.

- [ ] **Step 3: Build and install on Ubuntu**

Run `cargo build --release`, then install the new binary to `~/.local/bin/lusbip`.

- [ ] **Step 4: Build and install on NanoPi**

Copy the source archive to `pi@10.10.61.72`, build natively with Cargo, install `/home/pi/bin/lusbip`, then restart `lusbip.service`.

- [ ] **Step 5: Validate hardware behavior**

Attach CH340 from Ubuntu and run the authorized flash command with `BAUD=921600`. Record elapsed time and verify `/dev/ttyUSB0` remains present.
