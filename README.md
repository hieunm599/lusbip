# lusbip

`lusbip` là CLI Rust giúp chia sẻ và kết nối thiết bị USB qua mạng LAN bằng USB/IP trên Linux.

Mục tiêu hiện tại:

- Máy server quảng bá/export các thiết bị USB cục bộ qua USB/IP.
- Máy client hiển thị danh sách cổng USB remote trong terminal UI.
- Client dùng phím di chuyển và `Space` để attach/detach.
- Server và client tự refresh khi cắm/rút thiết bị USB.

## Yêu Cầu

Trên cả server và client:

- Linux.
- `usbip` userspace tools.
- Quyền `sudo` cho thao tác attach/detach.

Trên client cần kernel module VHCI:

```bash
sudo modprobe vhci-hcd
```

Trên Ubuntu/Debian, gói thường cần cài là:

```bash
sudo apt update
sudo apt install linux-tools-generic linux-tools-$(uname -r)
```

## Cài Đặt

Cài từ Cargo:

```bash
cargo install lusbip
```

Hoặc build từ source:

```bash
cargo build --release
install -Dm755 target/release/lusbip ~/.local/bin/lusbip
```

### Cài Trên Nano Pi

Nano Pi thường chạy ARM64/aarch64 và có vài điểm khác Ubuntu desktop.

Kiểm tra kiến trúc trước:

```bash
uname -m
```

Nếu kết quả là `aarch64`, có thể dùng một trong hai cách dưới.

#### Cách 1: Cài bằng Cargo trên Nano Pi

Cài Rust/Cargo nếu máy chưa có:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

Cài `lusbip`:

```bash
cargo install lusbip
```

Đảm bảo binary nằm trong PATH:

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.profile
. ~/.profile
lusbip --version
```

Nếu `cargo install` chậm hoặc hết RAM trên Nano Pi, dùng binary release ở cách 2.

#### Cách 2: Dùng Binary Release Cho ARM64

Tải artifact `lusbip-aarch64-unknown-linux-musl-<version>.tar.gz` từ GitHub Release, rồi cài vào `~/bin`:

```bash
mkdir -p ~/bin
tar -xzf lusbip-aarch64-unknown-linux-musl-<version>.tar.gz
install -m 755 lusbip-aarch64-unknown-linux-musl-<version>/lusbip ~/bin/lusbip
echo 'export PATH="$HOME/bin:$PATH"' >> ~/.profile
. ~/.profile
lusbip --version
```

#### Chuẩn Bị USB/IP Trên Nano Pi

Nano Pi chạy vai trò server nên cần thấy thiết bị USB local và có quyền truy cập USB:

```bash
lsusb
which usbip || true
```

Nếu thiếu `usbip`, cài gói tương ứng với distro đang chạy. Trên Debian/Ubuntu/Armbian thường thử:

```bash
sudo apt update
sudo apt install usbip linux-tools-generic linux-tools-$(uname -r)
```

Một số image ARM không có đủ `linux-tools-$(uname -r)` trong apt repo. Khi đó cần cài gói `usbip` riêng nếu distro cung cấp, hoặc dùng package kernel/tools đúng với image của board.

Chạy server:

```bash
sudo ~/bin/lusbip server --host 0.0.0.0 --port 3240
```

Server vẫn mở được khi chưa cắm CP2102. Khi cắm CP2102 sau, danh sách sẽ tự cập nhật trong khoảng 1 giây.

Các lỗi thường gặp trên Nano Pi:

- `lusbip: command not found`: `~/bin` hoặc `~/.cargo/bin` chưa nằm trong `PATH`.
- `usbip: command not found`: thiếu USB/IP userspace tools.
- Không thấy CP2102 trong server: kiểm tra `lsusb` trước; nếu `lsusb` không thấy thì lỗi nằm ở cổng USB/nguồn/cáp/kernel, không phải `lusbip`.
- `Permission denied` hoặc không claim được interface: chạy server bằng `sudo`.
- Port `3240` bị giữ: đổi port, ví dụ `--port 3241`, và client cũng dùng `--tcp-port 3241`.

## Chạy Server

Trên máy đang cắm thiết bị USB, ví dụ Nano Pi:

```bash
sudo lusbip server --host 0.0.0.0 --port 3240
```

Server vẫn mở kể cả khi chưa có thiết bị USB nào được cắm. Khi cắm thiết bị sau, danh sách export sẽ tự cập nhật.

Giới hạn theo VID/PID nếu cần:

```bash
sudo lusbip server --vid 10c4 --pid ea60 --host 0.0.0.0 --port 3240
```

## Chạy Client UI

Trên máy client:

```bash
sudo -v
lusbip client --remote 10.10.61.72 --tcp-port 3240
```

Phím trong UI:

- `↑/↓` hoặc `j/k`: di chuyển.
- `Space`: attach/detach thiết bị đang chọn.
- `Esc` hoặc `Ctrl+C`: detach các port đang attached trong màn hiện tại rồi thoát.

Trạng thái:

- `[ ]`: thiết bị remote chưa attach.
- `[x]`: thiết bị remote đang attached trên client.
- Spinner cuối dòng: attach/detach đang xử lý.

## Lệnh Hữu Ích

Liệt kê USB local:

```bash
lusbip list
```

Attach trực tiếp một bus id:

```bash
sudo -v
lusbip attach --remote 10.10.61.72 --tcp-port 3240 --bus-id 5-1
```

Detach thủ công một USB/IP port:

```bash
lusbip detach --port 00
```

Xem trạng thái:

```bash
lusbip status --remote 10.10.61.72 --tcp-port 3240
```

Kiểm tra môi trường:

```bash
lusbip doctor --remote 10.10.61.72 --tcp-port 3240
```

## Lab Mặc Định

Lab đang dùng trong repo:

- Server: Nano Pi `10.10.61.72`.
- Client: Ubuntu `10.10.60.208`.
- Thiết bị: CP2102, thường là `10c4:ea60`.

Server:

```bash
sudo ~/bin/lusbip server --host 0.0.0.0 --port 3240
```

Client:

```bash
sudo -v
~/.local/bin/lusbip client --remote 10.10.61.72 --tcp-port 3240
```

## Release

GitHub Actions sẽ build release khi push tag dạng `v*`:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Artifacts hiện tại:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-musl`

## Giới Hạn Hiện Tại

- Tập trung Linux host/client.
- Chưa hỗ trợ discovery tự động qua mDNS.
- Chưa hỗ trợ Windows/macOS như target chính.
- USB/IP vẫn phụ thuộc khả năng kernel/driver của thiết bị thật.
