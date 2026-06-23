# LUSBIP Harness

Tài liệu này là điểm vào cấp cao cho harness của `lusbip`: một CLI Rust hỗ trợ chia sẻ và kết nối thiết bị USB qua IP. Harness này được viết theo hướng agent-first: repo là nguồn sự thật, `AGENTS.md` là bản đồ ngắn, còn chi tiết triển khai nằm trong `docs/`.

Đọc theo thứ tự:

1. `AGENTS.md` để nắm quy tắc làm việc của agent trong repo.
2. `docs/PRODUCT_SPEC.md` để hiểu mục tiêu sản phẩm và phạm vi v1.
3. `docs/ARCHITECTURE.md` để hiểu biên module và luồng dữ liệu.
4. `docs/EXECUTION_PLAN.md` để biết thứ tự triển khai.
5. `docs/QUALITY.md` để biết tiêu chí kiểm thử và nghiệm thu.

## Mục Tiêu V1

`lusbip` v1 tập trung vào đường chạy Linux host/client:

- Máy chủ Linux liệt kê thiết bị USB cục bộ.
- Máy chủ Linux chạy USB/IP server nội bộ của `lusbip` và export các cổng USB local.
- Máy khách Linux hiển thị các cổng USB remote kèm trạng thái attached/detached bằng giao diện terminal kiểu zellij.
- Máy khách Linux dùng Space để attach hoặc detach cổng USB đang chọn; Enter không thực hiện action trong màn client.
- V1 chỉ được coi là hoàn thành khi đã qua E2E với 2 máy Linux cùng LAN.

## Nguồn Tham Khảo Bắt Buộc

- `/home/hieunm/workspace/poc/nusbip`: nguồn tham khảo ban đầu cho phần USB/IP server. `lusbip` không còn phụ thuộc path dependency vào repo này.
- `/home/hieunm/workspace/poc/remote-serial-hub`: tham khảo CLI layout, TUI `crossterm`, và luồng USB/IP hiện có.
- `https://openai.com/index/harness-engineering/`: tham khảo tinh thần harness engineering: repo-local docs, agent-legible context, execution plan, mechanical checks, và review loop.

Repo `git@github.com:hoangnb24/repository-harness.git` là tham khảo không bắt buộc trong giai đoạn này. Không chặn tiến độ nếu không truy cập được repo đó.

## Nguyên Tắc Agent

- Không xem `HARNESS.md` là nơi chứa mọi chi tiết. Khi cập nhật yêu cầu, hãy cập nhật file chi tiết trong `docs/` và chỉnh link trong `AGENTS.md` nếu cần.
- Không sửa repo tham khảo.
- Không claim “hoàn thành v1” nếu chưa có E2E 2 máy Linux theo `docs/QUALITY.md`.
- Nếu có code Rust, các lệnh tối thiểu trước khi báo kết quả là `cargo fmt --check`, `cargo clippy`, `cargo test`.
- Số agent mặc định cho giai đoạn triển khai hiện tại là 1 agent chính. Với thay đổi lớn, nên có thêm 1 agent review độc lập cho kiến trúc/test trước khi claim hoàn thành.
