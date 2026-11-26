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

## 讓同批的頁面提及可以離線開啟

有兩份已確認頁面的公開格式資料時，可把輸入的 `schema` 換成 `paperroom-notion-mcp-transfer/2`，另加陣列 `confirmed_page_references`。每筆代表一個經你確認的原廠區塊引用。若只有 Markdown 文字，使用第 1 版，或把陣列留空。工具不根據名字、網址或文字中的標籤猜測目標。

以下片段放在輸入 JSON 的最外層。頁面 ID 必須來自此次確認範圍；範例中的目標頁面也要在 `pages` 裡。

```json
"confirmed_page_references": [{
  "source_page_id": "11111111111111111111111111111111",
  "block_id": "33333333333333333333333333333333",
  "block_parent_page_id": "11111111111111111111111111111111",
  "block_type": "paragraph",
  "rich_text_type": "mention",
  "mention_type": "page",
  "target_page_id": "22222222222222222222222222222222",
  "code": false
}]
```

從你已取得且確認的 Notion 區塊資料抄取 `block.id`、`block.parent.page_id`、`block.type`，以及同一個 `paragraph.rich_text` 項目的 `type`、`mention.type`、`mention.page.id` 和 `annotations.code`。只支援直接屬於該來源頁的段落；巢狀區塊、其他類型與純文字形式的連結不在此流程內。這是離線提供的來源證據，工具不登入 Notion 查證。最多接受 2,048 筆。可對照 [Notion 區塊](https://developers.notion.com/reference/block)、[父層物件](https://developers.notion.com/reference/parent-object)及[頁面提及](https://developers.notion.com/reference/rich-text)。

匯入 ZIP 後，在來源頁下開啟「本機頁面引用」，從連結選單跳到同批目標頁。伴隨筆記只列出經確認的目標，保留每個頁面的 ID 供同名辨識；同一來源多次提及同一目標只列一次。原頁正文與第 1 版逐位元相同。清單記錄每筆列出、重複、未在選定範圍或型別未支援的情況、來源區塊 ID 及伴隨檔的 SHA-256。此處不建立資料庫屬性關聯或即時雙向同步。

型別、來源區塊與父頁 ID 由製作移轉包的人提供。這些欄位用於離線一致性檢查，無法作為 Notion 簽發的認證。清單只記錄批次外目標的略過原因，不複製該目標 ID；原始輸入檔會逐位元保留，因此仍可能含有批次外 ID。分享 ZIP 前請確認原始輸入適合收件者閱讀。

## 匯入確認過的資料庫具名關聯

使用第 3 版 `paperroom-notion-mcp-transfer/3`，原有的 `pages` 與 `confirmed_page_ids` 照舊。另提供 `confirmed_page_data_sources`（每篇所選頁面的 ID 對應所屬資料來源 ID）和 `confirmed_database_relations`。每條邊含 `source_page_id`、`property_name`、`property_type`（必須是 `relation`）、`target_data_source_id` 與 `target_page_id`。請從資料來源結構確認關聯欄位與目的資料來源，再從來源頁的欄位值確認目標 ID。這些是製作者提供的證據。CSV 中的網址或外觀相同的標題不足以確認關聯。

選「匯入 Notion ZIP」，核對顯示的條數後同意；取消會放棄整批匯入。於來源頁的「連結 → 具名關聯的來源與目標 → 此筆記指向的具名關聯」檢查目的筆記。兩端均以本次外層 ZIP 的明確 Markdown 檔名配對。目標缺失、多義、型別不符或超過工作區關聯上限會拒絕匯入。第 3 版只移轉列明的單向邊，並保留原始輸入在 ZIP；不推斷反向欄位、公式、彙總或完整資料庫結構。自我關聯不受支援。第 1 版文字和第 2 版引用伴隨筆記仍可使用。

## 把已匯入的 Notion CSV 建成有型別的集合

若 CSV 的數字、核取方塊或日期欄在紙間工作區已有同名的屬性型別，開啟該 CSV 的來源筆記，依序選「更多 → 附件與 PDF 摘錄 → 管理附件」，選擇原始 CSV，再選「依工作區欄位型別建立集合」。預覽會列出來源路徑、資料列數與每個套用的型別。確認後輸入新集合名稱；在集合的「編輯資料列的型別欄位」即可修改。

CSV 欄名會移除前後空白，再以完整名稱配對工作區既有型別，不猜測原廠欄型或建立新定義。數字採十進位整數或小數，不經浮點數轉換；核取方塊支援 true／false；日期採有效的 YYYY-MM-DD。型別欄的空儲存格留為未設定，0 和 false 都是有值。其餘欄位保留文字與前導零。第一欄仍是筆記標題。若來源或欄位定義在確認後改變，需重新預覽。整批失敗或取消時不建立集合；原始 CSV 保留，可另存比對。原先的「從 Notion CSV 建立新集合」維持純文字流程。
