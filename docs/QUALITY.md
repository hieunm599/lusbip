# Quality And Verification

File này định nghĩa tiêu chí kiểm thử, nghiệm thu, và cách báo cáo cho `lusbip`.

## Lệnh Kiểm Chứng Bắt Buộc

Khi repo đã có code Rust:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Nếu có thay đổi TUI, phải chạy smoke test terminal trên Linux để xác nhận raw mode, alternate screen, cursor, và phím thoát hoạt động đúng.

## Unit Test Bắt Buộc

- Parse `usbip list -r` output thành danh sách remote device.
- Bỏ qua header, dòng trống, dòng separator, và dòng không hợp lệ trong output `usbip`.
- Parse VID/PID hex với input có và không có `0x`.
- Filter USB device theo `bus_id`, VID, PID.
- Validate CLI args thiếu value, port không hợp lệ, VID/PID không hợp lệ.
- Parse `usbip port` output thành danh sách USB/IP port đang attach trên client.
- Chọn đúng USB/IP port cần detach theo remote host, remote bus id, hoặc VID:PID; không detach port không liên quan.
- TUI state transition cho up/down/toggle/chọn/hủy nếu state được tách khỏi render.

Không bắt buộc unit test:

- USB transfer phần cứng thật.
- Nội bộ USB/IP server khi không đổi protocol behavior.
- Terminal pixel-perfect rendering.

## E2E Bắt Buộc Cho V1

E2E v1 dùng 2 máy Linux cùng LAN. Lab mặc định của repo:

- Máy chủ USB/IP: Nano Pi `pi@10.10.61.72`.
- Máy khách USB/IP: Ubuntu `hieunm@10.10.60.208`.
- Thiết bị thật: CP2102 cắm trực tiếp trên Nano Pi.

Không ghi mật khẩu, token, hoặc SSH private key vào repo. Agent phải dùng SSH interactive, SSH key đã cấu hình sẵn, hoặc biến môi trường/cơ chế secret ngoài repo.

### Preflight

Trên Nano Pi:

```bash
ssh pi@10.10.61.72
uname -a
lsusb
```

Kỳ vọng: `lsusb` thấy CP2102. VID:PID thường gặp của CP2102 là `10c4:ea60`; nếu khác, ghi lại VID:PID thực tế trong báo cáo E2E.

Trên Ubuntu:

```bash
ssh hieunm@10.10.60.208
uname -a
usbip version
lusbip doctor --remote 10.10.61.72 --tcp-port 3241 --fix
```

Nếu `usbip` hoặc kernel module `vhci-hcd` chưa có, báo lỗi rõ và ghi lại bước cài đặt cần thiết thay vì claim E2E pass.
Nếu `lusbip doctor` báo `sudo cached` fail, chạy `sudo -v` trong cùng terminal/TTY sẽ chạy `lusbip attach` hoặc `lusbip detach`. Với agent hoặc CI dùng TTY riêng, cache `sudo` từ terminal khác có thể không áp dụng.
Nếu binary đang chạy không nhận `--fix`, kiểm tra `which lusbip`, `lusbip --version`, và cài lại binary mới trước khi tiếp tục E2E.

### Pre-clean Trên Ubuntu Client

Các USB/IP port cũ có thể đang giữ thiết bị từ lần chạy trước. Agent và CLI phải chủ động cleanup trước khi attach lại, nhưng chỉ detach các port xác định chắc chắn liên quan đến target hiện tại.

Trên Ubuntu:

```bash
usbip port
sudo usbip detach -p <PORT>
```

Quy tắc detach:

- Nếu `usbip port` cho thấy port đang attach tới `10.10.61.72`, detach port đó trước khi chạy attach mới.
- Nếu output có bus id hoặc VID:PID trùng CP2102 target, detach port đó.
- Nếu không xác định chắc port nào thuộc target hiện tại, không detach bừa; CLI phải hỏi người dùng hoặc fail rõ với output `usbip port`.
- Nếu `sudo usbip detach -p <PORT>` fail, báo rõ port, command, output lỗi, và output `usbip port` trước/sau.

### Chạy Server Trên Nano Pi

Build native trên Nano Pi để tránh sai kiến trúc CPU:

```bash
cargo build
sudo ./target/debug/lusbip server --host 0.0.0.0 --port 3240
```

Nếu port `3240` đang bị process USB/IP khác giữ trên Nano Pi, dùng port thay thế và truyền cùng port cho client:

```bash
sudo ./target/debug/lusbip server --host 0.0.0.0 --port 3241
```

Server chỉ export USB local và in danh sách thiết bị đang share. Nếu cần giới hạn riêng CP2102 trong lab, thêm filter:

```bash
sudo ./target/debug/lusbip server --vid 10c4 --pid ea60 --host 0.0.0.0 --port 3241
```

### Chạy Client Trên Ubuntu

```bash
usbip port
usbip list -r 10.10.61.72
./target/debug/lusbip attach --remote 10.10.61.72
lsusb
usbip port
```

Nếu server dùng port thay thế:

```bash
usbip --tcp-port 3241 list -r 10.10.61.72
./target/debug/lusbip attach --remote 10.10.61.72 --tcp-port 3241 --bus-id 5-1
```

Để chạy bằng màn client lựa chọn trong CLI:

```bash
./target/debug/lusbip client --remote 10.10.61.72 --tcp-port 3241
```

Trong màn client:

- Mỗi dòng là một cổng USB remote kèm trạng thái `[x]` nếu đang attached hoặc `[ ]` nếu chưa attached.
- Dùng phím mũi tên hoặc `j/k` để di chuyển.
- Nhấn Space trên dòng `[ ]` để attach.
- Nhấn Space trên dòng `[x]` để detach USB/IP port tương ứng.
- Enter không thực hiện attach/detach trong client UI.
- Khi attach/detach đang chạy, UI hiển thị spinner ở đuôi dòng thiết bị đang xử lý, không in log thành công, và giữ nguyên màn hình sau khi command hoàn tất.
- Khi nhấn Esc, client thoát UI và giữ các USB/IP port đang attached chạy nền.
- Khi nhấn Ctrl+C, client phải detach các USB/IP port đang attached trong màn hiện tại rồi mới thoát.

Kết quả chấp nhận:

- Nano Pi thấy CP2102 bằng `lsusb`.
- Ubuntu cleanup được stale USB/IP port liên quan trước khi attach, hoặc xác nhận không có stale port liên quan.
- `usbip list -r 10.10.61.72` trên Ubuntu thấy thiết bị đã share.
- `lusbip attach` attach thành công, không crash.
- `lsusb` trên Ubuntu thấy thiết bị CP2102 tương ứng.
- Chạy attach lặp lại phải idempotent: lần sau tự detach stale port liên quan rồi attach lại thành công.
- Khi dừng server bằng Ctrl+C/Esc, terminal máy chủ trở lại bình thường.

Sau khi test xong:

```bash
usbip port
sudo usbip detach -p <PORT>
```

Chỉ detach port thuộc thiết bị vừa attach trong test.

Nếu E2E fail, báo cáo phải có:

- OS/kernel version của máy chủ và máy khách.
- Version `usbip`.
- Command đã chạy.
- Output lỗi đầy đủ.
- Device VID:PID và bus id.
- Kết quả `dmesg` liên quan nếu có.
- Host nào đang chạy vai trò server/client.
- Trạng thái CP2102 trên Nano Pi trước khi server chạy.
- Output `usbip port` trước cleanup, sau cleanup, và sau attach nếu lỗi liên quan đến port đang bị giữ.

## Mechanical Enforcement

Khi thêm CI, workflow tối thiểu phải chạy:

- `cargo fmt --check`
- `cargo clippy`
- `cargo test`

Workflow release chạy khi push tag dạng `v*`, build và publish GitHub Release cho:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-musl`

Mỗi release phải có gói `.tar.gz` cho từng target và file `SHA256SUMS`.

Khi thêm tài liệu mới hoặc đổi luồng đọc, phải cập nhật `AGENTS.md` để agent sau có bản đồ đúng.

## Quy Tắc Báo Cáo

- Nói rõ đã thay đổi gì.
- Nói rõ command nào đã chạy và kết quả.
- Nói rõ phần nào chưa kiểm chứng được.
- Không nói “hoàn thành v1” nếu chưa qua E2E 2 máy Linux.
