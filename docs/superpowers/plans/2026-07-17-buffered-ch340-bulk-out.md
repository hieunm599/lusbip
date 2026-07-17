# Buffered CH340 Bulk OUT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coalesce the CH340's small USB/IP bulk-OUT URBs into bounded physical USB writes without allowing an ESP response to overtake buffered command bytes.

**Architecture:** The bulk-OUT worker owns an `nusb::EndpointWrite<Bulk>` with a 4 KiB buffer and returns USB/IP completion once the host has accepted bytes into that bounded buffer. Each bulk-IN worker holds the matching OUT-worker sender and requests a flush barrier before it submits an IN transfer. The barrier waits for queued writes to finish, preserving serial command/response ordering. A bounded Tokio channel provides backpressure.

**Tech Stack:** Rust, Tokio `mpsc`/`oneshot`, `nusb::io::EndpointWrite`, existing USB/IP protocol workers.

---

### Task 1: Model write-buffer barrier state

**Files:**
- Modify: `src/usbip_server/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add tests that a buffered OUT state requires a flush after accepting bytes, and that `mark_flushed` clears only that requirement.

```rust
#[test]
fn buffered_bulk_out_requires_a_flush_before_reading() {
    let mut state = BufferedBulkOutState::default();
    state.accept(32);
    assert!(state.needs_flush());
    state.mark_flushed();
    assert!(!state.needs_flush());
}
```

- [ ] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test buffered_bulk_out_requires_a_flush_before_reading --lib`

Expected: unresolved `BufferedBulkOutState`.

- [ ] **Step 3: Implement the minimal state helper**

Add a `BufferedBulkOutState` that tracks whether the writer has accepted bytes not yet flushed.

- [ ] **Step 4: Run the focused test and confirm it passes**

Run: `cargo test buffered_bulk_out_requires_a_flush_before_reading --lib`

Expected: PASS.

### Task 2: Buffer bulk OUT and add the bulk-IN flush barrier

**Files:**
- Modify: `src/usbip_server/mod.rs`

- [ ] **Step 1: Extend the bulk-OUT command protocol**

Add `Flush(oneshot::Sender<std::io::Result<()>>)` to `BulkOutCommand`. Change the OUT worker to own `EndpointWrite<Bulk>` configured with a 4096-byte buffer and multiple pending physical transfers. A submit writes into the buffer and emits success only when `write_all` accepts the request; a flush waits for `writer.flush()` and returns its exact result.

- [ ] **Step 2: Pass matching OUT senders to bulk-IN workers**

Build OUT workers first. When creating IN worker `0x82`, pass the sender for OUT endpoint `0x02` if it exists. Before every physical IN submit, send `Flush` and await its response. If the flush fails or the worker disappears, return a failed USB/IP submit instead of reading stale device state.

- [ ] **Step 3: Preserve cleanup and unlink behavior**

Keep the current import generation filter, bounded channels, unlink suppression, and task abortion paths. Channel closure must make a pending barrier fail rather than hang.

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test buffered_bulk_out_requires_a_flush_before_reading --lib
cargo test stale_bulk_out_completion_is_not_written_after_reimport --lib
```

Expected: PASS.

### Task 3: Verify and deploy

**Files:**
- Modify: `src/usbip_server/mod.rs`

- [ ] **Step 1: Run full verification**

```bash
cargo fmt --check
cargo clippy
cargo test
cargo build --release
```

- [ ] **Step 2: Obtain independent review**

Review the diff for serial ordering, flush-error propagation, worker lifecycle, and whether control/interrupt paths remain unchanged.

- [ ] **Step 3: Deploy both binaries**

Install `target/release/lusbip` to Ubuntu `~/.local/bin/lusbip`; build natively on NanoPi, install `/home/pi/bin/lusbip`, and restart `lusbip.service` only after the active physical flash finishes.

- [ ] **Step 4: Force a physical E2E write**

Attach CH340, then run the existing flash command with `--no-skip` added to the emitted `espflash` invocation. Compare elapsed time and confirm default verification succeeds.
