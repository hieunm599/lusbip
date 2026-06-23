# Execution Plan

Kế hoạch này dành cho agent triển khai code sau khi harness đã sẵn sàng.

## Phase 1: Scaffold CLI

- Tạo Cargo project Rust cho binary `lusbip`.
- Thêm dependency tối thiểu: `tokio`, `nusb`, `crossterm`, logging/error handling phù hợp.
- Tạo module `cli`, `commands`, `usb`, `server`, `client`, `tui`, `process`.
- Thêm `list`, `server`, `client`, `attach` vào help.
- Thêm test cho parse VID/PID và validate CLI args.

## Phase 2: USB Listing Và Filter

- Bọc `nusb::list_devices`.
- Chuẩn hóa metadata hiển thị.
- Thêm filter theo bus id, VID, PID.
- Test filter và formatting logic không cần USB thật.

## Phase 3: Server USB/IP Nội Bộ

- Dùng `usbip_server::UsbIpServer::new_from_host_with_filter`.
- Bind mặc định `0.0.0.0:3240`.
- Chế độ không filter export tất cả thiết bị USB local.
- Chế độ filter chỉ export thiết bị match filter và chạy text log.
- Track `bus_id -> client_ip` khi import thành công và release mapping khi client disconnect.
- Đảm bảo cleanup khi Ctrl+C/Esc/lỗi.

## Phase 4: Client Attach

- Bọc process chạy `usbip list -r <host>`.
- Bọc process chạy `usbip port` để phát hiện USB/IP port đang attach trên client.
- Parse output `usbip port` thành danh sách attached port, remote host, bus id, và VID:PID nếu output cung cấp.
- Bọc process chạy `usbip detach -p <PORT>`.
- Parse output thành danh sách remote device.
- Nếu thiếu `--bus-id`, mở client TUI để chọn.
- `lusbip client --remote <IP>` hiển thị tất cả thiết bị remote kèm trạng thái `[x]`/`[ ]`; Space sẽ attach/detach theo trạng thái dòng đang chọn và giữ nguyên màn hình sau khi xử lý. Esc/Ctrl+C detach các port đang attached trong màn hiện tại trước khi thoát.
- Trước `usbip attach`, auto-detach các stale port xác định chắc chắn liên quan đến remote host/bus id/VID:PID hiện tại.
- Nếu stale port không thể xác định an toàn, hỏi người dùng hoặc fail rõ; không detach port không liên quan.
- Chạy `usbip attach -r <host> -b <bus_id>`.
- Message lỗi rõ khi thiếu `usbip`, thiếu sudo, host unreachable, không có device, hoặc detach stale port thất bại.

## Phase 5: Quality Gate

- Chạy `cargo fmt --check`.
- Chạy `cargo clippy`.
- Chạy `cargo test`.
- Chạy terminal smoke test cho TUI trên Linux.
- Chạy E2E mặc định với Nano Pi `pi@10.10.61.72` làm server USB/IP, Ubuntu `hieunm@10.10.60.208` làm client, và CP2102 cắm trên Nano Pi.
- Chạy attach hai lần liên tiếp để xác nhận lần sau tự detach stale port liên quan rồi attach lại thành công.
- Chỉ claim v1 hoàn thành sau E2E 2 máy Linux theo `docs/QUALITY.md`.

## Review Loop

- Agent chính triển khai theo từng phase nhỏ.
- Trước phase server/client hoặc trước khi claim v1 hoàn thành, nên có agent review độc lập kiểm tra:
  - Biên module có rõ không.
  - Test có bao phủ parser/filter/CLI validation không.
  - Cleanup terminal/server có đường lỗi rõ không.
  - E2E evidence có đủ không.
