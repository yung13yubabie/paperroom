# 版本驗證紀錄

## 0.1.2 未簽章候選檔

Windows x64 GNU 執行檔為 1,906,176 位元組，SHA-256 為 `b0f9b1dd2bd4d041fe021d4f01772f40d69bc4430457a0dee1c747b4b806382e`。直接解析 PE 的 RT_VERSION 資源，產品名稱為 `Paperroom`；FileVersion 與 ProductVersion 的顯示字串都為 `0.1.2`，固定數字欄位均為 `0.1.2.0`。PE 憑證表為空。

以同一雜湊的 EXE 在無網卡的 Debian 13／Wine 10 虛擬機驗證首啟、建立和自動儲存筆記、正常關閉、存檔後強制結束及重開，已儲存筆記仍可讀，SQLite 完整性檢查通過。在筆記樹輸入繁中關鍵字，選取唯一結果後也能讀回標題與內文。另驗證可攜模式的首啟與正常關閉。Linux 的 13 項測試及以相同原始碼編譯的 Windows 測試程式 16 項全部通過。以下 v0.1.1 的完整 GUI 匯入、備份、還原紀錄屬於該版本。

候選版亦採用下述樹狀控制項修正。該修正在另一個 QA 分支通過 Windows Server 2022 驗證。帶 VERSIONINFO 的 0.1.2 完整候選尚未在 GitHub-hosted MSVC 建置或執行；簽章、時間戳和 SmartScreen 仍待驗收。

### 手動驗證公開 Release

在 GitHub 的 Actions 開啟 **Windows storage checks**，按 **Run workflow**，選擇含驗證程式的分支。在 `release_tag` 輸入公開標籤，例如 `v0.1.1`。留空則執行所選分支的 Windows 原始碼測試。

流程先解析標籤指向的 commit，再以公開網址下載 EXE、Windows ZIP、原始碼 ZIP 和 SHA-256 清單。核對三份檔案的雜湊及原始碼 ZIP 的 commit 與檔案內容，逐位元組比對 Windows ZIP 內外的 EXE。最後使用所選分支的測試腳本，對剛下載核對的同一份 EXE 執行 Windows 儲存測試。

## v0.1.1

GitHub-hosted Windows Server 2022 已以 SHA-256 核對原版 GNU EXE。預設模式和可攜模式都完成首啟、編輯、自動儲存、正常關閉、強制結束、重開與 SQLite 完整性驗證。[原版 EXE 的 Windows 實測](https://github.com/yung13yubabie/paperroom/actions/runs/45284932624)

標籤原始碼的 MSVC 測試有 15 項通過。原生編輯器測試要求 Windows 一次註冊樹狀與標準控制項，初始化回傳失敗。在獨立 QA 分支改為只註冊所需的樹狀控制項，正式啟動與測試共用同一檢查；16 項原生測試與儲存測試全數通過。原版 v0.1.1 EXE 並未編入這項修改。[標籤的測試紀錄](https://github.com/yung13yubabie/paperroom/actions/runs/45284252126) · [修正後的 QA 紀錄](https://github.com/yung13yubabie/paperroom/actions/runs/45284932451)

Windows x64 GUI 執行檔使用 Rust 1.100.0、GNU 目標和 MinGW-w64 GCC 14.2 建置。兩次建置分別使用空的輸出目錄，產物逐位元組相同。執行檔未簽章。

- EXE SHA-256 為 `0b281c84d7cf82c03c6262307a2b5b3585ba180e107119dc02b7518f1473de82`。
- Linux 上的 13 項 Rust 測試通過，包含寫入失敗的清理、寫入期間被其他程式佔用的輸出檔名，較新版資料庫被拒絕開啟後仍保留原本的日誌模式，以及既有工作區資料的開啟和備份。
- 在 Linux 直接執行本版的 Rust 儲存模組，以兩個獨立目錄分別模擬預設與可攜配置。兩次測試都觀察到寫入期間有主資料庫、WAL 和 SHM，正常關閉後只剩主資料庫。儲存一筆繁體中文筆記後強制終止，再開啟均能讀回該筆記，SQLite 完整性檢查均為 `ok`。這項測試直接指定資料路徑，沒有執行 Windows 的圖形介面與路徑選擇。
- Windows 測試執行檔在配置完整 UTF-8 locale 的 Wine 10 中通過 16 項測試，包括 12 層中文資料夾的 Markdown 匯出、SQLite 備份和重開。Wine 先前曾在取得深目錄的最終路徑時回報 Win32 錯誤 234。修正只在該錯誤時保留原本的 UTF-16 路徑並改用延伸路徑格式。
- 原始碼中的 `.github/workflows/windows.yml` 會在隔離的工作區記錄首啟、正常關閉、強制結束和重開後的檔案。發佈後另比對 Release 附件的 SHA-256，再對該 EXE 執行相同的測試。

### 斷網執行 Windows EXE

以 QEMU 啟動無網卡的 Debian 13 虛擬機，使用 Wine 10.0 開啟上列 SHA-256 的單一 EXE。工作區使用虛擬機的本機磁碟。首次開啟、建立繁體中文筆記、以中文關鍵字搜尋標題，以及以正常關閉再開啟的方式讀回筆記，均已觀察到應用程式畫面。另從檔案選單匯入了 UTF-8 Markdown 和含子頁的 Notion 測試 ZIP，逐筆核對儲存的標題與正文。從「備份工作區」產生快照，把備份複製到另一個空白可攜工作區，以同一支 EXE 開啟後能找到原本的繁中筆記，SQLite 完整性檢查回報 `ok`。搜尋用的中文字由測試工具輸入，未進行實體中文輸入法測試。

| 操作 | 實際觀察 |
| --- | --- |
| 首次啟動 | 首建的資料夾內出現主資料庫及 WAL、SHM。虛擬機中的檔案事件追蹤也捕捉到 SQLite 短暫建立的 `notes.sqlite3-journal`。 |
| 正常關閉最後一個視窗 | 工作區留下 `notes.sqlite3`。 |
| 已儲存後強制終止 | 主資料庫旁留下 WAL、SHM。重開可讀回該筆記，正常關閉後再檢查，SQLite 完整性結果為 `ok`。 |
| 可攜模式 | SQLite 檔案落在執行檔旁的 `paperroom-data`，正常關閉後只剩主資料庫。 |

測試圖使用虛擬機內的 Noto Sans CJK TC 替代字型。追蹤涵蓋受測 Wine 行程與子行程的檔案操作，包含 Wine 載入系統元件的動作。測試未取得乾淨 Windows 10／11 標準帳戶、原生 SmartScreen 結果或全機事件紀錄。

另以 v0.1.0 原版建立測試工作區，將副本交給 v0.1.1 開啟並正常關閉。原有筆記仍可讀取，資料庫結構版本為 2，SQLite 完整性檢查通過。

### 啟動受阻的範圍

雲端 Linux 主機最初在啟動 Wine 服務時回報 `wineserver: socket: Operation not permitted`，當時紙間尚未執行。獨立虛擬機的精簡 Wine 環境還缺套件提供的 zlib DLL 和繁中字型。補齊測試環境後，EXE 能離線啟動。Windows 版本的程式不需要安裝這套 Wine 環境。v0.1.1 改為啟動後才查詢三個高 DPI 系統函式，缺少時可以使用 96 DPI。尚未在早期 Windows 10 版本實機驗證。

Windows 若在開啟前提示封鎖，請記下提示全文、Windows 版本和檔案的 SHA-256。此 EXE 未簽章。SmartScreen 可能評估下載檔信譽，受管理的電腦另有執行政策。雜湊可核對下載內容，無法確認某台電腦的封鎖原因。可參考 [Microsoft 的 SmartScreen 說明](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)。

## v0.1.0 原始交付紀錄

### 交叉編譯

- 發行目標為 Windows x64，Rust 1.100.0，PE32+ Windows GUI 執行檔。
- 檢查 Windows 匯入表與動態載入路徑，應用程式使用 Windows 系統 DLL。SQLite 編入執行檔。
- REA 用於開發環境中的封裝與公開功能研究。發行程式沒有連向 REA 或筆記供應商的服務。
- 此次環境可交叉編譯、讀取 Windows PE，以及執行 Rust 資料層測試。未取得可登入的乾淨 Windows 圖形環境進行冷啟動、IME 和操作流程實測。

### RustSec 依賴檢查

RustSec Advisory Database 修訂 `43b5554d9d64ca91c8d0839b7150d4f8ee490cdc`，1,562 筆公告。第一次鎖檔檢查命中 SQLite 1 筆與 ZIP 6 筆。升級至 `rusqlite 0.43.0`（`libsqlite3-sys 0.40.0`）與 `zip 9.8.2`，使用同一份資料庫重新檢查為 **0 筆已知漏洞公告、0 筆警告**。此結果限定該資料庫和該鎖檔版本。

### 防禦性程式碼檢查

- 查核匯入路徑、壓縮檔資源上限、資料寫入的事務一致性、跨視窗編輯，以及 Windows 原生文字控制項。
- Linux 主機 8 項測試通過。測試涵蓋 UTF-8、由 Markdown ZIP 建立巢狀頁、忽略不支援項目、資料重開、工作區備份、垃圾桶、連結及白板位置。
- Astra 修復分支 `astra/defensive-fixes` 已合併，包含原稿及 RTF 預覽隔離、限制 ZIP 讀取、保護備份與修正切頁事件。另外交叉編譯了 Windows 原生控制項測試。
- 本機資料庫為明文。標準 Windows 檔案權限限制其他帳號的讀取，有檔案存取權限的人可以讀取筆記和備份。
