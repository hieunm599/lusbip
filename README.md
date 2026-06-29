# lusbip

`lusbip` là CLI Rust để chia sẻ thiết bị USB qua LAN bằng USB/IP trên Linux.

Mô hình chạy:

- **Server**: máy Linux đang cắm USB thật, ví dụ Nano Pi.
- **Client**: máy Linux muốn dùng USB remote, ví dụ Ubuntu dev machine.

Lab mặc định trong repo:

| Vai trò | Máy |
| --- | --- |
| Server | Nano Pi `10.10.61.72` |
| Client | Ubuntu `10.10.60.208` |
| Thiết bị | CP2102, thường là `10c4:ea60` |

## Chạy Nhanh Trong Lab

### Server

Trên máy đang cắm USB:

```bash
lsusb
sudo lusbip server --host 0.0.0.0 --port 3240
```

Nếu chỉ muốn export CP2102:

```bash
sudo lusbip server --vid 10c4 --pid ea60 --host 0.0.0.0 --port 3240
```

Trong màn server:

- `Esc`: đưa server xuống background nếu chưa có client attached.
- `Ctrl+C`: dừng server và release thiết bị.

### Client

Trên máy client:

```bash
lusbip doctor --remote 10.10.61.72 --tcp-port 3240 --fix
usbip port
usbip --tcp-port 3240 list -r 10.10.61.72
lusbip client --remote 10.10.61.72 --tcp-port 3240
```

Trong UI client:

- `Up/Down` hoặc `j/k`: di chuyển.
- `Space`: attach dòng `[ ]`, detach dòng `[x]`.
- `Esc`: thoát UI và giữ USB/IP port đang attached.
- `Ctrl+C`: detach các USB/IP port trong màn hiện tại rồi thoát.

Sau khi attach, kiểm tra:

```bash
lsusb
usbip port
lusbip status --remote 10.10.61.72 --tcp-port 3240
```

## Cài Đặt Và Chuẩn Bị

Yêu cầu chung:

- Linux.
- `lusbip`.
- `usbip` userspace tools.
- Quyền `sudo` cho server/attach/detach.

Cài `lusbip` từ Cargo:

```bash
cargo install lusbip
```

Hoặc build từ source:

```bash
cargo build --release
install -Dm755 target/release/lusbip ~/.local/bin/lusbip
lusbip --version
lusbip doctor --help
```

`lusbip doctor --help` phải có option `--fix`. Nếu không có, máy đang chạy binary cũ trong `PATH`; kiểm tra `which lusbip` rồi cài lại binary mới.

Thông thường chỉ cần để `lusbip` tự chuẩn bị client:

```bash
lusbip doctor --fix
```

Nếu cần làm thủ công trên Ubuntu/Debian client:

```bash
sudo apt update
sudo apt install usbip linux-tools-generic linux-tools-$(uname -r) linux-modules-extra-$(uname -r)
sudo modprobe vhci-hcd
```

Với Nano Pi/ARM64, có thể dùng `cargo install lusbip` nếu máy đủ tài nguyên, hoặc dùng release artifact `aarch64-unknown-linux-musl`.

## Lệnh Thường Dùng

Liệt kê USB local:

```bash
lusbip list
```

Attach trực tiếp nếu đã biết remote bus id:

```bash
sudo -v
lusbip attach --remote 10.10.61.72 --tcp-port 3240 --bus-id 5-1
```

Nếu không truyền `--bus-id`, `lusbip attach` sẽ mở UI chọn thiết bị.

Detach thủ công một USB/IP port:

```bash
lusbip detach --port 00
```

Kiểm tra và tự chuẩn bị môi trường client:

```bash
lusbip doctor --remote 10.10.61.72 --tcp-port 3240 --fix
lusbip status --remote 10.10.61.72 --tcp-port 3240
```

Nếu server dùng port khác, ví dụ `3241`, truyền cùng port cho client:

```bash
lusbip client --remote 10.10.61.72 --tcp-port 3241
```

## Lưu Ý Khi Attach

Trước khi attach, `lusbip` sẽ tự detach stale USB/IP port liên quan tới remote/bus id/VID:PID hiện tại.

CLI không detach bừa các port không xác định được là liên quan. Nếu cần cleanup thủ công, xem port trước rồi chỉ detach đúng port cần gỡ:

```bash
usbip port
lusbip detach --port 00
```

## Lỗi Thường Gặp

| Lỗi | Cách xử lý |
| --- | --- |
| `lusbip: command not found` | Kiểm tra `PATH` hoặc cài lại binary. |
| `usbip: command not found` | Cài `usbip` userspace tools. |
| `open vhci_driver (is vhci_hcd loaded?)` | Chạy `sudo modprobe vhci-hcd`; nếu vẫn fail, cài `linux-modules-extra-$(uname -r)`. |
| `sudo cached` fail trong `doctor` | Chạy `sudo -v` trong cùng terminal trước khi attach/detach. |
| Client không thấy remote device | Kiểm tra server, firewall/LAN, port, và `usbip --tcp-port <PORT> list -r <REMOTE>`. |
| Server không thấy thiết bị | Chạy `lsusb` trên server; nếu `lsusb` không thấy thì vấn đề nằm ở USB/cáp/nguồn/kernel. |
| Server permission denied hoặc không claim được interface | Chạy server bằng `sudo`. |
| Port `3240` bị giữ | Đổi port server, ví dụ `3241`, rồi truyền cùng port cho client. |
| `unexpected argument '--fix'` | Binary đang chạy là bản cũ; kiểm tra `which lusbip`, `lusbip --version`, rồi cài lại binary mới. |

## Giới Hạn Hiện Tại

- Chỉ tập trung Linux host/client.
- Chưa hỗ trợ discovery tự động qua mDNS.
- Chưa hỗ trợ Windows/macOS như target chính.
- USB/IP vẫn phụ thuộc driver/kernel của thiết bị thật.
