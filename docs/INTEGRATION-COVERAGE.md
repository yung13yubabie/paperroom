# 離線工作區第一階段：功能與遷移驗收

從三款工具搬筆記到紙間時，先決定要帶走哪些資料。正文、頁面階層、卡片在白板的位置、附件及分享權限，各自需要對應的格式。紙間原有單頁 Markdown 匯出適合攜出文字；要保留紙間的完整工作區，請使用工作區備份。

這是一個尚未發佈的整合分支。「第一階段」一欄標示已接到程式介面的能力。完整功能家族與來源另見[原版功能盤點](FEATURES.md)及[公開文件索引](PUBLIC-FEATURE-INDEX.tsv)。

## REA 與公開介面證據

REA 6.10.1 以 `inspect-artifact <filename> --json` 分析三份官方發行包及三份合成 ZIP。Notion 7.44.1 的 MSIX 包含公開 Appx manifest，可核對 x64、產品版本與發行者，並看到簽章檔與 Electron 封裝。簽章信任鏈未驗證。Obsidian 1.15.1 與 Heptabase 1.117.0 的官方 AppImage 均比對了下載雜湊，REA 識別為 ELF，AppImage Type 2 另由檔頭確認。這兩份 REA 記錄只涵蓋外層。原始碼未由發行包取出；格式能否由產品接受，仍須用正常介面或匯入測試驗證。[REA 結果與下載來源](REA-EVIDENCE.json)

- 在 Notion 官方公開範本，把 Projects 從 Active 切到 Board，看到相同資料庫按 Planning、Backlog 等狀態分欄。此觀察涵蓋公開唯讀檢視。[Projects & Tasks 示範](https://notion.notion.site/Projects-Tasks-1ea72db0bc4f4c519e67ffed245560b2)
- 在 Heptabase 的公開白板，看到彩色分區與跨區連線，雙擊卡片能讀正文和內部連結。未測建立、拖曳後保存或重開。[Mindstorms 公開白板](https://app.heptabase.com/p/whiteboard/fab30d1b-c71b-402f-9dab-ee99ac48ef7c?utm_source=landing&utm_medium=gallery)
- Obsidian 的宿主圖形環境曾無法建立 Xvfb socket。改以無網卡的 Debian 客體重試後，已在官方 Obsidian 1.15.1 的正常介面驗證合成 Vault 的筆記屬性、WikiLink、反向連結、Canvas 節點及每日筆記。詳見[第二階段觀察](WORKSPACE-SECOND-STAGE.md)。[官方下載](https://obsidian.md/download)

## 哪些工作流程有對應功能？

| 工作流程 | 三款工具的公開能力 | 紙間第一階段 |
| --- | --- | --- |
| 寫作、整理與搜尋 | Notion 以頁面與區塊組織內容。Obsidian 以 Vault 內的 Markdown 檔案、連結與反向連結導覽。Heptabase 可搜尋卡片，並查閱卡片所在的白板。[Notion 頁面與區塊](https://www.notion.com/help/what-is-a-block) · [Obsidian 官方說明](https://obsidian.md/help) · [Heptabase 操作介面](https://wiki.heptabase.com/user-interface-logic) | 既有階層筆記、標題與正文搜尋、標星、Markdown 編輯／閱讀、筆記連結與反向連結。進階區塊、區塊嵌入、連結圖譜與多窗格編輯仍有差距。 |
| 每日筆記 | Obsidian 的 Daily Notes 可依日期開頁；Heptabase Journal 還可把一段時間的日誌放上白板回顧。[Obsidian 每日筆記](https://obsidian.md/help/plugins/daily-notes) · [Heptabase 日誌流程](https://wiki.heptabase.com/three-ways-to-make-sense-of-your-fleeting-thoughts-in-journal) | 已接入：依本機日期開啟當日筆記，重複開啟回到同一筆。日曆視圖、週期回顧、任務提醒與範本套用仍待後續規劃。 |
| 視覺化整理 | Obsidian Canvas 可保存節點、群組和連線。Heptabase 同一卡片可放入多張白板，另有子白板、分區、心智圖及連線。[JSON Canvas 格式](https://jsoncanvas.org/spec/1.0/) · [Heptabase 研究流程](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [Heptabase 功能紀錄](https://wiki.heptabase.com/roadmap) | 已接入：命名多張白板，讓同一篇筆記在各板保存各自的位置，並建立或移除同一板上的卡片連線。分區、心智圖、子白板及 JSON Canvas 匯入仍未完成。 |
| 屬性、表格與規劃 | Notion 資料庫提供屬性、公式、關聯及彙總。Obsidian Bases 對檔案屬性提供視圖、篩選與公式。Heptabase 標籤資料庫提供表格與看板。[Notion 關聯與彙總](https://www.notion.com/help/relations-and-rollups) · [Obsidian Bases](https://obsidian.md/help/bases) · [Heptabase 表格與看板](https://wiki.heptabase.com/version-one) | 已接入：每篇筆記的文字屬性、按一個欄位搜尋的唯讀表格，以及獨立的 UTF-8 CSV 匯入。CSV 第一欄是筆記標題，欄值保持文字。尚無型別、公式、關聯與彙總、看板、時間軸、圖表、表單、儀表板或自動化。 |
| 閱讀與摘錄 | Heptabase 有 PDF 畫線與集中摘錄、桌面網頁分頁、Web Clipper 收件匣及 Readwise 整合。Obsidian 可在 Vault 保存附件；Notion 匯出時可包含檔案。[Heptabase 閱讀入口](https://wiki.heptabase.com/user-interface-logic) · [Obsidian 附件](https://obsidian.md/help/attachments) · [Notion 匯出](https://www.notion.com/help/export-your-content) | 可在正文手動記錄摘錄及來源。附件封存、PDF 頁碼與選取區域、畫線回跳、影音轉錄、網頁擷取及外部文獻服務均未列入本次整合。 |
| 離線使用與線上服務 | Notion 桌面／行動版可下載指定頁面，子頁須分別下載。Obsidian 的筆記及 Bases 資料存於本機，另有 Sync 與 Publish。Heptabase 桌面版有本機資料庫；同步、AI、分享與協作會涉及遠端服務，版本還原要求連線。[Notion 離線頁面](https://www.notion.com/help/use-pages-offline) · [Obsidian 官方說明](https://obsidian.md/help) · [Heptabase 資料儲存](https://support.heptabase.com/en/articles/10448141-is-my-data-safe-in-heptabase) · [版本還原](https://support.heptabase.com/en/articles/10448124-restore-from-version-history) | 紙間以 Windows 本機 SQLite 工作區操作。這些本機功能沿用同一工作區。升級會將資料庫從第 2 版遷移到第 3 版；跨版建立的副本應以備份或另一套工作區驗證。雲端同步、共同編輯、權限、公開網站、AI、外掛平台及企業管理均在此次範圍外。 |

## 搬移前怎麼驗收？

請使用合成筆記、來源資料夾副本及新的目標工作區。記下候選版的 commit、EXE SHA-256、作業系統與連線狀態。每一列都是可重做的檢查；執行後再填入實測紀錄。

| 資料與案例 | 操作 | 通過條件與需記錄的界線 |
| --- | --- | --- |
| Notion 匯出的父頁、中文子頁、CSV、圖片 | 在 Notion 選 Markdown & CSV 並含子頁，於紙間匯入 ZIP 內的 Markdown。需要表格時，從原匯出檔另行取出 CSV 並選「匯入 CSV 表格」。 | 比對頁數、父子關係和中文正文。核對略過項目提示並保留原 ZIP；附件與頁內連結重映射未支援。獨立 CSV 匯入未還原日期型別、公式或資料庫視圖。 |
| Obsidian 的 Markdown 資料夾 | 準備兩個子目錄，各有同名中文筆記及不同正文，另放圖片、.canvas 與 .obsidian 設定。記下來源檔清單和雜湊後匯入副本。 | 核對兩篇同名筆記的歸屬與內容，關閉重開後再查看。對照來源雜湊及匯入摘要。相對及路徑 WikiLink 要能定位原筆記；無法唯一定位的檔名不建立連結。YAML 保留原文，未解析成紙間屬性。只定位到筆記層，未定位標題或區塊錨點。附件、嵌入與 Vault 雙向同步均未完成。 |
| Heptabase 卡片及白板 | 自行匯出可攜文字，僅將受支援的 UTF-8 Markdown 交給紙間單篇匯入。保留原服務的完整備份。 | 核對標題和正文。Heptabase 備份還原、卡片識別碼、白板座標、PDF 畫線錨點及分享權限沒有對應匯入器。命名多白板須另以紙間原生操作驗收。[Heptabase 備份說明](https://support.heptabase.com/en/articles/10448124-restore-from-version-history) |
| 舊白板升級與多板共用 | 在舊版建立兩張卡，記下位置並備份；於工作區副本升級。將同一篇筆記加入第二張命名白板，移動卡片、修改正文，再從第二板移除。 | 舊卡與位置仍在；移動只影響所在白板；修改正文可從兩板看到。移除後第一張白板與原筆記仍在。完整備份另開後再比對。 |
| 中繼資料與表格 | 以兩篇同名筆記填入不同的文字欄值，在表格比對列。回到筆記後修改其中一筆，再重開工作區。 | 表格依值與標題排序，可按文字篩選；中文欄值重開及另開完整備份後一致，空值保持空值。分開核對 Markdown 單頁匯出實際包含哪些欄位。 |
| 每日筆記 | 透過入口開啟當日頁、寫入中文後關閉，再於同一天重開；使用可控制日期的測試另驗跨日。 | 同一天回到原筆、正文保留；隔天有獨立頁，仍可搜尋前一天的筆記。檢查手動建立的同名頁有無被誤用或覆寫。 |
| 離線、寫入失敗與復原 | 在標準 Windows 帳戶斷網首啟，建立合成資料、搜尋、關閉重開、另開完整備份。另測儲存已確認後的受控中斷。 | 記錄首次啟動是否要求登入或下載，逐項核對筆記、屬性及白板。分開記錄已確認儲存與尚在輸入的內容。Windows、Wine 與純資料層測試各自註明環境。 |

若只需要攜出某一篇的正文，可用紙間的 Markdown 匯出。若要搬走整套工作區，請使用備份並另開核對。原始 Vault、第三方備份和工作區副本應保留到驗收完成。[紙間資料與備份說明](STATE.md)
