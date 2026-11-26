# Heptabase 操作驗收

[返回三款互通驗收](INTEROP-ACCEPTANCE.md)

## 已實裝且驗證

| 操作 | 驗收範圍 |
| --- | --- |
| 在紙間多張白板放置同一筆記並開回來源 | 紙間本機以同一筆記 ID 表示放置；從白板和筆記可往返指定位置。驗收包含同名筆記以及重開。[本機操作](UNIFIED-WORKSPACE.md) |
| 開啟有獨立標準 Canvas 的 Markdown ZIP | 合成 ZIP 驗過路徑對應、多處放置、原始畫布以及備份復原。範圍是 [JSON Canvas](https://jsoncanvas.org/spec/1.0/) 的對應欄位。[實測](https://github.com/yung13yubabie/paperroom/actions/runs/45761722174/job/139713033401) |
| 從檔案卡和群組背景另存原始圖片 | 合成 ZIP 使用不同資料夾內的同名 PNG，透過實際右鍵分別另存。隔離還原後和原檔位元相同。[原生驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45765855523/job/139727549909) |
| 用完整工作區備份保留本機白板、附件及卡位 | 已測同包白板、節點與附件，備份在隔離工作區重開後維持身分和來源。[白板與附件](https://github.com/yung13yubabie/paperroom/actions/runs/45763582824/job/139720173418) |
| 由原有卡片建立子白板並定位上層卡位 | 本機建立子白板時擺入原筆記 ID，原白板和其他白板仍共用筆記。返回會定位上層保存的子白板卡位。[本輪驗收流程](../scripts/windows-native-child-board.ps1) |

## 已實裝待真實來源

- 可匯入原廠備份中可辨認的 Markdown 部分。合成包已驗，待用獲准的原廠備份核對實際檔名、巢狀頁、附件及正文。[Heptabase 資料可攜性](https://support.heptabase.com/en/articles/10364279-business-sustainability-and-data-portability)

## 未實裝

- 原廠 `All-Data.json` 的解析、完整 Space 還原、由備份重建原廠白板關係或歷史。標準 Canvas 另有獨立入口。
- 官方備份對所有私有卡片型別、圖表、原廠標籤和應用程式狀態的完整相容。

## 離線不可實現的雲端服務

- 邀請 Space 分享、權限、多人即時白板編輯和提供外部閱讀的即時發佈連結。[協作與權限](https://support.heptabase.com/en/articles/10510497-collaboration-and-permissions) · [發佈白板](https://support.heptabase.com/en/articles/12121546-publish-a-whiteboard)
