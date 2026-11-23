# 三款筆記軟體功能對照

範圍涵蓋三家的公開說明中心與文件索引。以 REA 6.10.1 盤點本地保存的官方公開文件，並用它檢查一份依 Notion 公開格式製作的 Markdown ZIP 範例。以下按公開功能家族整理。原始文件入口逐項收在 [PUBLIC-FEATURE-INDEX.tsv](PUBLIC-FEATURE-INDEX.tsv)。索引包含指南與歷史公告，網址數量不等於獨立功能數量。另外用 REA 靜態檢查三家的實際桌面發行包。涵蓋公開功能範圍，未登入逐項試用，也未驗證每個方案的權限組合。

## REA 桌面發行包檢查

| 應用程式 | 品項內的檔案 | Electron 應用封裝 SHA-256 |
| --- | ---: | --- |
| Notion 7.44.1 | 119 | `0178af07754b1c63c275dc8c3524f16a77b6ec8c1408a17a35e01af74a5bbc07` |
| Obsidian 1.15.1 | 369 | `42bf7f34782752f159d63c17ac6bac4ad01dd55709578f9879f76f8295abb2eb` |
| Heptabase 1.117.0 | 12,789 | `7732eacb3eafcd348e2eaa221bf32d46cb278b1c3187550b901cf675e88afd88` |

發行來源採用各家官方下載。檔案數是靜態封裝清單中的檔案數，與功能個數或功能是否可用無關。

## Notion

| 公開功能家族 | 紙間目前支援 |
| --- | --- |
| 頁面、子頁、側欄、階層導覽 | 巢狀筆記樹、搬移、路徑、標星 |
| 區塊、斜線選單、格式、快捷鍵 | 透過 Markdown 呈現標題、清單、待辦、引用、程式碼、分隔線，具斜線選單和閱讀模式 |
| 連結頁、提及、同步區塊、分欄、摺疊、目錄、數學、繪圖、音訊、附件和網頁嵌入 | 內部筆記連結；其餘目前未支援 |
| 搜尋、快速跳轉、最近瀏覽、首頁、通知與收件匣 | 中文標題／正文搜尋及連結跳轉 |
| 資料庫、資料來源、屬性、模板、公式、關聯、彙總、篩選、排序、分組 | SQLite 為應用程式儲存層；沒有使用者資料庫 |
| 表格、看板、清單、日曆、時間軸、圖庫、圖表、表單、儀表板 | 可拖曳筆記卡片的白板；未重現資料庫視圖 |
| 專案、任務、Wiki、翻譯、驗證及知識管理流程 | 可用一般頁面、子頁和待辦手動整理 |
| 匯入、匯出、備份、垃圾桶、頁面版本、複製和還原 | 匯入 Markdown／Notion Markdown ZIP、單頁 Markdown 匯出、工作區備份、垃圾桶還原；無版本歷史 |
| 團隊空間、成員、來賓、群組、分享權限、即時共編、評論、通知與提醒 | 單機工作區 |
| 離線下載、桌面／Web／手機、元件和捷徑 | Windows x64 原生，本機常駐完整工作區 |
| Sites、公開頁、網域、Marketplace、範本、付費方案 | 目前未支援 |
| 按鈕、資料庫自動化、Webhook、連線、API、Workers、CLI、外部代理 | 目前未支援 |
| Notion AI、會議筆記、AI 搜尋、連接器、代理、點數、管理與稽核 | 目前未支援 |
| 帳戶安全、SAML、SCIM、稽核紀錄、資料區域、加密和企業管理 | 使用 Windows 本機檔案權限；沒有應用程式層級加密 |

[Notion Help Center](https://www.notion.com/help) · [Notion 匯出格式](https://www.notion.com/help/export-your-content)

## Obsidian

| 公開功能家族 | 紙間目前支援 |
| --- | --- |
| 本機 Vault、資料夾、檔案總管、刪除方式、附件位置、匯入 | 本機 SQLite 工作區、筆記樹、Markdown 匯入／匯出、垃圾桶；不直接開啟或監看 Vault |
| Markdown、閱讀檢視、Live Preview、原始碼、核取方塊、程式碼、數學、表格、Callouts、圖示 | Markdown 編輯和獨立閱讀模式，支援基本區塊與待辦；無行內即時預覽 |
| `[[WikiLinks]]`、標題／區塊參照、嵌入、外部連結、別名 | 標題式或穩定 ID 式連結，從獨立選單導覽；不展開嵌入與區塊參照 |
| Backlinks、Outgoing Links、未連結提及及 Graph View | 明確的雙向連結清單；沒有圖譜、未連結提及或建議 |
| Search、Quick Switcher、Command Palette、Bookmarks、Page Preview、Outline | 搜尋、標星、快捷鍵與頁面路徑；未提供完整命令面板、懸浮預覽或大綱 |
| Canvas、連線、群組、各類節點、JSON Canvas | 本機筆記卡片白板，可拖曳與保存座標；未匯入／匯出 JSON Canvas、未支援邊與群組 |
| Properties、Tags、Bases、程式化公式與多種視圖 | 目前未支援 |
| Daily Notes、Templates、Random Note、Slides、Word Count、Audio Recorder、Workspaces 等核心外掛 | 目前未支援 |
| 設定、主題、CSS 片段、外掛、開發者 API、URI、CLI | 目前未支援 |
| Sync、版本歷史、Publish、Teams、Web Clipper、分享與公開 | 目前未支援 |

[Obsidian 官方說明](https://obsidian.md/help) · [JSON Canvas 1.0](https://jsoncanvas.org/spec/1.0/)

## Heptabase

| 公開功能家族 | 紙間目前支援 |
| --- | --- |
| 筆記卡、卡庫、篩選、相關卡、近期與全域搜尋 | 一篇筆記可作一張卡，左欄可搜尋、標星 |
| 白板、子白板、分區、位置、顏色、連線、心智圖、自由文字 | 一張工作區白板，可放置和拖曳筆記卡、雙擊開頁、保留位置 |
| 內部連結、引用、反向連結、關聯探索、並排窗格、分頁 | 內部與反向連結；沒有卡片嵌入、分頁與並排閱讀 |
| 屬性、標籤、表格、看板、按欄位整理 | 目前未支援 |
| 日誌、每日卡、日曆、任務／待辦 | 一般筆記可放待辦，無日誌日曆 |
| PDF 螢光標記、區域批註、Highlight Cards；影音、圖片、網頁、電子書與研究資料 | 目前未支援 |
| Readwise、Zotero、匯入、資料匯出、版本歷史 | UTF-8 Markdown 單篇匯入、單頁匯出與工作區備份；無 Heptabase 專屬資料還原或來源重映射 |
| AI 聊天、研究、PDF／影片／白板上下文、AI Tutor、建議、跨卡片檢索和自動動作 | 目前未支援 |
| 跨平台同步、公開卡片或白板、多人協作、評論、行動裝置 | Windows x64 單機工作區 |
| 介面快捷鍵、側欄工具、命令面板、離線操作 | 部分本機快捷鍵、頁面樹、搜尋及離線白板 |

[Heptabase Public Wiki](https://wiki.heptabase.com/) · [基本元素](https://wiki.heptabase.com/fundamental-elements) · [介面](https://wiki.heptabase.com/user-interface-logic)
