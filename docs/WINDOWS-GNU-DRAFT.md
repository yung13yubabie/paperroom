# 紙間三來源整合的 Windows GNU 草稿

此 ZIP 是未簽署的 Windows x64 草稿。實際來源提交、EXE SHA-256、兩次建置的校驗以及原始依賴通知，請讀封裝中的建置清冊和證據檔。既有 v0.1.2 預覽另有固定來源與工件。本檔不代表 Windows 10 或 Windows 11 乾淨客戶端已驗收，也不授予專案的正式發行權利。

這個候選可透過紙間既有 SQLite 工作區，使用所選 Notion 頁面自訂文字交接包、Heptabase Markdown 與相容標準 JSON Canvas、以及部分 Obsidian Bases 表格或清單。可用範圍見 README。原廠功能和匯出格式分別驗證。第三方登入不屬於 EXE 的執行依賴。

## 逐項依賴證據與用戶端收據

新版封裝的 `evidence/gnu-dependency-redistribution.json` 綁定來源提交和 EXE。它逐項列出 Cargo 目標與建置角色、Cargo.lock 記下的 registry 封存雜湊、套件宣告授權、包內通知原文的路徑與雜湊，以及可從第一份最終 GNU link map 辨認的非空 Rust 封存物件。建置器比對 Cargo `compiler-artifact` 訊息的完整輸出路徑與套件 ID，涵蓋舊 `release/deps` 及 Cargo 1.100 的 `release/build/<package>/<hash>/out`。未能唯一比對的物件列在未歸屬清單。原始絕對路徑及 Cargo 訊息留在建置端，封裝只記錄封存檔與物件名稱。沒有觀察到的項目只代表本次 map 未歸屬，可能受連結最佳化或巨集展開影響。MinGW、GCC 與 Rust 執行階段另見 GNU runtime 成員收據。Windows OS DLL 只列直接匯入。

`alloc-stdlib 0.2.4` 的 crate 封存沒有可收錄的授權檔名。封裝另附官方固定上游版本的原文與對應說明，並以雜湊核對。其他缺件列為未解，不能把開放原始碼宣告當成原始通知。這份清冊提供建置和通知證據。發行者仍要以實際最終產物審查適用條款與未辨識元件。

沿用相同 build 腳本產出新包後，可用 `package-windows-gnu-audit.py verify --require-dependency-evidence` 與其既有必要參數重新核對下載檔。舊包仍可用一般 `verify` 查驗其原有證據。若要做真正的用戶端斷網測試，使用 [Windows GNU 候選的斷網驗收](../qa/windows-client-offline/GNU-CANDIDATE.md)。Windows Server CI 不填入用戶端結果欄。

GNU 封裝會在建置及 ZIP 讀回時檢查 EXE 大小，兩處均以 24 MiB 為上限。ZIP 壓縮大小及所有解壓成員合計上限仍是 48 MiB 和 64 MiB。

## GCC／MinGW 執行階段與再散布證據

以下核對 #30 草稿 `2b442a77a9e4bad034832c07c1eba72cd42167f7` 的實際下載物。EXE SHA-256 為 `a101eb041173c80b1a360095e116dc1c83bec462d952c323aa8b8c5d445c1787`，GNU ZIP SHA-256 為 `60a2a0060d7083247e4fc6b149afbd1185d725a55bc4a5910aa1734135fb74cd`。其他候選以自己的封裝收據核對。

- **GCC。** 非空最終連結成員為 `crtbegin.o`、`crtend.o` 與 `libgcc_eh.a(unwind-seh.o)`。三筆連結記錄所指檔案的雜湊均與固定 Debian `gcc-mingw-w64-x86-64-posix` 14.2.0-19+27+b1 內原件相符。該包的控制資料指向 `Built-Using: gcc-14 (= 14.2.0-19)`。上游的兩個啟動物件和 SEH 來源皆標明 GPL-3.0-or-later 搭配 GCC Runtime Library Exception 3.1。 [GCC 14.2.0 啟動物件規則](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/libgcc/config/i386/t-cygming) · [crtbegin](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/libgcc/config/i386/cygming-crtbegin.c) · [crtend](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/libgcc/config/i386/cygming-crtend.c) · [SEH](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/libgcc/unwind-seh.c) · [Debian GCC 14.2.0-19 版權](https://metadata.ftp-master.debian.org/changelogs/main/g/gcc-14/gcc-14_14.2.0-19_copyright)
- **實際隨附的文字。** ZIP 另附完整 GPLv3 與 GCC Runtime Library Exception 3.1。對照 GCC 14.2.0 的原始檔，差異限於排版、版權符號及連結表示。GCC cross package 隨附的 Debian copyright 確實重印舊的 GCC 9 說明。GCC 14.2.0-19 的 Debian 來源歸屬以本節連結補足。本輪將包內兩份授權文字更新為同標籤的 `COPYING3` 與 `COPYING.RUNTIME` 原件。 [GCC 14.2.0 GPLv3 原檔](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/COPYING3) · [GCC 14.2.0 exception 原檔](https://github.com/gcc-mirror/gcc/blob/releases/gcc-14.2.0/COPYING.RUNTIME)
- **MinGW 與 Rust。** MinGW-w64 12.0.0-5 的啟動物件、靜態成員及 DLL 匯入封存，另對應該版 Debian copyright。Rust 1.100.0 的 14 個 sysroot 封存輸入及 `COPYRIGHT-library.html` 也逐一比對實際工具鏈。本包的 PE 有 `msvcrt.dll` 等 Windows 系統匯入。ZIP 只有一個 EXE，沒有 Windows DLL 或工具鏈封存檔。

GCC 的額外許可適用於有原件聲明的執行階段，條件包含與獨立模組組合、且所有目標碼均來自符合定義的編譯流程。此配方以 Rust／LLVM 和固定 GNU 工具鏈產生並連結目標碼，關閉跨 crate LTO，未設定 GCC 中間碼外部最佳化。發行者仍需依所採配方、實際產物與各模組原條款判定適用範圍，並決定 Paperroom 自身的授權及通知提供方式。現有文件和清冊提供可追溯證據，正式發行仍需由權利人審查。 [GCC 例外條件](https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html) · [Cargo LTO 設定](https://doc.rust-lang.org/cargo/reference/profiles.html#lto)
