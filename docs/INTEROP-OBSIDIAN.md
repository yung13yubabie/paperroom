# Obsidian 操作驗收

[返回三款互通驗收](INTEROP-ACCEPTANCE.md)

## 已實裝且驗證

| 操作 | 驗收範圍 |
| --- | --- |
| 匯入 Vault Markdown 並開啟原本筆記 | 原廠 Obsidian 測試 Vault 驗過相對路徑及畫布附件。另以兩篇同名的合成筆記核對依完整路徑區分內容與身分。[Vault 與驗收](UNIFIED-WORKSPACE.md) |
| 開啟支援的 Bases 表格、清單、欄位和公式 | 有明確支援的檔案欄位、字面路徑與布林／數值比較。Windows 實測同一欄位的兩個別名、缺欄、null、false 和 0，保存分組和收合。原廠語法的適用範圍以文件為準。[紙間驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45763582824/job/139720173599) · [原廠語法](https://obsidian.md/help/bases/syntax) |
| 讓既有候選依欄位條件和排序進出檢視 | 合成 Base 驗過相等的數值依路徑及 ID 排定，變更欄位後既有候選進出，重開和隔離還原結果一致。[動態檢視](https://github.com/yung13yubabie/paperroom/actions/runs/45766439419/job/139728353543) |
| 從同一路徑的兩張 Canvas 卡開啟各自釘選的檢視 | 原廠桌面實際存出相同 .base 路徑與兩個獨立的節點 subpath。紙間驗過各自開啟、撤銷與重新配對、重開、隔離還原及原始 Canvas 位元不變。[原廠樣本與紙間重開](https://github.com/yung13yubabie/paperroom/actions/runs/45766439419/job/139728353032) |
| 畫布帶本機圖片、背景和連線 | 原廠存檔經紙間開啟、重開與逐位元匯出，核對三條連線與兩張 PNG。[原廠畫布往返](https://github.com/yung13yubabie/paperroom/actions/runs/45763582824/job/139720173405) |
| 從已釘選的 Canvas 卡直接修改原筆記欄位 | 檔案卡右鍵開啟布林與數值欄位編輯；同 Canvas 各卡依目前筆記重新取值。來源變更時會重驗卡片、視圖、筆記與版本。[本輪驗收流程](../scripts/windows-pinned-card-live-scope.ps1) |
| 在同一 Vault 經明確確認接納新檔、移動和改名 | 另選隨 Vault 來源變動的 Bases 模式，以已核對的來源身分和路徑重算。當前檢視和完整備份仍依同一筆記 ID。[本輪驗收流程](../scripts/windows-pinned-card-live-scope.ps1) |

## 已實裝待真實來源

- 本機比較公式、複合排序、群組與編輯有 Paperroom 的實測邊界。仍需涵蓋不同原廠版本、複合實際 Vault 設定及有外掛的授權樣本，才能對那些情境宣稱相容。[原廠視圖規格](https://obsidian.md/help/bases/views)

## 未實裝

- 原廠 Bases 完整函數、所有視圖和外掛。地圖和卡片版型的完整同等呈現。
- Obsidian 設定資料夾、外掛狀態和原廠視窗配置完全還原。

## 離線不可實現的雲端服務

- Obsidian Sync 的遠端儲存、跨裝置歷史及共同 Vault，以及 Obsidian Publish 的網站服務。[Sync](https://obsidian.md/help/sync) · [Publish](https://obsidian.md/publish)
