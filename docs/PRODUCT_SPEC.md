# Product Spec

`lusbip` là CLI Rust giúp chia sẻ và kết nối thiết bị USB qua mạng LAN bằng USB/IP.

## Phạm Vi V1

V1 tập trung vào Linux host/client:

- `lusbip list`: liệt kê thiết bị USB cục bộ.
- `lusbip server`: chạy USB/IP server trên máy có thiết bị USB thật.
- `lusbip client` hoặc `lusbip attach`: liệt kê thiết bị từ máy chủ và attach thiết bị được chọn.
- `lusbip client`: màn client hiển thị mọi cổng USB remote kèm trạng thái `[x]`/`[ ]`, dùng Space để attach/detach cổng đang chọn. `lusbip tui` chỉ là alias tương thích.
- E2E bắt buộc với 2 máy Linux cùng LAN.

## Ngoài Phạm Vi V1

- GUI desktop.
- Windows/macOS support đầy đủ.
- Auto discovery qua mDNS.
- Multi-host management.
- Session persistence.
- Status dashboard nâng cao.
- Command `detach` riêng cho người dùng đã thuộc v1 để chủ động gỡ USB/IP port khi cần. Auto-detach stale USB/IP attachment trên client vẫn là bắt buộc trong workflow `client`/`attach` để E2E chạy lặp lại được.

## CLI Dự Kiến

Tên binary: `lusbip`.

```text
lusbip list
lusbip server [--vid <HEX>] [--pid <HEX>] [--bus-id <BUS_ID>] [--host <IP>] [--port <PORT>]
lusbip client --remote <IP> [--tcp-port <PORT>]
lusbip attach --remote <IP> [--bus-id <BUS_ID>] [--tcp-port <PORT>]
lusbip detach --port <PORT>
lusbip status [--remote <IP>] [--tcp-port <PORT>]
lusbip doctor [--remote <IP>] [--tcp-port <PORT>] [--fix]
lusbip tui [--remote <IP>] [--tcp-port <PORT>]
```

Mặc định:

- `server --host`: `0.0.0.0`
- `server --port`: `3240`
- VID/PID nhận dạng hex, chấp nhận cả `1366` và `0x1366`.
- Nếu chọn default `client --remote 127.0.0.1`, help phải nói rõ để tránh nhầm với E2E LAN.

## Hành Vi Người Dùng

- `list` in bảng thiết bị USB cục bộ: bus id, address, VID:PID, manufacturer, product, serial.
- `server` không filter thì export tất cả thiết bị USB local mà module USB/IP nội bộ thấy được.
- `server` có filter thì chỉ export thiết bị match filter và chạy ở chế độ log text.
- Trong màn server, Esc chuyển server sang process nền; Ctrl+C dừng server và release thiết bị.
- `client`/`attach` có `--bus-id` thì attach trực tiếp.
- `client` không có `--bus-id`; command này query remote device và mở UI để chọn/toggle attach.
- `client --remote <IP>` query remote device và attached ports, hiển thị mỗi cổng USB remote cùng trạng thái `[x]` nếu attached hoặc `[ ]` nếu detached.
- Trong màn client, Space trên cổng `[ ]` sẽ attach; Space trên cổng `[x]` sẽ detach USB/IP port tương ứng. Enter không thực hiện action.
- Khi attach/detach đang xử lý lâu, UI chỉ hiển thị spinner quay ở đuôi dòng thiết bị đang xử lý, và refresh trạng thái sau khi command hoàn tất mà không thoát màn hình.
- Khi nhấn Esc trong client TUI, client thoát màn UI và giữ các USB/IP port đang attached chạy nền.
- Khi nhấn Ctrl+C trong client TUI, client detach các USB/IP port đang attached trong màn hiện tại rồi mới thoát.
- Trước khi attach, `client`/`attach` phải kiểm tra `usbip port` và tự detach stale port liên quan đến remote host/bus id/VID:PID hiện tại.
- Không detach toàn bộ USB/IP port bừa bãi. Nếu không xác định chắc port nào thuộc target hiện tại, CLI phải hỏi người dùng hoặc fail rõ với output `usbip port`.
- `detach --port <PORT>` chạy `sudo usbip detach -p <PORT>` để gỡ thủ công một USB/IP port.
- `status` hiển thị USB/IP port đang attach và thiết bị remote export nếu có `--remote`.
- `doctor` kiểm tra `usbip`, sudo cache, attached ports, và remote export để chuẩn bị E2E.
- `doctor --fix` tự chuẩn bị Linux client khi có thể: cài Linux USB/IP tools/kernel module packages trên Ubuntu/Debian, nạp `vhci-hcd`, và cleanup stale VHCI ports liên quan.

## Lỗi Cần Rõ Ràng

- Thiếu công cụ `usbip`.
- Thiếu quyền root/sudo.
- Không có thiết bị USB.
- Không kết nối được máy chủ.
- Không có thiết bị exportable trên máy chủ.
- VID/PID, port, hoặc bus id không hợp lệ.
- Không detach được stale USB/IP port đang giữ thiết bị.
