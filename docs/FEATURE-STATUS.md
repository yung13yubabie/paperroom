# 三款產品功能狀態

本表按產品記錄 252 列公開功能，包含同一類功能在不同產品的列。每列依目前紙間的程式和證據標示。[公開文件索引](PUBLIC-FEATURE-INDEX.tsv) · [操作及驗收紀錄](UI-ACCEPTANCE.md)

| 產品 | 列數 | 已驗證 | 部分 | 未實作 | 外部服務 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Notion | 99 | 8 | 39 | 24 | 28 |
| Obsidian | 88 | 6 | 39 | 34 | 9 |
| Heptabase | 65 | 5 | 35 | 13 | 12 |
| 合計 | 252 | 19 | 113 | 71 | 49 |

「已驗證」表示清冊該列指的本機工作流可從正常操作入口完成，並驗過相關的存檔、重開與資料邊界。「部分」表示已有可用的本機對應，列內會註明尚缺的行為。「未實作」表示目前沒有對應入口或資料模型。「外部服務」表示該項仍需外部服務。Canvas 的拖曳及與官方 Obsidian 的回匯證據沿用上一輪的合成樣本。[Canvas 操作證據](UI-ACCEPTANCE.md#canvas-拖曳與回匯)

前一批通過的 Vault 雙向協調、遷移報告、多行 PDF 與核取方塊都有本機證據。其對應列仍各有範圍缺口，包括自動同步、匯出中繼資料與可跳轉連結、複雜 PDF 排版，以及更廣的屬性和視圖能力，所以維持原列狀態。局部流程的檢查點不直接計入 252 列。

第 13 版候選補上日期欄範圍、多欄排序、Vault 標籤階層與改名追蹤，以及白板文字和 PDF 標註復原。[操作與驗收紀錄](UI-ACCEPTANCE.md#第-13-版候選的本機工作流)

第 15 版候選可拖曳日期條、在既有檢視修改狀態與標題過濾，並在核對 Vault 搬移時更新同批次的 Canvas 檔案參照。相鄰兩頁的 PDF 選區可放進同一張摘錄卡。相關列仍列為部分支援，原有跨白板摘錄卡的已驗證範圍見該列。[操作步驟](UI-ACCEPTANCE.md#第-15-版的時間軸vault-與雙頁摘錄)

第 22 版候選擴充白板排列、日期篩選與 Vault 未連結提及，也新增跨頁待辦總覽。這些工作流的驗證及限制另記在操作頁。[第 22 版操作](UI-ACCEPTANCE.md#第-22-版候選)

第 21 版候選加入白板圖片貼上及含圖的研究 Canvas ZIP、多層範本子頁、日期篩選和 Vault 標籤面板。[操作與驗收](UI-ACCEPTANCE.md#第-21-版候選)

第 20 版候選擴充資料列範本、Vault 搬移與白板卡庫。下表各列列出這次已可操作的範圍和仍缺的行為。[候選操作與界線](UI-ACCEPTANCE.md#第-20-版候選)

## 尚待落地的主要範圍

- **資料庫**：集合專屬欄位、日期相對區間的篩選、相依排程、多層分組、進階公式相容、範本、附件與屬性的版本歷史。
- **Vault**：變更監看、新檔／改名／附件的增量協調、外掛相容，以及帶設定與索引中繼資料的完整 Vault 往返。
- **白板及閱讀**：自由分區幾何、跨板巢狀結構、PDF 多欄或複雜排版文字、摘錄嵌入。

## Notion

| 功能 | 狀態 | 此版本的範圍 | 依據 |
|---|---|---|---|
| 筆記新增 | 已驗證 | 本機筆記樹與父子 ID。 | [官方](https://www.notion.com/help/create-your-first-page) · [store.rs](../src/store.rs) |
| 子頁 | 已驗證 | 本機筆記樹與父子 ID。 | [官方](https://www.notion.com/help/create-your-first-page) · [store.rs](../src/store.rs) |
| 移動頁面 | 已驗證 | 本機筆記樹與父子 ID。 | [官方](https://www.notion.com/help/create-your-first-page) · [store.rs](../src/store.rs) |
| 巢狀頁面導覽 | 已驗證 | 本機筆記樹與父子 ID。 | [官方](https://www.notion.com/help/create-your-first-page) · [store.rs](../src/store.rs) |
| 垃圾桶 | 已驗證 | 保留子樹、標星及板上位置。 | [官方](https://www.notion.com/help/duplicate-delete-and-restore-content) · [store.rs](../src/store.rs) |
| 還原筆記 | 已驗證 | 保留子樹、標星及板上位置。 | [官方](https://www.notion.com/help/duplicate-delete-and-restore-content) · [store.rs](../src/store.rs) |
| 整頁複製 | 部分 | 可複製單篇或使用中的子頁，保留正文、屬性和附件。關聯、白板、PDF 標註及版本歷史仍留在原筆記。 | [官方](https://www.notion.com/help/duplicate-delete-and-restore-content) · [duplicate_note.rs](../src/duplicate_note.rs) |
| 頁面歷史版本還原 | 部分 | 每篇可比較目前草稿或保存內容與選取的舊版，逐行預覽標題及正文差異，再還原已保存的版本。屬性、附件及頁面配置的歷史仍待做。 | [官方](https://www.notion.com/help/duplicate-delete-and-restore-content) · [操作步驟](UI-ACCEPTANCE.md#筆記版本差異與還原) |
| 區塊編輯 | 部分 | Markdown 原稿與獨立閱讀檢視；僅部分格式及命令。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 斜線選單 | 部分 | Markdown 原稿與獨立閱讀檢視；僅部分格式及命令。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 文字樣式 | 部分 | Markdown 原稿與獨立閱讀檢視；僅部分格式及命令。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 待辦清單 | 部分 | Markdown 原稿與獨立閱讀檢視；僅部分格式及命令。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 同步區塊 | 部分 | 多篇筆記可共用同一段帶 ID 的 Markdown，並從引用處定位、編輯原文或解除單一引用。任意富文字區塊、跨工作區和多人即時同步仍待做。 | [官方](https://www.notion.com/help/synced-blocks) · [synced_blocks.rs](../src/synced_blocks.rs) |
| 分欄 | 未實作 | 現有儲存單位是整篇筆記。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 折疊區塊 | 未實作 | 現有儲存單位是整篇筆記。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [markdown.rs](../src/markdown.rs) |
| 目錄 | 部分 | F8 可從紙間跳到本篇 Markdown 標題。離線 HTML 篇頁有可點擊的目錄，同名標題各有目的地。筆記內可插入的獨立目錄區塊仍待做。 | [官方](https://www.notion.com/help/columns-headings-and-dividers) · [操作步驟](UI-ACCEPTANCE.md#長筆記大綱與離線目錄) |
| 區塊 ID 嵌入 | 部分 | 同工作區可插入並顯示唯一 ID 的 Markdown 區塊。原廠區塊物件與匯入格式仍待做。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [操作步驟](UI-ACCEPTANCE.md#同步區塊vault-嵌入與日誌月曆) |
| 數學公式排版 | 未實作 | 資料公式與數學排版各自獨立。 | [官方](https://www.notion.com/help/math-equations) · [markdown.rs](../src/markdown.rs) |
| 連結其他頁 | 部分 | 支援紙間 ID 與已解析的 Wiki 連結。 | [官方](https://www.notion.com/help/create-links-and-backlinks) · [note_links.rs](../src/note_links.rs) |
| 反向連結 | 部分 | 支援紙間 ID 與已解析的 Wiki 連結。 | [官方](https://www.notion.com/help/create-links-and-backlinks) · [note_links.rs](../src/note_links.rs) |
| 附件 | 部分 | 從 Markdown ZIP 匯入包內能解析的相對附件，保留來源路徑和位元。支援類型及容量有限，PDF 可離線翻頁及標註。 | [官方](https://www.notion.com/help/images-files-and-media) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 圖片 | 部分 | 限定檔案類型與容量，PDF 可離線翻頁、畫框。 | [官方](https://www.notion.com/help/images-files-and-media) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| PDF | 部分 | 限定檔案類型與容量，PDF 可離線翻頁、畫框。 | [官方](https://www.notion.com/help/images-files-and-media) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 音訊播放 | 未實作 | 沒有媒體播放器或內嵌瀏覽器入口。 | [官方](https://www.notion.com/help/images-files-and-media) · [window.rs](../src/window.rs) |
| 影片播放器 | 未實作 | 沒有媒體播放器或內嵌瀏覽器入口。 | [官方](https://www.notion.com/help/images-files-and-media) · [window.rs](../src/window.rs) |
| 網頁嵌入 | 未實作 | 沒有媒體播放器或內嵌瀏覽器入口。 | [官方](https://www.notion.com/help/images-files-and-media) · [window.rs](../src/window.rs) |
| 工作區搜尋 | 部分 | 標題／正文搜尋、標星和頁樹。 | [官方](https://www.notion.com/help/navigate-with-the-sidebar) · [store.rs](../src/store.rs) |
| 側欄捷徑 | 部分 | 標題／正文搜尋、標星和頁樹。 | [官方](https://www.notion.com/help/navigate-with-the-sidebar) · [store.rs](../src/store.rs) |
| 收藏 | 部分 | 標題／正文搜尋、標星和頁樹。 | [官方](https://www.notion.com/help/navigate-with-the-sidebar) · [store.rs](../src/store.rs) |
| 最近瀏覽 | 未實作 | 未見對應瀏覽歷程或首頁設定。 | [官方](https://www.notion.com/help/navigate-with-the-sidebar) · [window.rs](../src/window.rs) |
| 可自訂首頁 | 未實作 | 未見對應瀏覽歷程或首頁設定。 | [官方](https://www.notion.com/help/navigate-with-the-sidebar) · [window.rs](../src/window.rs) |
| 自訂資料庫 | 部分 | 具名集合、成員與狀態。可把已存檢視設定套入其他集合，沿用目標集合自身的資料列。跨來源資料仍待做。 | [官方](https://www.notion.com/help/intro-to-databases) · [collections.rs](../src/collections.rs) |
| 資料列 | 部分 | 具名集合、成員與狀態，並非任意資料來源。 | [官方](https://www.notion.com/help/intro-to-databases) · [collections.rs](../src/collections.rs) |
| 資料來源 | 部分 | 具名集合、成員與狀態，並非任意資料來源。 | [官方](https://www.notion.com/help/intro-to-databases) · [collections.rs](../src/collections.rs) |
| 屬性模型 | 部分 | 集合內可寫入或清除已宣告的數字、核取方塊與日期，其他檢視會讀到相同的值。日期編輯會檢查既有起訖設定，下拉選項等型別仍待做。 | [官方](https://www.notion.com/help/database-properties) · [properties.rs](../src/properties.rs) |
| 文字欄位 | 已驗證 | 中文值可寫入、查詢、重開。 | [官方](https://www.notion.com/help/database-properties) · [properties.rs](../src/properties.rs) |
| 關聯 | 部分 | 從集合的可見資料列新增、移除或開啟同集合內的具名關聯。使用穩定筆記 ID，可辨認不同路徑的同名筆記。跨集合關聯欄位配置仍待做。 | [官方](https://www.notion.com/help/relations-and-rollups) · [rollups.rs](../src/rollups.rs) |
| 彙總 | 部分 | 集合內可從同一資料列操作入口查看關聯和彙總。公式可依具名關聯計數或合計最新欄位值，顯示來源有問題的筆記 ID。 | [官方](https://www.notion.com/help/relations-and-rollups) · [rollups.rs](../src/rollups.rs) |
| 公式運算 | 部分 | 可編輯既有集合公式並在保存後重算。支援數字／布林／日期型別值、條件和關聯計數／加總。錯誤顯示在結果，完整語法待做。 | [官方](https://www.notion.com/help/formulas) · [formula.rs](../src/formula.rs) |
| 看板 | 部分 | 具名看板可按條件篩選並在同一集合內移欄。可改名、複製檢視規則及編輯共用欄位。卡片欄位的自由排列待做。 | [官方](https://www.notion.com/help/boards) · [workspace_ui.rs](../src/workspace_ui.rs) |
| 分組 | 部分 | 集合表格與看板可依單一欄位分組並保存設定。多層分組待做。 | [官方](https://www.notion.com/help/boards) · [workspace_ui.rs](../src/workspace_ui.rs) |
| 表格 | 部分 | 集合表格可命名、複製檢視、保存篩選條件及從資料列開啟筆記。可從同一視圖編輯已宣告的型別欄位。行內任意欄位配置待做。 | [官方](https://www.notion.com/help/tables) · [集合檢視](../src/collection_views.rs) |
| 篩選 | 部分 | 具名檢視支援三層 AND／OR、關聯與彙總。日期欄及日期公式可依本機日期使用週、月及前後日數條件，條件和群組可在同層複製、前後移。時間戳及其他欄位型別待做。 | [官方](https://developers.notion.com/reference/filter-data-source-entries) · [本輪操作](UI-ACCEPTANCE.md#第-22-版候選) |
| 排序 | 部分 | 可保存最多 8 個排序欄及各自方向，依優先順序套用。空值和無效值置後，關聯與公式等衍生欄位排序待做。 | [官方](https://www.notion.com/help/views-filters-and-sorts) · [collection_views.rs](../src/collection_views.rs) |
| 資料庫範本 | 部分 | 可從現有列或空白建立範本，修改型別預設並撰寫多層 Markdown 子頁。可選根頁的關聯和附件。嵌入、子頁專屬資源與從現有筆記整棵擷取待做。 | [規格與文件](https://www.notion.com/help/database-templates) · [本輪操作](UI-ACCEPTANCE.md#第-21-版候選) |
| 重複範本 | 未實作 | 未實作範本引擎。 | [官方](https://www.notion.com/help/database-templates) · [window.rs](../src/window.rs) |
| 連結資料庫 | 未實作 | 集合與雙向連結不提供此行為。 | [官方](https://www.notion.com/help/data-sources-and-linked-databases) · [collections.rs](../src/collections.rs) |
| 跨來源視圖 | 未實作 | 集合與雙向連結不提供此行為。 | [官方](https://www.notion.com/help/data-sources-and-linked-databases) · [collections.rs](../src/collections.rs) |
| 儀表板 | 未實作 | 未提供這些對應的資料庫視圖或表單收件。 | [官方](https://www.notion.com/help/dashboards) · [window.rs](../src/window.rs) |
| 資料圖表 | 未實作 | 未提供這些對應的資料庫視圖或表單收件。 | [官方](https://www.notion.com/help/charts) · [window.rs](../src/window.rs) |
| 圖庫 | 未實作 | 未提供這些對應的資料庫視圖或表單收件。 | [官方](https://www.notion.com/help/galleries) · [window.rs](../src/window.rs) |
| 日曆視圖 | 部分 | 月曆與週曆可選開始及結束日期欄，跨日起訖包含首尾兩日。可在月曆入口編輯日期並反映至集合其他視圖。拖曳排程、時間與時區待做。 | [官方](https://www.notion.com/help/calendars) · [collection_views.rs](../src/collection_views.rs) |
| 時間軸 | 部分 | 可儲存水平月時間軸，依共用起訖日裁切跨月區間。點日期條開啟筆記，可拖條身平移或拉可見端點調整。相依排程與縮放待做。 | [官方](https://www.notion.com/help/timelines) · [collection_views.rs](../src/collection_views.rs) |
| 地圖視圖 | 未實作 | 未提供這些對應的資料庫視圖或表單收件。 | [官方](https://www.notion.com/help/maps) · [window.rs](../src/window.rs) |
| 清單視圖 | 部分 | 可儲存、改名或複製具名清單，保留各自的篩選及排序設定並共用資料列。可設定的欄位顯示與排列待做。 | [官方](https://www.notion.com/help/lists) · [collection_views.rs](../src/collection_views.rs) |
| 表單 | 未實作 | 未提供這些對應的資料庫視圖或表單收件。 | [官方](https://www.notion.com/help/forms) · [window.rs](../src/window.rs) |
| CSV 匯入 | 部分 | 支援有限本機格式；CSV 網址保留文字，未轉為遠端物件。 | [官方](https://www.notion.com/help/import-data-into-notion) · [archive.rs](../src/archive.rs) |
| Markdown 匯入 | 部分 | 支援有限本機格式；CSV 網址保留文字，未轉為遠端物件。 | [官方](https://www.notion.com/help/import-data-into-notion) · [archive.rs](../src/archive.rs) |
| Notion ZIP 匯入 | 部分 | 保留 Markdown 階層、可解析的包內附件及原始 CSV。CSV 可逐位元另存。關聯網址維持文字，原服務的活關聯和公式需另行建立。 | [官方](https://www.notion.com/help/import-data-into-notion) · [archive.rs](../src/archive.rs) |
| 內容匯出 | 部分 | 單頁 Markdown、工作區 SQLite、附件、Canvas、指定 Vault ZIP，以及筆記與附件 ZIP。 | [官方](https://www.notion.com/help/export-your-content) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| 工作區 HTML 匯出 | 部分 | 可匯出活動筆記、階層導覽、原始 Markdown 與附件，解壓後在瀏覽器離線讀取。公式、白板、垃圾桶、原廠區塊排版及帶錨點連結仍留在各自資料格式。 | [官方](https://www.notion.com/help/export-your-content) · [html_export.rs](../src/html_export.rs) |
| 排版 PDF 匯出 | 未實作 | 現有工作區備份格式為 SQLite。 | [官方](https://www.notion.com/help/export-your-content) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| 離線工作區 | 部分 | 可攜 Windows EXE，SQLite 在本機；無廠商離線快取同步。 | [官方](https://www.notion.com/help/use-pages-offline) · [UI-ACCEPTANCE.md](../docs/UI-ACCEPTANCE.md) |
| 動作按鈕 | 未實作 | 未提供自訂觸發及工作流程。 | [官方](https://www.notion.com/help/database-buttons) · [window.rs](../src/window.rs) |
| 本機資料自動化 | 未實作 | 未提供自訂觸發及工作流程。 | [官方](https://www.notion.com/help/database-buttons) · [window.rs](../src/window.rs) |
| 來賓 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 成員群組 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 頁面分享權限 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 多人即時共編 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 遠端評論 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 通知收件匣 | 外部服務 | 需身分、遠端傳遞或共同工作區。 | [官方](https://www.notion.com/help/sharing-and-permissions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 公開網站 | 外部服務 | 依賴託管、域名或平台帳戶。 | [官方](https://www.notion.com/help/edit-and-customize-your-notion-sites) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 自訂網域 | 外部服務 | 依賴託管、域名或平台帳戶。 | [官方](https://www.notion.com/help/edit-and-customize-your-notion-sites) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 公開範本市集 | 外部服務 | 依賴託管、域名或平台帳戶。 | [官方](https://www.notion.com/help/edit-and-customize-your-notion-sites) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Notion API | 外部服務 | 需相應服務與授權。 | [官方](https://www.notion.com/help/create-integrations-with-the-notion-api) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 外部整合 | 外部服務 | 需相應服務與授權。 | [官方](https://www.notion.com/help/create-integrations-with-the-notion-api) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Webhook | 外部服務 | 需相應服務與授權。 | [官方](https://www.notion.com/help/webhook-actions) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 外部代理 | 外部服務 | 需相應服務與授權。 | [官方](https://www.notion.com/help/create-integrations-with-the-notion-api) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Notion AI | 外部服務 | 這些廠商服務並未整合進紙間。 | [官方](https://www.notion.com/help/notion-ai-connectors) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| AI 會議筆記 | 外部服務 | 這些廠商服務並未整合進紙間。 | [官方](https://www.notion.com/help/ai-meeting-notes) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 企業搜尋 | 外部服務 | 這些廠商服務並未整合進紙間。 | [官方](https://www.notion.com/help/notion-ai-connectors) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| AI 連接器 | 外部服務 | 這些廠商服務並未整合進紙間。 | [官方](https://www.notion.com/help/notion-ai-connectors) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 自訂代理 | 外部服務 | 這些廠商服務並未整合進紙間。 | [官方](https://www.notion.com/help/notion-ai-connectors) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| SAML | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/domain-management) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| SCIM | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/provision-users-and-groups-with-scim) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 域名管理 | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/domain-management) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 企業稽核 | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/audit-log) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 資料區域 | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/data-residency) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 帳務 | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/domain-management) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 訂閱管理 | 外部服務 | 依賴組織身分及後端管理。 | [官方](https://www.notion.com/help/domain-management) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Notion Calendar 帳戶 | 外部服務 | 需第三方帳戶或伺服器。 | [官方](https://www.notion.com/help/notion-calendar-connections) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 外部行事曆 | 外部服務 | 需第三方帳戶或伺服器。 | [官方](https://www.notion.com/help/notion-calendar-connections) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Notion Mail | 外部服務 | 需第三方帳戶或伺服器。 | [官方](https://www.notion.com/help/notion-calendar-connections) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 頁面版面配置 | 未實作 | 現有資料集合未提供該視圖。 | [官方](https://www.notion.com/help/layouts) · [介面驗收](UI-ACCEPTANCE.md) |
| 資料列相依性 | 未實作 | 現有資料集合未提供該視圖。 | [官方](https://www.notion.com/help/tasks-and-dependencies) · [介面驗收](UI-ACCEPTANCE.md) |
| 桌面分頁 | 未實作 | 目前使用單一主視窗。 | [官方](https://www.notion.com/help/new-tabs-in-the-notion-desktop-app) · [介面驗收](UI-ACCEPTANCE.md) |
| 行動裝置小工具 | 未實作 | 紙間沒有行動版。 | [官方](https://www.notion.com/help/mobile-widgets) · [介面驗收](UI-ACCEPTANCE.md) |
| 自訂頁面圖示與封面 | 未實作 | 頁面儲存沒有自訂封面。 | [官方](https://www.notion.com/help/customize-and-style-your-content) · [介面驗收](UI-ACCEPTANCE.md) |
| 離線 Windows 桌面執行 | 已驗證 | Windows EXE 已在只有 loopback 的 Wine 客體啟動；原生斷網 Windows 待驗。 | [官方](https://www.notion.com/help/use-pages-offline) · [介面驗收](UI-ACCEPTANCE.md) |

## Obsidian

| 功能 | 狀態 | 此版本的範圍 | 依據 |
|---|---|---|---|
| Vault | 部分 | 匯入為本機筆記樹，已配對的 Markdown 可手動批次讀入或寫回。具差異預覽、備份及衝突保留兩份。已配對的附件可手動更新同一路徑的磁碟內容。外部已完成搬移時，可核對精確來源路徑並在備份後更新已驗證的連結。自動監看、新檔處理、磁碟檔案操作及已標註 PDF 更新待做。 | [官方](https://obsidian.md/help/manage-vaults) · [vault.rs](../src/vault.rs) |
| 檔案總管 | 部分 | 可按匯入批次逐層瀏覽保存的 Vault 原始相對路徑，選到檔案後從工作區開啟，來源磁碟離線也可使用。手動核對的外部搬移可更新原始相對路徑。自動偵測及原地檔案管理仍待做。 | [官方](https://obsidian.md/help/plugins/file-explorer) · [vault_search_ui.rs](../src/vault_search_ui.rs) |
| 資料夾 | 部分 | 匯入與 ZIP 匯出保存中文巢狀路徑。含來源附件的 ZIP 解壓後可回匯為獨立批次。未保存的空資料夾不在匯出範圍。 | [官方](https://obsidian.md/help/manage-vaults) · [資料夾與附件往返](UI-ACCEPTANCE.md#vault-資料夾與附件往返) |
| 原地編輯 Markdown | 已驗證 | 明確選取本機 UTF-8 檔、編輯並寫回同檔。外部改檔拒寫，保留草稿；正常存檔先留舊版。 | [官方](https://obsidian.md/help/manage-vaults) · [介面驗收](UI-ACCEPTANCE.md#型別公式vault-原檔與白板標註來源) |
| 附件 | 部分 | 匯入保留可解析附件的相對來源及內容。可隨 Vault ZIP 移轉具有明確路徑的來源附件，匯出前核對回匯配對。另有同一路徑的新內容更新功能；缺檔、衝突或會影響既有 PDF 標註時會拒絕更新。 | [官方](https://obsidian.md/help/attachments) · [資料夾與附件往返](UI-ACCEPTANCE.md#vault-資料夾與附件往返) |
| Markdown | 部分 | 支援基本標題、粗體、引用、清單與程式碼等語法。 | [官方](https://obsidian.md/help/syntax) · [markdown.rs](../src/markdown.rs) |
| 閱讀檢視 | 部分 | 支援基本標題、粗體、引用、清單與程式碼等語法。 | [官方](https://obsidian.md/help/syntax) · [markdown.rs](../src/markdown.rs) |
| 原始碼編輯 | 部分 | 支援基本標題、粗體、引用、清單與程式碼等語法。 | [官方](https://obsidian.md/help/syntax) · [markdown.rs](../src/markdown.rs) |
| 待辦 | 部分 | 支援基本標題、粗體、引用、清單與程式碼等語法。 | [官方](https://obsidian.md/help/syntax) · [markdown.rs](../src/markdown.rs) |
| Live Preview | 未實作 | 未核得等效操作。 | [官方](https://obsidian.md/help/advanced-syntax) · [markdown.rs](../src/markdown.rs) |
| Callout | 部分 | 閱讀畫面可呈現五種標準 Callout。原 Markdown 照原文保存，折疊、自訂類型與圖示待做。 | [官方](https://obsidian.md/help/callouts) · [markdown.rs](../src/markdown.rs) |
| 數學排版 | 未實作 | 未核得等效操作。 | [官方](https://obsidian.md/help/advanced-syntax) · [markdown.rs](../src/markdown.rs) |
| 多游標 | 未實作 | 未核得等效操作。 | [官方](https://obsidian.md/help/multiple-cursors) · [markdown.rs](../src/markdown.rs) |
| Markdown 摺疊 | 未實作 | 未核得等效操作。 | [官方](https://obsidian.md/help/advanced-syntax) · [markdown.rs](../src/markdown.rs) |
| 自訂圖示 | 未實作 | 未核得等效操作。 | [官方](https://obsidian.md/help/advanced-syntax) · [markdown.rs](../src/markdown.rs) |
| WikiLinks | 部分 | 外部工具已搬移的 Vault 筆記，若相對路徑改寫後仍唯一指向同一個筆記，可在備份後採用。保留別名和區塊 ID。多義連結、正文不同或無法核對時會停止。自動原地維護待做。 | [官方](https://obsidian.md/help/links) · [第 20 版操作](UI-ACCEPTANCE.md#第-20-版候選) |
| 標題跳轉 | 部分 | 可從同篇或指定 Vault 路徑的連結跳到標題。父子標題鏈可區分同名小節。Markdown 支援百分比編碼。重名無法唯一定位時會提示。原廠桌面對照待完成。 | [官方](https://obsidian.md/help/links) · [筆記連結](../src/note_links.rs) |
| 顯示別名 | 部分 | Vault 路徑消歧、別名及唯一標題段落；選單呈現段落。 | [官方](https://obsidian.md/help/links) · [note_links.rs](../src/note_links.rs) |
| YAML aliases 作為查找索引 | 部分 | 匯入 Vault 的來源搜尋可比對常見 YAML 別名。可用唯一別名找未連結提及，原始碼編輯器的自動連結建議仍待做。 | [官方](https://obsidian.md/help/aliases) · [vault_search.rs](../src/vault_search.rs) |
| 區塊引用 | 部分 | 可搜尋並複製唯一區塊的工作區連結，預覽後跳到區塊，再按 F9 返回。傳入連結報告可檢視同批次來源與重複或遺失的 ID。已支援同一批次內的文字區塊嵌入和反向追查。自動產生區塊 ID 及原 Vault 檔的即時偵測仍待做。 | [官方](https://obsidian.md/help/links) · [第 20 版操作](UI-ACCEPTANCE.md#第-20-版候選) |
| 頁面嵌入 | 部分 | 同一批次的整篇 Markdown、標題段落及唯一區塊可在閱讀模式嵌入。媒體或插件提供的嵌入仍待做。 | [官方](https://obsidian.md/help/embeds) · [transclusion.rs](../src/transclusion.rs) |
| 明確反向連結 | 已驗證 | 由已解析的筆記參照列出。 | [官方](https://obsidian.md/help/plugins/backlinks) · [note_links.rs](../src/note_links.rs) |
| 明確傳出連結 | 已驗證 | 由已解析的筆記參照列出。 | [官方](https://obsidian.md/help/plugins/outgoing-links) · [note_links.rs](../src/note_links.rs) |
| 圖譜 | 部分 | 局部、工作區和匯入 Vault 三種範圍，已解析的邊可查有向最短路徑，並以節點返回筆記。 | [官方](https://obsidian.md/help/plugins/graph) · [note_links.rs](../src/note_links.rs) |
| 未連結提及 | 部分 | 可在同一匯入批次用唯一名稱或別名找未連結提及，可加來源完整標籤，確認後建立工作區連結。持續顯示、文字篩選、排序及外部 Vault 連結格式待做。 | [官方](https://obsidian.md/help/plugins/backlinks) · [本輪操作](UI-ACCEPTANCE.md#第-22-版候選) |
| 搜尋 | 部分 | 標題／正文搜尋，以及已匯入 Vault 的資料夾、別名和標籤交集搜尋，可開啟精確來源。完整查詢語言待做。 | [官方](https://obsidian.md/help/plugins/search) · [vault_search.rs](../src/vault_search.rs) |
| 書籤 | 部分 | 標題／正文搜尋與標星；無完整查詢語言。 | [官方](https://obsidian.md/help/plugins/bookmarks) · [store.rs](../src/store.rs) |
| 快捷鍵 | 部分 | 標題／正文搜尋與標星；無完整查詢語言。 | [官方](https://obsidian.md/help/plugins/search) · [store.rs](../src/store.rs) |
| 命令面板 | 部分 | F6 搜尋並執行五項內建導覽命令。全應用程式命令、可設定或擴充命令待做。 | [官方](https://obsidian.md/help/plugins/command-palette) · [navigation_ui.rs](../src/navigation_ui.rs) |
| 快速切換器 | 部分 | F7 依標題、來源路徑或完整別名打開指定筆記，也可用 tag:#父層 找含子標籤的匯入筆記。模糊排序、快速新建及新分頁待做。 | [官方](https://obsidian.md/help/plugins/quick-switcher) · [navigation_ui.rs](../src/navigation_ui.rs) |
| 大綱 | 部分 | F8 依標題層級與行號列出已保存內容或目前草稿，可跳到同名標題的精確位置。拖曳重新排序段落待做。 | [官方](https://obsidian.md/help/plugins/outline) · [navigation_ui.rs](../src/navigation_ui.rs) |
| 懸浮頁面預覽 | 未實作 | 沒有上述獨立入口。 | [官方](https://obsidian.md/help/plugins/page-preview) · [window.rs](../src/window.rs) |
| JSON Canvas 匯入與匯出 | 部分 | 保留未知欄位，可修改既有節點。紙間研究 ZIP 可攜帶普通筆記及圖片，以標準檔案節點連同附件匯出，並在乾淨紙間工作區回匯。若重新匯入 ZIP 會把原先未確定的路徑配成特定附件，或配到不同附件，紙間會拒絕輸出。畫布也可平移、以游標縮放，並保存各自的視野。一般 Canvas 附件交換和未支援結構仍有缺口。 | [規格與文件](https://jsoncanvas.org/spec/1.0/) · [Canvas 操作](UI-ACCEPTANCE.md#canvas-平移縮放與重開) |
| 文字節點 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 檔案節點 | 部分 | 核對 Vault 搬移時，可在同一交易更新綁定該批次的檔案節點。保留原節點 ID、子路徑和未知欄位。原磁碟檔由外部工具管理。自動監聽待做。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 連結節點 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 群組節點 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 邊 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 節點幾何 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 節點編輯 | 部分 | 保留未知欄位；修改後重新序列化。支援部分原生操作，預覽有繪製上限。 | [官方](https://obsidian.md/help/plugins/canvas) · [canvas_edit.rs](../src/canvas_edit.rs) |
| 屬性 | 部分 | 本機文字屬性；匯入的 YAML 保留在原文。 | [官方](https://obsidian.md/help/plugins/properties) · [properties.rs](../src/properties.rs) |
| 屬性檢視 | 部分 | 本機文字屬性；匯入的 YAML 保留在原文。 | [官方](https://obsidian.md/help/plugins/properties) · [properties.rs](../src/properties.rs) |
| 標籤面板 | 已驗證 | 可按名稱或頻率排序並切換平面或樹狀。可展開或收合單一分支及全部分支。可從標籤選單切換查詢條件並開啟正確來源，已驗同名筆記及離線重開。 | [規格與文件](https://obsidian.md/help/plugins/tags) · [本輪操作](UI-ACCEPTANCE.md#第-21-版候選) |
| 巢狀標籤 | 已驗證 | 匯入 Vault 可查完全相符或含子層的標籤，能辨識內文與常見 YAML tags 欄，並從標籤樹開到原筆記。其他 YAML 語法及原廠完整查詢語言另見搜尋列。 | [官方](https://obsidian.md/help/plugins/tags) · [vault_search.rs](../src/vault_search.rs) |
| .base 格式 | 未實作 | 本機集合、公式與看板不讀寫 .base。 | [官方](https://obsidian.md/help/bases) · [collections.rs](../src/collections.rs) |
| Bases 視圖 | 未實作 | 本機集合、公式與看板不讀寫 .base。 | [官方](https://obsidian.md/help/bases) · [collections.rs](../src/collections.rs) |
| Bases 公式語法 | 未實作 | 本機集合、公式與看板不讀寫 .base。 | [官方](https://obsidian.md/help/bases) · [collections.rs](../src/collections.rs) |
| 每日筆記 | 已驗證 | 日期識別碼跨改名、關閉、備份維持。 | [官方](https://obsidian.md/help/plugins/daily-notes) · [store.rs](../src/store.rs) |
| Templates | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/web-clipper/templates) · [window.rs](../src/window.rs) |
| Random Note | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/random-note) · [window.rs](../src/window.rs) |
| Slides | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/slides) · [window.rs](../src/window.rs) |
| Word Count | 部分 | 可查看目前筆記的 Markdown 正文字數和非空白字元數，略過程式碼。即時狀態列計數待做。 | [官方](https://obsidian.md/help/plugins/word-count) · [note_text_tools.rs](../src/note_text_tools.rs) |
| Audio Recorder | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/audio-recorder) · [window.rs](../src/window.rs) |
| Workspaces | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/workspaces) · [window.rs](../src/window.rs) |
| Note Composer | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/note-composer) · [window.rs](../src/window.rs) |
| Unique Note | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/unique-note) · [window.rs](../src/window.rs) |
| Format Converter | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/format-converter) · [window.rs](../src/window.rs) |
| File Recovery | 部分 | 紙間工作區可保存、預覽及還原筆記文字。Vault 檔案與 Canvas 的獨立檔案快照待做。 | [官方](https://obsidian.md/help/plugins/file-recovery) · [note_history.rs](../src/note_history.rs) |
| Footnotes 外掛 | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/footnotes) · [window.rs](../src/window.rs) |
| Web viewer | 未實作 | 沒有相應核心外掛入口。 | [官方](https://obsidian.md/help/plugins/web-viewer) · [window.rs](../src/window.rs) |
| 主題 | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/themes) · [window.rs](../src/window.rs) |
| CSS snippets | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/appearance) · [window.rs](../src/window.rs) |
| 擴充套件 API | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/appearance) · [window.rs](../src/window.rs) |
| 社群外掛 | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/community-plugins) · [window.rs](../src/window.rs) |
| URI | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/uri) · [window.rs](../src/window.rs) |
| CLI | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/cli) · [window.rs](../src/window.rs) |
| 多分頁 | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/tabs) · [window.rs](../src/window.rs) |
| 彈出視窗 | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/pop-out-windows) · [window.rs](../src/window.rs) |
| 行動版 | 未實作 | Windows 應用未提供上述相容層或平台版本。 | [官方](https://obsidian.md/help/mobile) · [window.rs](../src/window.rs) |
| 其他工具匯入 | 部分 | 限支援的 Markdown、資料夾、CSV 與 ZIP；未實作各廠商完整轉換。 | [官方](https://obsidian.md/help/import) · [archive.rs](../src/archive.rs) |
| Obsidian Sync | 外部服務 | 需官方帳戶或雲端服務。 | [官方](https://obsidian.md/help/teams/sync) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 遠端版本歷史 | 外部服務 | 需官方帳戶或雲端服務。 | [官方](https://obsidian.md/help/teams/sync) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Teams | 外部服務 | 需官方帳戶或雲端服務。 | [官方](https://obsidian.md/help/teams/sync) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 跨裝置分享 | 外部服務 | 需官方帳戶或雲端服務。 | [官方](https://obsidian.md/help/teams/sync) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Publish | 外部服務 | 需託管服務。 | [官方](https://obsidian.md/help/teams/publish) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 公開站點 | 外部服務 | 需託管服務。 | [官方](https://obsidian.md/help/teams/publish) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| SEO | 外部服務 | 需託管服務。 | [官方](https://obsidian.md/help/teams/publish) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 自訂網域 | 外部服務 | 需託管服務。 | [官方](https://obsidian.md/help/teams/publish) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 站點流量分析 | 外部服務 | 需託管服務。 | [官方](https://obsidian.md/help/teams/publish) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Web Clipper | 未實作 | 未提供瀏覽器擴充套件。 | [官方](https://obsidian.md/help/web-clipper) · [window.rs](../src/window.rs) |
| Markdown 表格工具 | 部分 | 可保留原稿；未驗證全部 Markdown 表格編輯操作。 | [官方](https://obsidian.md/help/advanced-syntax) · [介面驗收](UI-ACCEPTANCE.md) |
| 分割工作區布局 | 未實作 | 未提供 Obsidian 工作區序列化或分割窗格。 | [官方](https://obsidian.md/help/workspace) · [介面驗收](UI-ACCEPTANCE.md) |
| 檔案格式與附件放置位置 | 部分 | 支援限定匯入及附件副本，資料庫位置由紙間選擇。 | [官方](https://obsidian.md/help/file-formats) · [介面驗收](UI-ACCEPTANCE.md) |
| Web Clipper 樣板 | 未實作 | 未提供可安裝的擷取擴充套件。 | [官方](https://obsidian.md/help/web-clipper) · [介面驗收](UI-ACCEPTANCE.md) |
| 尋找與取代 | 部分 | 可在目前筆記區分大小寫循環尋找，預覽並確認全部取代。逐筆取代與正規表示式待做。 | [官方](https://obsidian.md/help/find-and-replace) · [介面驗收](UI-ACCEPTANCE.md) |
| Bases 卡片視圖 | 未實作 | 紙間檔案交換不支援 .base。 | [官方](https://obsidian.md/help/bases/views/cards) · [介面驗收](UI-ACCEPTANCE.md) |
| Bases 地圖視圖 | 未實作 | 未實作地理位置欄位。 | [官方](https://obsidian.md/help/bases/views/map) · [介面驗收](UI-ACCEPTANCE.md) |
| Bases 清單視圖 | 未實作 | 未讀取或呈現 .base 清單。 | [官方](https://obsidian.md/help/bases/views/list) · [介面驗收](UI-ACCEPTANCE.md) |

## Heptabase

| 功能 | 狀態 | 此版本的範圍 | 依據 |
|---|---|---|---|
| 筆記卡 | 部分 | 筆記可加入多個白板，另有篩選、關聯及搜尋。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [collections.rs](../src/collections.rs) |
| 卡片資料庫 | 部分 | 可預覽及開啟普通筆記卡和 PDF 摘錄卡，批次加入白板或標星，保留原卡 ID 和既有位置。獨立的圖片、音訊與影片卡等類型待做。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [第 20 版操作](UI-ACCEPTANCE.md#第-20-版候選) |
| 卡片篩選 | 部分 | 兩種卡片可篩文字、完整標籤、類型、標星和目前白板放置狀態。PDF 摘錄另有附件 ID 和頁碼。屬性、日期及保存篩選條件待做。 | [官方](https://wiki.heptabase.com/changelog) · [第 20 版操作](UI-ACCEPTANCE.md#第-20-版候選) |
| 相關卡片 | 部分 | 筆記可加入多個白板，另有篩選、關聯及搜尋。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [collections.rs](../src/collections.rs) |
| 白板 | 部分 | 可將筆記卡、圖片和文字依實際尺寸對齊與等距排列，重開後可復原最近一次排列。自由分區幾何與其他物件仍有缺口。 | [官方](https://support.heptabase.com/en/articles/17060792-heptabase-mcp-tool-reference) · [本輪操作](UI-ACCEPTANCE.md#第-22-版候選) |
| 白板獨立矩形與橢圓 | 部分 | 草稿版可新增、直接拖曳移動、以數值調整位置與尺寸，以及刪除指定形狀。原生 Windows 介面驗收及原廠手勢、樣式與匯入相容仍待核對。研究 ZIP 會拒絕匯出含形狀的白板，請用工作區備份。 | [官方 v1.118.0](https://wiki.heptabase.com/changelog#v11180) · [board_shape_tag_ui.rs](../src/board_shape_tag_ui.rs) |
| 白板標籤 | 部分 | 草稿版可新增、移除白板本身的標籤，並以完整標籤找白板，或一併找指定 Vault 的筆記。原生 Windows 介面驗收及原廠匯出匯入仍待核對。研究 ZIP 會拒絕匯出含白板標籤的白板，請用工作區備份。 | [官方 v1.118.0](https://wiki.heptabase.com/changelog#v11180) · [board_tags.rs](../src/board_tags.rs) |
| 分區 | 部分 | 可重新命名、整組移動可見成員、收合及展開，並打開來源筆記或 PDF。自由分區幾何待做。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [boards.rs](../src/boards.rs) |
| 連線 | 部分 | 同白板連線連到卡片 ID，分區平移後端點仍連到原卡片。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [boards.rs](../src/boards.rs) |
| 卡片拖曳 | 部分 | 同板可拖曳多選物件。跨板可把選取的筆記或 PDF 摘錄卡拖至已指定的目的區，再確認位移。保留相對位置、組內連線及完整分區。文字方塊與子白板尚不支援跨板。 | [官方](https://support.heptabase.com/en/articles/17060792-heptabase-mcp-tool-reference) · [白板操作](UI-ACCEPTANCE.md#第-18-版的資料列範本區塊返回與跨板拖曳) |
| 跨白板重用卡片 | 已驗證 | 可將多張現有筆記卡及 PDF 摘錄卡一起放到另一張白板。兩張板共用原卡片內容和來源，位置各自保存。重複放入保留原位。離線重開後，從兩張板都能看到同一次編輯。 | [官方](https://wiki.heptabase.com/how-Heptabases-founder-use-Heptabase-for-learning) · [白板操作](UI-ACCEPTANCE.md#第-16-版的篩選標題與跨板卡片) |
| 巢狀子白板 | 已驗證 | 白板可建立、拖曳並開啟子白板，顯示並跳至目前板的任一上層。變更上層後仍保存內容。會拒絕循環與超過 16 層。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [board_objects.rs](../src/board_objects.rs) |
| 心智圖節點 | 部分 | 本機白板可建立根及子節點、編輯、拖曳、改掛父節點。可選取分支一起移動，或按 JSON Canvas 規格輸出文字、位置及箭頭。重開及工作區備份保留可編輯分支。原廠手勢、樣式及分支刪除仍有差異。 | [官方](https://wiki.heptabase.com/use-mindmap-nodes-as-a-substitute-for-text-elements) · [mind_map.rs](../src/mind_map.rs) |
| 卡片內嵌 | 未實作 | 本機筆記白板不提供這些物件，JSON Canvas 為獨立視圖。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [boards.rs](../src/boards.rs) |
| 自由文字物件 | 已驗證 | 白板可新增、移動、編輯及移除獨立文字方塊。重開後可復原最近移除的一個，原位有重疊時改放空位。每個物件限 4,096 字。 | [官方](https://wiki.heptabase.com/fundamental-elements) · [board_objects.rs](../src/board_objects.rs) |
| 內部連結 | 部分 | 紙間有限的固定命令與筆記導覽。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [note_links.rs](../src/note_links.rs) |
| 反向連結 | 部分 | 紙間有限的固定命令與筆記導覽。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [note_links.rs](../src/note_links.rs) |
| 快捷鍵 | 部分 | 紙間有限的固定命令與筆記導覽。 | [官方](https://wiki.heptabase.com/keyboard-shortcuts) · [note_links.rs](../src/note_links.rs) |
| 全域搜尋 | 部分 | 可跨工作區搜尋 PDF 引文、註解、來源名稱與頁碼，也可依摘錄卡名稱、原引文或原註解開啟卡片，或將同一張卡加入目前白板。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [pdf_search_ui.rs](../src/pdf_search_ui.rs) |
| 側欄工具 | 部分 | 紙間有限的固定命令與筆記導覽。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [note_links.rs](../src/note_links.rs) |
| 並排窗格 | 未實作 | 尚無對應操作。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [window.rs](../src/window.rs) |
| 多分頁 | 未實作 | 尚無對應操作。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [window.rs](../src/window.rs) |
| 命令面板 | 部分 | 有本機導覽命令與鍵盤入口。完整白板和卡片動作待做。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [navigation_ui.rs](../src/navigation_ui.rs) |
| 卡片最近紀錄 | 未實作 | 尚無對應操作。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [window.rs](../src/window.rs) |
| 文字屬性 | 部分 | 集合狀態分組與單一文字屬性查詢。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [workspace_ui.rs](../src/workspace_ui.rs) |
| 表格 | 部分 | 集合狀態分組與單一文字屬性查詢。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [workspace_ui.rs](../src/workspace_ui.rs) |
| 分欄看板 | 部分 | 集合狀態分組與單一文字屬性查詢。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [workspace_ui.rs](../src/workspace_ui.rs) |
| 多類型標籤屬性 | 未實作 | 資料模型只支援文字與具名關聯。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [properties.rs](../src/properties.rs) |
| 通用欄位配置 | 未實作 | 資料模型只支援文字與具名關聯。 | [官方](https://wiki.heptabase.com/organize-knowledge-and-projects) · [properties.rs](../src/properties.rs) |
| 每日卡 | 部分 | 可開日期筆記、編寫 Markdown 待辦。 | [官方](https://wiki.heptabase.com/build-a-journal-calendar-whiteboard-in-heptabase) · [store.rs](../src/store.rs) |
| 日誌 | 部分 | 可開日期筆記、編寫 Markdown 待辦。 | [官方](https://wiki.heptabase.com/build-a-journal-calendar-whiteboard-in-heptabase) · [store.rs](../src/store.rs) |
| 任務 | 部分 | 可開日期筆記、編寫 Markdown 待辦。 | [官方](https://wiki.heptabase.com/build-a-journal-calendar-whiteboard-in-heptabase) · [store.rs](../src/store.rs) |
| 日誌月曆 | 部分 | 用本機月曆開啟或建立當日日誌，標示未完成待辦及垃圾桶日期。原廠以白板排列日期與卡片的方式仍待做。 | [官方](https://wiki.heptabase.com/build-a-journal-calendar-whiteboard-in-heptabase) · [journal_ui.rs](../src/journal_ui.rs) |
| 跨頁任務匯總 | 已驗證 | 可在工作區、白板或單篇筆記找已完成與未完成待辦，跳至原文或切換勾選。重開後保留狀態，匯入副本有獨立狀態。來源變更會拒絕套用舊結果。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [本輪操作](UI-ACCEPTANCE.md#第-22-版候選) |
| 到期提醒 | 未實作 | 沒有對應日曆或任務排程。 | [官方](https://wiki.heptabase.com/build-a-journal-calendar-whiteboard-in-heptabase) · [window.rs](../src/window.rs) |
| PDF 閱讀 | 部分 | 本機點陣頁面與正規化矩形；已記錄 Wine 驗收及限制。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 頁碼摘錄 | 部分 | 本機點陣頁面與正規化矩形；已記錄 Wine 驗收及限制。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 矩形區域批註 | 部分 | 本機點陣頁面與正規化矩形；已記錄 Wine 驗收及限制。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 摘錄回跳 | 部分 | 工作區搜尋、白板釘選及獨立摘錄卡可回到原 PDF 和選區。卡庫可預覽保存的引文與來源狀態。雙頁卡可切換兩頁。已移除的單頁標註另支援復原。非 PDF 來源待做。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 文字字元級螢光標記 | 部分 | 可拖選同頁的可辨識左至右文字，逐行保存位置並於重開後回跳。版面與繪製順序一致的規則雙欄可跨欄。可逐頁選取同一份 PDF 的相鄰兩頁，再一次保存並互跳。掃描頁、複雜排版及連續拖曳跨頁待做。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 自動擷取選區文字 | 部分 | 可在支援的單欄或規則雙欄依閱讀順序擷取逐行引文。複製或保存時保留換行。相鄰兩頁各自選取後可依頁序合併複製。掃描頁、旋轉文字及無法可靠解碼的文字待做。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| Highlight Card 跨白板回鏈 | 已驗證 | 同一張獨立 PDF 摘錄卡可放兩張白板。跨程序重開後，兩處都返回同一 PDF、頁碼及選區，也能從來源標註開回同一張卡。可從白板搜尋並放入既有摘錄卡，重複放入時保留原座標。移除一處放置或刪除來源標註後，卡片正文仍保留。若原 PDF 可讀，也能回到保存的選區。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [驗收](UI-ACCEPTANCE.md#月曆vault-來源搜尋與跨板摘錄) |
| OCR | 未實作 | 未實作此 PDF 文字選取與卡片模型。 | [官方](https://wiki.heptabase.com/pdf-annotation) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 研究素材 | 部分 | 可存檔及複製出受支援格式。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [pdf_view_ui.rs](../src/pdf_view_ui.rs) |
| 圖片附件 | 部分 | 可貼上剪貼簿像素或本機 PNG／JPEG，定位、縮放及連線。研究 ZIP 的 Canvas 圖片路徑保留繁中檔名，原位元保留。原廠桌面對照及其他圖檔格式待驗或待做。 | [官方](https://jsoncanvas.org/spec/1.0/) · [本輪操作](UI-ACCEPTANCE.md#第-22-版候選) |
| 電子書閱讀 | 未實作 | 目前無相容入口。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| 影音逐字稿 | 未實作 | 目前無相容入口。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| 網頁擷取 | 未實作 | 目前無相容入口。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| Zotero 匯入 | 部分 | 支援 Zotero CSL JSON 書目元資料，包括同名項目、作者、日期與 DOI。原始檔可逐位元匯出。Zotero 收藏庫、PDF、標註及服務同步仍待支援。 | [Zotero](https://www.zotero.org/support/dev/data_formats) · [zotero.rs](../src/zotero.rs) |
| Heptabase 備份格式還原 | 未實作 | 目前無相容入口。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [QUICKSTART.md](../docs/QUICKSTART.md) |
| 逐卡版本歷史 | 部分 | 筆記卡可查看本機標題與正文的舊版本。其他物件或附件歷史待做。 | [官方](https://support.heptabase.com/en/articles/10448124-restore-from-version-history) · [note_history.rs](../src/note_history.rs) |
| 即時協作 | 外部服務 | 需線上服務。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 評論通知 | 外部服務 | 需線上服務。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 分享白板 | 外部服務 | 需線上服務。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 跨裝置同步 | 外部服務 | 需線上服務。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 帳戶權限 | 外部服務 | 需線上服務。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| AI 聊天 | 外部服務 | 需相應模型、服務或外部帳戶；候選未啟用。 | [官方](https://wiki.heptabase.com/work-with-ai) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| AI Tutor | 外部服務 | 需相應模型、服務或外部帳戶；候選未啟用。 | [官方](https://wiki.heptabase.com/work-with-ai) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 跨卡 AI 搜尋 | 外部服務 | 需相應模型、服務或外部帳戶；候選未啟用。 | [官方](https://wiki.heptabase.com/work-with-ai) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| AI 動作 | 外部服務 | 需相應模型、服務或外部帳戶；候選未啟用。 | [官方](https://wiki.heptabase.com/work-with-ai) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| Readwise 同步 | 外部服務 | 需相應模型、服務或外部帳戶；候選未啟用。 | [官方](https://wiki.heptabase.com/the-best-way-to-acquire-knowledge-from-readings) · [LOCAL-OPTIONS.md](../docs/LOCAL-OPTIONS.md) |
| 原生行動介面 | 未實作 | 現有應用程式只建置 Windows x64。 | [官方](https://wiki.heptabase.com/user-interface-logic) · [介面驗收](UI-ACCEPTANCE.md) |
| AI PDF 對談 | 外部服務 | 目前無文件對談服務。 | [官方](https://wiki.heptabase.com/work-with-ai) · [介面驗收](UI-ACCEPTANCE.md) |
| 卡片版本復原 | 部分 | 筆記卡的舊標題及正文可個別預覽並還原。當前文字也保存為版本。其他卡片類型與其附帶狀態待支援。 | [官方](https://support.heptabase.com/en/articles/10448124-restore-from-version-history) · [介面驗收](UI-ACCEPTANCE.md) |
| 白板分享連結 | 外部服務 | 沒有公開連結或遠端存取權限。 | [官方](https://wiki.heptabase.com/collaborate-and-discuss-with-others) · [介面驗收](UI-ACCEPTANCE.md) |
