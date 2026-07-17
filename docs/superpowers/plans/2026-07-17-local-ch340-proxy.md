# Local CH340 USB/IP Proxy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (\`- [ ]\`) syntax for tracking.

**Goal:** Add \`lusbip attach --accelerate\` so a localhost proxy batches CH340 bulk-OUT traffic before it crosses the LAN.

**Architecture:** \`src/usbip_proxy.rs\` is a local USB/IP server for \`vhci-hcd\` and client for NanoPi. It acknowledges small local CH340 OUT URBs after bounded buffering, then sends larger remote submits. Bulk-IN, EP0 control, buffer limit, idle, and disconnect are flush barriers.

**Tech Stack:** Rust, Tokio TCP/mpsc/oneshot, internal USB/IP protocol module, Linux \`usbip\`.

---

## File Structure

- \`src/usbip_server/usbip_protocol.rs\`: deserialize remote server responses.
- \`src/usbip_proxy.rs\`: batch state and forwarding session.
- \`src/cli.rs\`, \`src/main.rs\`, \`src/client.rs\`: explicit accelerated attach workflow.
- \`tests/cli_parse.rs\`, \`tests/cli_and_usb.rs\`: CLI and target safety tests.

### Task 1: Deserialize USB/IP remote replies

**Files:**
- Modify: \`src/usbip_server/usbip_protocol.rs\`
- Test: \`src/usbip_server/usbip_protocol.rs\`

- [ ] **Step 1: Write failing tests**

\`\`\`rust
#[tokio::test]
async fn reads_import_response_with_exported_device() {
    let device = UsbDevice::new(7);
    let bytes = UsbIpResponse::op_rep_import_success(&device).to_bytes();
    let (mut tx, mut rx) = tokio::io::duplex(1024);
    tx.write_all(&bytes).await.unwrap();
    let response = UsbIpResponse::read_from_socket(&mut rx).await.unwrap();
    assert!(matches!(response, UsbIpResponse::OpRepImport { status: 0, device: Some(_) }));
}

#[tokio::test]
async fn reads_submit_response_payload() {
    let header = UsbIpHeaderBasic { command: USBIP_RET_SUBMIT.into(), seqnum: 9, devid: 0, direction: 0, ep: 0 };
    let bytes = UsbIpResponse::usbip_ret_submit_success(&header, 0, 3, vec![1, 2, 3], vec![]).to_bytes();
    let (mut tx, mut rx) = tokio::io::duplex(1024);
    tx.write_all(&bytes).await.unwrap();
    let response = UsbIpResponse::read_from_socket(&mut rx).await.unwrap();
    assert!(matches!(response, UsbIpResponse::UsbIpRetSubmit { actual_length: 3, transfer_buffer, .. } if transfer_buffer == vec![1, 2, 3]));
}
\`\`\`

- [ ] **Step 2: Verify RED**

Run: \`cargo test reads_import_response_with_exported_device reads_submit_response_payload --lib\`

Expected: compiler error because \`UsbIpResponse::read_from_socket\` is missing.

- [ ] **Step 3: Implement the parser**

Add \`pub async fn read_from_socket<T: AsyncReadExt + Unpin>(socket: &mut T) -> Result<Self>\`. Decode \`OP_REP_IMPORT\`, \`USBIP_RET_SUBMIT\`, and \`USBIP_RET_UNLINK\` with big-endian fields. Read a payload only for an IN submit response; unknown commands return \`InvalidData\`.

- [ ] **Step 4: Verify GREEN and commit**

Run: \`cargo test reads_import_response_with_exported_device reads_submit_response_payload --lib\`

Expected: PASS.

\`\`\`bash
git add src/usbip_server/usbip_protocol.rs
git commit -m "feat: parse USBIP server responses"
\`\`\`

### Task 2: Create pure CH340 batching state

**Files:**
- Create: \`src/usbip_proxy.rs\`
- Modify: \`src/lib.rs\`
- Test: \`src/usbip_proxy.rs\`

- [ ] **Step 1: Write failing tests**

\`\`\`rust
#[test]
fn ch340_bulk_out_is_eligible_only_for_endpoint_02() {
    assert!(is_accelerated_ch340_bulk_out(0x1a86, 0x7523, 0x02, 0));
    assert!(!is_accelerated_ch340_bulk_out(0x1a86, 0x7523, 0x82, 1));
    assert!(!is_accelerated_ch340_bulk_out(0x10c4, 0xea60, 0x02, 0));
}

#[test]
fn batch_flushes_at_four_kib_and_preserves_order() {
    let mut batch = Ch340OutBatch::new();
    batch.push(vec![1; 2048]).unwrap();
    batch.push(vec![2; 2048]).unwrap();
    assert_eq!(batch.take_if_full(), Some([vec![1; 2048], vec![2; 2048]].concat()));
}

#[test]
fn barrier_returns_all_pending_bytes() {
    let mut batch = Ch340OutBatch::new();
    batch.push(vec![7, 8]).unwrap();
    assert_eq!(batch.flush(), vec![7, 8]);
    assert!(batch.flush().is_empty());
}
\`\`\`

- [ ] **Step 2: Verify RED**

Run: \`cargo test ch340_bulk_out_is_eligible_only_for_endpoint_02 batch_flushes_at_four_kib --lib\`

Expected: unresolved module/types.

- [ ] **Step 3: Implement the pure model**

Define \`pub const CH340_BATCH_LIMIT: usize = 4096\`, \`is_accelerated_ch340_bulk_out\`, and \`Ch340OutBatch { bytes: Vec<u8> }\`. Limit batching to VID:PID \`1a86:7523\`, endpoint \`0x02\`, OUT direction. \`flush\` uses \`std::mem::take\`.

- [ ] **Step 4: Verify GREEN and commit**

Run: \`cargo test ch340_bulk_out --lib\`

Expected: PASS.

\`\`\`bash
git add src/lib.rs src/usbip_proxy.rs
git commit -m "feat: model CH340 proxy batching"
\`\`\`

### Task 3: Implement local-server/remote-client forwarding

**Files:**
- Modify: \`src/usbip_proxy.rs\`
- Test: \`src/usbip_proxy.rs\`

- [ ] **Step 1: Write failing ordering tests**

\`\`\`rust
#[test]
fn control_request_requires_out_flush() {
    let mut state = ProxyOrderState::default();
    state.accept_batched_out(vec![1, 2]);
    assert_eq!(state.before_forward(ProxyRequestKind::Control), ProxyAction::FlushThenForward);
}

#[test]
fn bulk_in_requires_out_flush() {
    let mut state = ProxyOrderState::default();
    state.accept_batched_out(vec![1]);
    assert_eq!(state.before_forward(ProxyRequestKind::BulkIn), ProxyAction::FlushThenForward);
}

#[test]
fn failed_remote_flush_poisoned_session() {
    let mut state = ProxyOrderState::default();
    state.record_remote_flush_failure("broken pipe");
    assert!(state.check_healthy().is_err());
}
\`\`\`

- [ ] **Step 2: Verify RED**

Run: \`cargo test control_request_requires_out_flush bulk_in_requires_out_flush --lib\`

Expected: unresolved proxy ordering types.

- [ ] **Step 3: Implement forwarding**

Implement \`run_proxy(remote: SocketAddr, bus_id: String, listener: TcpListener)\`:

1. Connect to NanoPi, send \`OpReqImport\`, parse \`OpRepImport\`, reject a nonzero status or absent device.
2. Accept one localhost connection; respond to \`OpReqDevlist\` and a matching \`OpReqImport\` with the imported device.
3. Acknowledge eligible local OUT chunks immediately after they enter \`Ch340OutBatch\`; when full, send one larger remote \`UsbIpCmdSubmit\` and await its return.
4. On local bulk-IN or EP0 control, flush the batch first, then forward the original command and replace the remote response sequence number with the local sequence number.
5. Forward unknown, interrupt, and non-CH340 requests one-for-one. A remote failure poisons the session and returns failed local submits.
6. Use a 2 ms idle timer only for a local batch. Loopback acknowledgements let ESP flash chunks fill the batch before the timer; EOF drops the remote import connection.

- [ ] **Step 4: Verify GREEN and commit**

Run: \`cargo test proxy --lib\`

Expected: batch, ordering, and poisoned-session tests pass.

\`\`\`bash
git add src/usbip_proxy.rs
git commit -m "feat: proxy CH340 USBIP bulk writes locally"
\`\`\`

### Task 4: Wire explicit accelerated attach

**Files:**
- Modify: \`src/cli.rs\`
- Modify: \`src/main.rs\`
- Modify: \`src/client.rs\`
- Test: \`tests/cli_parse.rs\`
- Test: \`tests/cli_and_usb.rs\`

- [ ] **Step 1: Write failing CLI and safety tests**

\`\`\`rust
#[test]
fn parses_accelerated_attach() {
    let cli = Cli::parse_from(["lusbip", "attach", "--remote", "10.10.61.72", "--bus-id", "5-1", "--accelerate"]);
    match cli.command {
        Commands::Attach(args) => assert!(args.accelerate),
        _ => panic!("attach command expected"),
    }
}

#[test]
fn accelerated_attach_rejects_non_ch340_target() {
    let device = RemoteUsbDevice { bus_id: "7-1".into(), description: "Silicon Labs : CP210x UART Bridge (10c4:ea60)".into() };
    assert!(accelerated_target(&device).is_err());
}
\`\`\`

- [ ] **Step 2: Verify RED**

Run: \`cargo test parses_accelerated_attach --test cli_parse\`

Expected: \`AttachArgs\` has no \`accelerate\` field.

- [ ] **Step 3: Implement lifecycle**

Add \`accelerate: bool\` to \`AttachArgs\`; pass it through \`main.rs\` into \`run_attach\`. Accelerated mode requires a bus id, queries metadata, rejects anything except \`1a86:7523\`, binds a deterministic localhost port, spawns \`run_proxy\`, auto-detaches only the known target, then invokes the existing attach runner with \`127.0.0.1\`, proxy port, and original bus id. Direct attach is unchanged.

- [ ] **Step 4: Verify GREEN and commit**

Run:

\`\`\`bash
cargo test parses_accelerated_attach --test cli_parse
cargo test accelerated_attach_rejects_non_ch340_target --test cli_and_usb
\`\`\`

Expected: PASS.

\`\`\`bash
git add src/cli.rs src/main.rs src/client.rs tests/cli_parse.rs tests/cli_and_usb.rs
git commit -m "feat: add accelerated CH340 attach mode"
\`\`\`

### Task 5: Review, deploy, and measure

**Files:**
- Modify: \`docs/superpowers/specs/2026-07-17-local-ch340-proxy-design.md\` only if an invariant changes.

- [ ] **Step 1: Mechanical verification**

\`\`\`bash
cargo fmt --check
cargo clippy
cargo test
cargo build --release
git diff --check
\`\`\`

Expected: every command exits zero.

- [ ] **Step 2: Independent review**

Review \`src/usbip_proxy.rs\` for request/response sequence mapping, batch ownership, EP0 and IN barriers, failure propagation, and cleanup.

- [ ] **Step 3: Deploy**

Install the Ubuntu binary at \`~/.local/bin/lusbip\`. Build natively on NanoPi with \`source ~/.cargo/env && cargo build --release\`; install \`/home/pi/bin/lusbip\`; restart only \`lusbip.service\`. Do not detach unrelated VHCI ports.

- [ ] **Step 4: E2E monitor**

\`\`\`bash
sudo -v
~/.local/bin/lusbip attach --accelerate --remote 10.10.61.72 --bus-id 5-1
ls -l /dev/ttyUSB0
cd /home/hieunm/workspace/poc/p4mipi-demo
make monitor
\`\`\`

Expected: monitor prints an ESP boot log.

- [ ] **Step 5: Forced benchmark**

Run the existing \`espflash flash\` command with \`--no-skip\`, bootloader, partition table, and \`--baud 921600\`. Require all 84 app blocks to verify and record elapsed time against the direct baseline of 8m46s.

- [ ] **Step 6: Cleanup**

Detach only the identified proxy attachment using \`lusbip detach --port <PORT>\`. Confirm NanoPi releases bus \`5-1\`.
