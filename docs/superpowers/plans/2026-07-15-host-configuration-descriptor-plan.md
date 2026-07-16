# Host Configuration Descriptor Preservation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve original host configuration descriptors for USB/IP imports.

**Architecture:** Store optional raw configuration descriptors on `UsbDevice`. On Linux, populate them from the sysfs `descriptors` file. `GET_DESCRIPTOR(Configuration)` serves the stored bytes first and otherwise uses the existing generated descriptor.

**Tech Stack:** Rust, nusb, Linux sysfs, existing USB/IP server module.

---

### Task 1: Descriptor parsing and selection

**Files:**
- Modify: `src/usbip_server/device.rs`

- [ ] Write a failing unit test for selecting an indexed raw configuration descriptor and truncating it to `wLength`.
- [ ] Run `cargo test configuration_descriptor` and confirm failure because no raw descriptor is stored.
- [ ] Add minimal raw descriptor storage and selection with synthesized fallback.
- [ ] Re-run `cargo test configuration_descriptor` and confirm pass.

### Task 2: Linux host capture

**Files:**
- Modify: `src/usbip_server/mod.rs`
- Test: `src/usbip_server/mod.rs`

- [ ] Write a failing unit test with concatenated device/configuration bytes, asserting the parser retains each complete configuration descriptor.
- [ ] Run the focused test and confirm failure.
- [ ] Add a small parser and Linux sysfs read path; on read/parse failure retain fallback behavior.
- [ ] Re-run the focused test and confirm pass.

### Task 3: Validation and device test

**Files:**
- Modify: `src/usbip_server/device.rs`, `src/usbip_server/mod.rs`

- [ ] Run `cargo fmt --check`, `cargo clippy`, and `cargo test`.
- [ ] Build and deploy the server to NanoPi, restart only `lusbip.service`, then attach `303a:1001` from Ubuntu.
- [ ] Confirm `cdc_acm` binds and `/dev/ttyACM*` is created; list the CP2102/CH340 exports to ensure they remain exportable.
