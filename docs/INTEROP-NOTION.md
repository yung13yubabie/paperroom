# Notion 操作驗收

[返回三款互通驗收](INTEROP-ACCEPTANCE.md)

## 已實裝且驗證

| 操作 | 驗收範圍 |
| --- | --- |
| 建立和編輯本機集合 | 集合可保存多欄排序、欄位及具名檢視。此為紙間本機能力。[原生驗收證據](https://github.com/yung13yubabie/paperroom/pull/17) |
| 以原始 CSV 附件建立可編輯資料列 | 合成 ZIP 驗證數字、核取方塊、日期、前導零、空值、取消及拒收。重開和隔離還原保留原始 CSV 位元。[Windows 驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45763582824/job/139720173464) |
| 從資料列開啟確切來源頁和附件 | 合成匯入以批次、完整 ZIP 路徑和筆記 ID 手動確認。驗過同名頁與同名附件各自保留，隔離還原仍開到同一筆記。[Windows 驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45765855523/job/139727549924) |
| 保留工作區內的具名關聯 | 來源可經使用者確認，紙間在既有筆記模型中保存方向和目標 ID。MCP 交接格式的來源證據與一般 CSV 的連結文字分別處理。[交接文件](NOTION-MCP-TRANSFER.md) |
| 更換既有資料列的精確 Notion 來源 | 顯示舊、新路徑及筆記身分。確認後原子替換，取消或寫入失敗會保留舊來源。兩個同名附件各自保留。[本輪驗收流程](../scripts/windows-notion-source-replacement.ps1) |

## 已實裝待真實來源

| 操作 | 尚缺的證據 |
| --- | --- |
| 從真實 Markdown／CSV ZIP 保留巢狀頁和本機附件 | 有可執行匯入。已用官方 MCP 在專用測試資料庫比對預設檢視三筆、篩選後一筆及正向關聯 ID。仍需同一資料庫的原廠匯出 ZIP，把實際頁面與附件逐檔比對。[原廠匯出](https://www.notion.com/help/export-your-content) |
| 從可核對的資料重建已確認的具名邊 | 紙間交接第 3 版有來源、目標與資料來源 ID 驗證。仍要對上相同原廠匯出或可授權交接的證據。CSV 的 URL 只足以保留原始文字。[原廠關聯與彙總](https://www.notion.com/help/relations-and-rollups) |

## 未實裝

- 原廠資料庫設定、原廠公式語法、活彙總、反向欄位與多視圖的完整匯出還原。

## 離線不可實現的雲端服務

- 跨使用者即時共同編輯、團隊空間與訪客權限、Notion 公開發佈，都取決於有權帳戶和伺服器。[Notion 分享與權限](https://www.notion.com/help/sharing-and-permissions)
