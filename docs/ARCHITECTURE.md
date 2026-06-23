# Architecture

Kiến trúc ưu tiên module nhỏ, biên rõ ràng, và dễ test. Side effect như đọc USB thật, chạy process, raw terminal, và TCP server phải nằm sau wrapper để logic thuần có thể kiểm thử được.

## Module Đề Xuất

- `cli`: parse arguments, in help/version, chuyển execution sang command handler.
- `commands`: điều phối command `list`, `server`, `client`/`attach`, và `detach` nếu có.
- `usb`: bọc API `nusb`, tạo metadata hiển thị, filter theo bus id, VID, PID.
- `server`: dùng module USB/IP nội bộ, tạo `UsbIpServer::new_from_host_with_filter`, bind `0.0.0.0:3240`, cleanup khi thoát.
- `client`: chạy `usbip list -r <host>`, parse output, chọn remote device, kiểm tra `usbip port`, detach stale port liên quan, rồi chạy `usbip attach -r <host> -b <bus_id>`.
- `tui`: quản lý alternate screen, raw mode, render danh sách, và state chọn/toggle.
- `process`: bọc `std::process::Command` cho `usbip list`, `usbip port`, `usbip detach`, và `usbip attach` để command invocation có thể test hoặc mock.

## Dependency Direction

- `cli` phụ thuộc `commands`.
- `commands` phụ thuộc `usb`, `server`, `client`, `tui`.
- `server` phụ thuộc module `usbip_server` nội bộ và `usb`.
- `client` phụ thuộc `process` và parser output cho remote device plus attached port.
- `tui` không gọi trực tiếp `usbip_server` hoặc `usbip`; TUI chỉ nhận state và trả action.

## Auto-Detach Trên Client

Trước khi attach, client phải xử lý stale USB/IP port đang giữ thiết bị từ lần chạy trước:

- Parse `usbip port` thành attached port state.
- Match port cần detach theo remote host, bus id, hoặc VID:PID của target hiện tại.
- Chạy `usbip detach -p <PORT>` cho các port match chắc chắn.
- Không detach port không liên quan. Nếu match không chắc chắn, trả lỗi rõ hoặc yêu cầu người dùng xác nhận.
- Parser `usbip port` phải là logic thuần để unit test được bằng fixture output.

## TUI Tiêu Chuẩn

Server:

- Không có màn chọn attach/detach ở server.
- `server` mặc định export tất cả thiết bị USB local mà module USB/IP nội bộ thấy được.
- Filter `--vid`, `--pid`, `--bus-id` chỉ dùng để giới hạn danh sách export khi cần.
- Esc chuyển server sang process nền; Ctrl+C dừng server, cleanup và release device.

Client TUI:

- Hiển thị remote host và tcp port.
- Hiển thị tất cả thiết bị exportable từ `usbip list -r`.
- Ghép với output `usbip port` để hiển thị trạng thái `[x]` nếu attached hoặc `[ ]` nếu detached trên từng dòng.
- Space trên dòng `[ ]` sẽ chạy attach cho bus id tương ứng.
- Space trên dòng `[x]` sẽ chạy detach USB/IP port tương ứng. Enter không thực hiện action trong client TUI.
- Trong khi chạy attach/detach blocking, redraw TUI định kỳ với spinner ở đuôi dòng thiết bị đang xử lý; sau khi command hoàn tất, reload danh sách và giữ nguyên màn hình.
- Khi nhận Esc, client TUI restore terminal và thoát UI nhưng giữ USB/IP port đang attached.
- Khi nhận Ctrl+C, client TUI chạy cleanup detach cho các port đang attached trong danh sách hiện tại, sau đó restore terminal và thoát.

Yêu cầu terminal:

- Luôn restore raw mode, cursor, alternate screen trong mọi đường thoát.
- Nếu render hoặc event loop lỗi, app trả message rõ và không để terminal kẹt raw mode.
- Text phải chịu được terminal nhỏ; description dài cần được cắt ngắn.

## Nguồn Tham Khảo Cục Bộ

- Module `src/usbip_server` là phần USB/IP server nội bộ của `lusbip`.
- Dùng `/home/hieunm/workspace/poc/remote-serial-hub/cli/src/usbip.rs` để tham khảo TUI và luồng USB/IP.
- Không copy nguyên khối lớn; chỉ tái dùng ý tưởng và API phù hợp.
