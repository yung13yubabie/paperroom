# 紙間 Paperroom

紙間是用 Rust 寫成的 Windows 本機筆記。Windows 10 或 11 的 x64 電腦可直接開啟 `Paperroom.exe`。應用程式使用系統內建的原生控制項，SQLite 隨程式一同編譯。預設資料夾是 `%LOCALAPPDATA%\Paperroom`。

- [簡易教學](docs/QUICKSTART.md)
- [Notion、Obsidian、Heptabase 功能對照](docs/FEATURES.md)
- [官方公開文件目錄](docs/PUBLIC-FEATURE-INDEX.tsv)
- [驗證與防禦檢查](docs/VERIFICATION.md)

## 從原始碼建置

開發者可用 Rust 1.100 在 Windows 的 GNU 工具鏈執行 `cargo build --locked --release --target x86_64-pc-windows-gnu`。交叉編譯需安裝相同 Rust 目標和 MinGW-w64，並把 `x86_64-w64-mingw32-gcc` 設為連結器與 SQLite 的 C 編譯器。應用程式以 Windows x64 為發行目標。Linux 可以執行 `cargo test --locked` 驗證資料層。
