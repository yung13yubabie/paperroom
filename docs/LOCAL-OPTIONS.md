# 紙間離線格式與可選服務研究

## REA 6.10.1 靜態紀錄

已對官方 Notion 7.44.1 x64 MSIX、Obsidian 1.15.1 x64 AppImage、Heptabase 1.117.0 x64 AppImage 執行 `node package/scripts/rea.mjs inspect-artifact <filename> --json`。發行來源、SHA-256、REA evidence ID、容器範圍與簽章限制收於 [六份靜態檢查摘要](REA-EVIDENCE.json)。兩份 AppImage 與官方發布雜湊相同，REA 在兩者只涵蓋 ELF 外層。Notion 的 MSIX 確認有公開套件 manifest、簽章項目與 ASAR 路徑。這次未啟動桌面程式或讀取內嵌程式碼，也未驗證 Notion 簽章信任鏈。

以相同命令實際送驗 `research.canvas`、`research.pdf` 和 `research-fixtures.zip`。REA 拒絕前兩個裸檔，皆回 `unsupported_target / file_format_rejected` 且結束碼 1。對 ZIP 結束碼 0，涵蓋 central directory 成員 `Research.md`、`research.canvas` 和 `research.pdf`。ZIP SHA-256 為 `314c5580abb51772979c85826527b7ee336e89e5ceca4c32ba76e0fd868a6eae`。用 Python `json` 和 `zipfile` 驗證兩個節點、一條邊、相對檔名可解析及 ZIP CRC。用 `pdfinfo research.pdf` 驗證 PDF 1.4、1 頁，`pdftotext research.pdf -` 讀出預期句子。這是合成樣本的有限驗證。

## 公開文件中的操作對應

| 官方公開操作 | 紙間可採用的資料對應 |
| --- | --- |
| Obsidian 透過命令面板的 **Canvas: Create new canvas** 建立 `.canvas`，在白板上拖入 Vault 筆記或加入獨立文字卡 | JSON Canvas 的 `file` 節點以工作區相對路徑連到匯入筆記。 `text` 節點可保持獨立。保留節點順序、位置、尺寸、邊的方向和標籤。合成樣本涵蓋 `file`、`text` 與單條邊。 |
| Heptabase 的 **Settings → Backup → Export now** 匯出 Markdown，完整備份另含 `All-Data.json`。公開文件也記載 PDF 卡、畫線及筆記。 | Markdown 作可移轉內容。PDF 原檔、頁碼、選區和註解分開儲存，維持可離線回到原文。白板關係、PDF 高亮等欄位需要獨立格式證據，ZIP 成員清單不能驗證。 |
| Notion 在桌面或手機頁面開啟 **Available offline**，子頁需獨立下載，AI、分享與權限編輯受連線限制。 | 紙間可在本機搜尋及編輯已匯入內容，資料範圍由使用者的匯出包決定。遠端工作區分享與權限需要另外設計部署。 |

來源：[Obsidian Canvas](https://obsidian.md/help/plugins/canvas)、[JSON Canvas 1.0](https://jsoncanvas.org/spec/1.0/)、[Heptabase 資料可攜](https://support.heptabase.com/en/articles/10364279-business-sustainability-and-data-portability)、[Heptabase PDF 與權限](https://support.heptabase.com/en/articles/10510497-collaboration-and-permissions)、[Notion 離線頁面](https://www.notion.com/help/use-pages-offline)。以上操作取自官方公開介面說明。

## 可選的離線或自架做法

- **跨裝置檔案同步**。Syncthing 可在核准裝置間交換匯出的 Markdown、附件和備份快照。它會複製檔案移除操作，衝突出現時需處理衝突副本。紙間應讓使用者主動選擇同步資料夾，避免將正在寫入的 SQLite 檔案及 WAL 交給檔案同步，保留獨立版本備份。[Syncthing FAQ](https://docs.syncthing.net/users/faq.html)
- **即時多人編輯**。自架 Yjs WebSocket 可交換文件更新及游標等狀態，可自行持久化。專案仍需設計帳號、工作區授權、備份和權限撤銷。Nextcloud Office/Collabora 提供另一條自架共編路線，適用於外部 Office 文件，可由使用者另行部署並選擇連線，不直接處理紙間資料庫。[Yjs WebSocket](https://docs.yjs.dev/ecosystem/connection-provider/y-websocket) · [Nextcloud Office](https://docs.nextcloud.com/server/stable/admin_manual/office/index.html)
- **本機 AI**。Ollama 預設監聽 `127.0.0.1:11434`，可用 `OLLAMA_NO_CLOUD=1` 關閉雲端模型與網路搜尋，重新啟動後可查閱日誌確認。紙間可讓使用者選擇本機模型和送入的筆記或選取文字。初次取得模型需連線或預先備好模型。同步、共編與 AI 皆是按需啟用的方案，這次未部署。[Ollama 本機常見問題](https://docs.ollama.com/faq)
