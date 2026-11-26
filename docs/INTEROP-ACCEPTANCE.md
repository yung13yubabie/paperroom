# 三款筆記工具的互通驗收

依平常的操作分開驗收。每列標示的是該列寫明的範圍。資料格式、原廠服務和紙間本機操作各自驗證。完整功能背景另見 [逐項功能清冊](FEATURE-STATUS.md)。

| 想完成的工作 | Notion | Obsidian | Heptabase |
| --- | --- | --- | --- |
| 把頁面、欄位與附件搬進紙間 | [資料列和原始頁面](INTEROP-NOTION.md) | [Vault 與 Bases](INTEROP-OBSIDIAN.md) | [Markdown 及伴隨畫布](INTEROP-HEPTABASE.md) |
| 在本機修改和導覽 | [集合、排序和來源關聯](INTEROP-NOTION.md#已實裝且驗證) | [動態檢視與 Canvas 卡片](INTEROP-OBSIDIAN.md#已實裝且驗證) | [筆記卡和子白板](INTEROP-HEPTABASE.md#已實裝且驗證) |
| 檢查原廠備份或連線服務 | [原廠 ZIP、協作和發佈](INTEROP-NOTION.md#已實裝待真實來源) | [原廠設定、Sync 和 Publish](INTEROP-OBSIDIAN.md#未實裝) | [All-Data、Space 和分享](INTEROP-HEPTABASE.md#已實裝待真實來源) |

- **已實裝且驗證**表示已從指定入口跑完所述流程。合成輸入、原廠實檔和實際原廠介面證據各以連結標出。Windows Server 驗收只覆蓋所列 Windows Server 情境。
- **已實裝待真實來源**表示已有可執行的匯入或資料層能力，但還需要同一操作情境的原廠真檔或可寫測試空間。
- **未實裝**表示紙間尚缺所述能力。合法保留來源可以讓日後補驗。
- **離線不可實現的雲端服務**指出功能依賴跨裝置伺服器、共同編輯連線或原廠帳戶。紙間可另有本機能力，其證據另列。

## 什麼會阻擋正式發版

| 項目 | 放行所需證據 |
| --- | --- |
| 免安裝、離線的乾淨 Windows 10／11 | 從實際下載的同一份套件取 EXE，以標準帳戶斷網啟動，建檔、編輯、搜尋、備份還原、關閉再開。記錄作業系統、版本、環境與 EXE 雜湊。Windows Server 有獨立的已驗範圍。 |
| 最終可散布封裝 | 核對本次固定來源、工具鏈、簽章狀態、兩次獨立建置、通知檔與實際下載內容。GNU 與 MSVC 的連結和授權證據各依實際套件判定。 |
| 發佈說明的相容性聲明 | 有原廠匯出和相符驗收的流程才可標成原廠往返。Notion 原廠 ZIP 和 Heptabase 原廠 All-Data 尚缺經授權的樣本，會阻擋這兩項相容性聲明。 |
| 發行範圍與權限 | 正式發行另需核對候選、授權和版本範圍後才執行。草稿套件供驗證。 |

本輪整合的固定來源、下載 SHA-256 與驗收工作連結以[草稿 PR #26](https://github.com/yung13yubabie/paperroom/pull/26) 最終描述為準。
