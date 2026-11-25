# 草稿 #1–#11 跨功能整合

此分支以 `73928b6` 為基底，包含既有的完整工作區備份驗收 `4fcc372`。公開發行與凍結的 GNU 候選各有自己的版本記錄。

| 來源 | 確認的草稿 head |
| --- | --- |
| #1 | `a923d793bcc7fe35caf1afdcc808e84c8a7e4d38` |
| #2 | `f9cabe209d4ff8c8fc24ae2e54304f1f6d033c1c` |
| #3 | `d29627c9e986a462ba576660f0fc002f6dc018da` |
| #4 | `9cf4a6da256f674e6a7243efd6f9f40788e5ab23` |
| #5 | `1188540ff731afb581c226247310c536dedc119c` |
| #6 | `2f087c5b148d1c60bdcfb0f15a092d5223af3d36` |
| #7 | `d43084a2569adfe14791a1dd22f52c97cdeea172` |
| #8 | `7078250d8593dba64efdc3fb48766a0947ff6f04` |
| #9 | `7e40240f1399afb4fa7c88de54aeeb16e5ee2c87` |
| #10 | `fa2e88fd01c7903ab6c49f41bc599cb40b57c217` |
| #11 | `37e48a7633d22d91cf719e1b545a752b5c057912` |

#9 的集合寫入沿用 #8 的集合欄位規則及資料列入口。#10 的精確 Notion 路徑解析與 #9 的 CSV 操作都保留在匯入報告與教學。#11 的 Canvas 匯入沿用附件封裝資料表。共用 GUI 腳本保留三組獨立測試，再執行同一工作區的整合測例。

工作區仍使用 schema 31。從 schema 30 升級時新增空的 `notion_page_paths` 表，已有的集合、欄位規則與筆記資料保持原值。新匯入的 Markdown 才會寫入原始頁面路徑。CSV 新集合與 Canvas 附件均使用既有資料表。

## 重跑

1. 執行 `cargo test --locked`。整合測例從一份由測試建立的 schema 30 工作區開始，也會備份並在隔離副本還原。
2. Windows 工作流程會執行全套原生 Rust 測試、實際的公開 v0.1.1 升級與安裝免除檢查，並上傳未簽章的測試 ZIP。獨立工作會下載該 ZIP，核對提交、包與 EXE 的雜湊，再啟動原生介面。 [此分支的 Actions](https://github.com/yung13yubabie/paperroom/actions?query=branch%3Adev%2Fintegration%2Fpr1-11-notion-canvas-20261125)
3. Linux 與 Windows 使用同一份 [合成定義](../tests/fixtures/notion-canvas-integration.json) 產生 Notion ZIP。先從 CSV 建集合並修改列欄位，再驗證兩個同名 Markdown 子頁各自開到正確正文。第二份 CSV 以既有的非文字屬性作為欄名，應明確拒絕，資料庫前後相同。
4. 在同一工作區匯入含本機圖片的 Canvas，關閉重開後匯出 ZIP，再回匯。由介面備份後，在隔離工作區核對相同連結、CSV 列、畫布與附件位元。最後比對原工作區、備份及輸入檔案雜湊。

原生 CI 使用 Windows Server。乾淨 Windows 10／11 標準帳戶的斷網人工驗收仍待執行。
