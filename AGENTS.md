# Quy Tắc Cho Agent

File này là bản đồ ngắn cho agent làm việc trong repo `lusbip`. Không nhồi chi tiết dài vào đây; chi tiết nằm trong `docs/`.

## Luồng Đọc Bắt Buộc

1. Đọc `HARNESS.md`.
2. Đọc `docs/PRODUCT_SPEC.md` để hiểu mục tiêu và phạm vi.
3. Đọc `docs/ARCHITECTURE.md` trước khi tạo hoặc sửa module.
4. Đọc `docs/EXECUTION_PLAN.md` trước khi triển khai.
5. Đọc `docs/QUALITY.md` trước khi báo kết quả.

## Quy Tắc Triển Khai

- Ưu tiên Rust, module nhỏ, biên rõ ràng.
- Tách logic thuần khỏi side effect để dễ test.
- `main.rs` chỉ nên parse CLI và gọi command handler.
- Không sửa `/home/hieunm/workspace/poc/nusbip` hoặc `/home/hieunm/workspace/poc/remote-serial-hub`.
- Không thêm dependency nặng nếu `tokio`, `nusb`, `crossterm` và module USB/IP nội bộ đã đủ.
- Nếu thay đổi tài liệu điều hướng, cập nhật file này để agent sau không đọc nhầm.
- Không commit mật khẩu, token, SSH private key, hoặc credential nào khác. Thông tin đăng nhập runtime phải nằm ngoài repo.
- Không bắt người dùng tự cleanup USB/IP port cũ nếu workflow có thể xác định và detach tự động. Chỉ detach port liên quan đến target hiện tại; không detach bừa toàn bộ `usbip port`.

## Kiểm Chứng Bắt Buộc

Khi repo đã có code Rust:

```bash
cargo fmt --check
cargo clippy
cargo test
```

Khi claim v1 hoàn thành, phải có E2E 2 máy Linux cùng LAN theo `docs/QUALITY.md`. Lab mặc định: Nano Pi `pi@10.10.61.72` làm server với CP2102 cắm trực tiếp, Ubuntu `hieunm@10.10.60.208` làm client.

## Agent Review

- Mặc định: 1 agent chính triển khai.
- Với thay đổi lớn hoặc trước khi claim v1 hoàn thành: dùng thêm 1 agent review độc lập để kiểm tra kiến trúc, test, và tiêu chí nghiệm thu.
