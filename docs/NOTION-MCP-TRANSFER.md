# 將明確選取的 Notion 頁面文字帶入紙間

這個選用工具在本機將確認過的頁面文字製成紙間可匯入的 ZIP。執行轉檔需要 Python 3.10 以上。紙間開啟 ZIP 時不需要 Python。

1. 在既有授權的連接器中逐篇確認頁面 ID，只讀取同意移轉的頁面。取出標題及正文的 Notion Markdown 文字。省略外層服務回應、工作區路徑、權限及討論。保留實際的截短與未知區塊欄位；如果無從確認結果完整，停止移轉。
2. 依下列格式組成 UTF-8 JSON。只使用所選頁面，將正文完整放在字串中。工具會原封保存這份 JSON，請先確認內容適合放進可攜檔。
3. 在命令列（Windows 可用 cmd.exe）執行 `python scripts/notion_mcp_transfer.py transfer.zip < selected.json`。輸出檔名必須尚未存在。將 ZIP 放在自己的保存位置。
4. 在紙間「檔案」選擇 Notion 匯出 ZIP 的匯入入口，開啟這份轉檔包。依標題和正文核對每篇。JSON 收據及原始輸入留在 ZIP，匯入時可看到略過非筆記檔案的計數。

```json
{"schema":"paperroom-notion-mcp-transfer/1","confirmed_page_ids":["11111111-1111-1111-1111-111111111111"],"pages":[{"id":"11111111-1111-1111-1111-111111111111","title":"鏡頭","markdown":"第一段。","truncated":false,"unknown_block_count":0,"unknown_block_ids":[]}]}
```

每一個 ID 都必須有且僅有一筆對應記錄。可放 1–128 篇，輸入上限 8 MiB，每篇輸出的 Markdown 含標題上限 4 MiB。同名頁面使用不同 ID 當成 ZIP 內路徑。正文可保留空白、中文、emoji、外部網址和 Notion 標記。紙間讀入後可能正規化 CRLF。清單中的 SHA-256 可核對原始輸入、每篇正文及 Markdown 檔案。

這是紙間自訂的文字移轉包。原廠匯出的資料庫、附件、權限、即時檢視及關聯各有自己的結構；此流程只移轉提供的字串。內嵌服務標籤及連結會留在文字中，不會取回子頁、媒體或建立可用的關聯。子頁要另行明確選取。請保留原始 Notion 內容及這份 ZIP。工具只讀標準輸入，無須提供 token、附件路徑或網路連線。持有 ZIP 的人可閱讀其全部內容。
