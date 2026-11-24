# 防禦性檢查與公開資料來源

本頁記錄紙間的防禦性檢查線索、程式碼中可見的處理和測試界限。索引用來發想情境，並非漏洞證據；本頁不代表完整安全稽核或所有 Windows 環境的驗證。

## 線索來源

- [AwesomeAIAgentUseCases](https://github.com/NageshMashette/AwesomeAIAgentUseCases) 可公開存取，列有文件審閱、威脅偵測和品質監測等應用方向；其範例不是紙間的實作，也不能證明紙間具備代理或安全功能。本次將這些方向轉為匯入資料、資料完整性及錯誤處理的人工檢查。
- [awesome-rust](https://github.com/rust-unofficial/awesome-rust) 是 Rust 函式庫與資源索引；本次用它辨識資料庫、檔案及測試相關領域，具體行為再查 Rust、SQLite 和 Microsoft 原始文件。沒有安裝上述專案或執行其程式。

## 情境與證據

狀態說明：「已修正」表示找到具體風險並在程式碼中加入處理，不等同所有環境實測；「未觀察」是已檢查範圍內的結果；「待驗證」表示仍缺獨立動態證據。

| 情境 | 狀態與具體證據 | 剩餘界限 |
| --- | --- | --- |
| ZIP 路徑、解壓容量及重複資料 | **已有防護**：`archive.rs` 限制項目數、深度和大小，拒絕不安全路徑及符號連結；不將 ZIP 任意展開到檔案系統。Wine 已測 Notion 父頁與子頁匯入。 | 非全面的壓縮格式安全驗證；異常輸入測試及依賴維護仍需持續。 |
| 輸出覆寫、作業中斷 | **已修正**：獨立 `.partial`、`create_new`、同步後再發佈；目的地存在時拒絕覆寫。 [Rust 原子建立](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new)、[Windows MoveFile](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefile)。 | 強制結束可留下暫存目錄；磁碟故障及所有檔案系統未完整驗證。 |
| 長路徑與 Unicode | **已修正並於 Wine 測試**：輸出父目錄採擴展絕對路徑；已針對 Wine 回傳的 Win32 錯誤 234 加入限定後備處理。Windows 測試程式在 Wine 10 UTF-8 環境通過 16 項，含 12 層長 Unicode 資料夾、Markdown 匯出、SQLite 備份與重開，以及控制項和工作區路徑檢查。 | Wine 為相容環境；原生 Windows 的長路徑、特殊檔名及標準帳戶仍待驗證。 |
| 版本遷移與新格式 | **已修正**：開啟時在切換 WAL 前檢查資料庫版本；不支援的新版本拒絕開啟。 | 不代表所有歷史版本或故障遷移都已實測。 |
| 多視窗同時修改 | **已修正**：筆記以版本號拒絕舊編輯；白板卡片數檢查及新增納入同一立即事務。 | 實際原生 Windows 多視窗競爭仍待驗證，失敗時請先複製未儲存文字。 |
| SQLite 復原、WAL 與備份 | **已有防護**：WAL、`synchronous=FULL`、SQLite `VACUUM INTO` 快照；bundled SQLite 3.54.1，已高於官方 WAL-reset 修復版本 3.51.3。[SQLite WAL](https://www.sqlite.org/wal.html)、[VACUUM INTO](https://www.sqlite.org/lang_vacuum.html#vacuuminto)。 | 不手動刪除 WAL/SHM/journal；保留並測試備份。斷電及硬體故障並未窮盡。 |
| 工作資料庫的網路位置 | **已修正**：拒絕 UNC 與 Windows 識別的遠端映射磁碟，符合 [SQLite WAL 限制](https://www.sqlite.org/wal.html)。 | 判斷依賴 Windows 路徑與磁碟類型；特殊虛擬或第三方檔案系統不能全數保證。備份與匯出目的地另有其檔案系統限制。 |
| 視窗回應與錯誤 | **已修正**：若自動儲存計時器無法建立，立即嘗試儲存；關閉及 Windows 工作階段結束會嘗試保存。失敗顯示狀態，不直接丟棄文字。[Windows 關機通知](https://learn.microsoft.com/en-us/windows/win32/shutdown/shutting-down)。 | 大型匯入及慢速磁碟的 UI 延遲、計時器失敗與關機互動未在原生 Windows 全面測試。 |
| 離線、外部元件及框架 | **未觀察**：原始碼與 PE 檢查未見應用程式自行聯網，Wine 斷網測試可啟動與保存；程式使用原生 Win32，不使用 eframe 或 WebView2。 | 不推論 Windows、Shell、系統安全軟體或網路磁碟的行為。Wine 不能取代原生 Windows 測試。 |
| 固定依賴與 GitHub 發行供應鏈 | **已驗證**：cargo-audit 0.23.0 對全目標及 Windows x64 依賴各回報 0 筆已知漏洞、0 筆警告；使用 1,563 筆公告的 RustSec 資料庫修訂 `52384ab5581a4487b62c549eef95cf7ba7e8bad5`（2026-11-23）。受檢的 v0.1.1 Cargo.lock SHA-256 為 `5816a87c264d251000df9279e49f09e571e7cf8f0c2109f88d3961e1ef8b2e03`；Windows Actions 工作流程已設定固定兩個官方 action 的完整提交、停用 checkout 憑據持久化，並檢查三項資產、ZIP 中 EXE 及來源 ZIP 所對應的 tag 提交；標籤的首輪 Windows 測試與後續 QA 分支結果見[驗證紀錄](VERIFICATION.md)。 | RustSec 結果限於該鎖檔及當時資料庫，不保證沒有未知漏洞；雜湊不能代替程式碼簽章或來源信任。 |
| Windows 安全政策與 SmartScreen | **待驗證**：執行檔未簽章。 | 尚未驗證原生 Windows 標準帳戶、IME、SmartScreen 或管理政策行為，不推測實際提示。 |

原始驗證與測試限制見 [驗證紀錄](VERIFICATION.md)，資料位置與安全清理見 [資料、暫存與離線使用](STATE.md)。
