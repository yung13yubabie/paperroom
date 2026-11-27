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
| 限制單一 Bases 檢視的結果數 | 以公開 `limit` 語法匯入未分組的動態表格或清單，按現有條件和排序顯示前 N 筆。改值後重算，未顯示的候選仍保存。 [原廠語法](https://obsidian.md/help/bases/syntax) |
| 讓型別分組降冪顯示 | 支援以既有數值、布林欄位或可辨認的布林比較公式分組並降冪顯示。缺欄、null 與無效值各保留獨立分組，排序在有效值之後。原始設定維持原樣。 [驗收腳本](../scripts/windows-bases-desc.ps1) |

### 重新命名 Vault 小節

開啟已匯入的 Vault 筆記，按「連結」→「重新命名 Vault 小節…」。選取本篇唯一的單行純文字 ATX 標題（例如 `## 舊小節`），輸入新名稱，再核對預覽列出的筆記路徑、編號與引用筆數。按「重新命名」會一次儲存全部變動。這個本機工作流程參考了 [Obsidian 的小節重新命名命令](https://obsidian.md/changelog/2021-08-23-desktop-v0.12.14/)。

檢查範圍是同次匯入、仍在使用中的 Vault 筆記。支援能定位到同一篇筆記的直接 Wiki 連結與行內 Markdown 連結，也包含本篇的引用。既有別名與顯示文字保留，Markdown 網址的新小節名稱會使用 UTF-8 百分比編碼。原始 Vault 檔案維持原樣，可另用既有 Vault 寫回預覽核對變更。外部檔案、垃圾桶、嵌入與 HTML 連結不在這次調整範圍。

若改名會讓已解析的參照式連結或階層定位失效、目標名稱衝突，或受影響的正文含螢光標記，紙間會停止。預覽後有人修改相關筆記時，請重新預覽。一次可檢查同批最多 5000 篇、合計 16 MiB 正文及目標筆記 2000 個小節。

若一般 JSON Canvas 已明確配對到這個 Vault，檔案卡的路徑及小節也都唯一指向這篇筆記，預覽會列出要更新的 Canvas 編號與節點 ID。套用後，工作區內儲存的 Canvas `subpath` 會指向新小節；同名的其他筆記、區塊 ID、未配對畫布及 `.base` 檢視釘選維持原定位。可能把先前無效的小節變成有效連結時，請先確認或編輯該檔案卡，再重新預覽。磁碟上的來源 Canvas 和 ZIP 保留原始位元組。工作區 Canvas 可依既有容量限制撤銷上一步；這只撤銷該畫布。如需一併復原正文與各畫布，請在改名前建立完整備份。 [驗收步驟](../scripts/windows-vault-heading.ps1)

## 已實裝待真實來源

- 本機比較公式、複合排序、群組與編輯有 Paperroom 的實測邊界。仍需涵蓋不同原廠版本、複合實際 Vault 設定及有外掛的授權樣本，才能對那些情境宣稱相容。[原廠視圖規格](https://obsidian.md/help/bases/views)

## 未實裝

- 原廠 Bases 完整函數、所有視圖和外掛。地圖和卡片版型的完整同等呈現。
- Obsidian 設定資料夾、外掛狀態和原廠視窗配置完全還原。

### 保留已綁定的 Bases 來源

還原版本或刪除附件時，紙間會檢查已綁定的 `.base` 原檔。版本還原若會移除或改動原檔，預覽會停止整次操作；同一則筆記的其他附件與正文仍可正常還原。需要原始檔時使用「儲存附件副本」。若要保留不同內容的 `.base`，另行匯入新來源。

## 離線不可實現的雲端服務

- Obsidian Sync 的遠端儲存、跨裝置歷史及共同 Vault，以及 Obsidian Publish 的網站服務。[Sync](https://obsidian.md/help/sync) · [Publish](https://obsidian.md/publish)

### 找到反向連結在原文中的位置

在已匯入 Vault 的目標筆記開啟「連結 → 查看 Vault 反向連結原文位置…」，選擇來源路徑、行號和連結文字。編輯器會反白該次引用的完整 Markdown。同一篇來源引用兩次會出現兩列。這份清單限於同一 Vault 其他現用筆記的 Markdown 與 Wiki 筆記連結。程式碼、嵌入、無法唯一解析的同名連結不列入。若來源在選單開啟期間改動，請重新開啟清單。完整掃描上限為 5,000 份路徑、64 MiB 含連結正文及 500 個命中。
