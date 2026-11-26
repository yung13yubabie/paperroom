# 紙間三來源整合的 Windows GNU 草稿

此 ZIP 是未簽署的 Windows x64 草稿。實際來源提交、EXE SHA-256、兩次建置的校驗以及原始依賴通知，請讀封裝中的建置清冊和證據檔。既有 v0.1.2 預覽另有固定來源與工件。本檔不代表 Windows 10 或 Windows 11 乾淨客戶端已驗收，也不授予專案的正式發行權利。

這個候選可透過紙間既有 SQLite 工作區，使用所選 Notion 頁面自訂文字交接包、Heptabase Markdown 與相容標準 JSON Canvas、以及部分 Obsidian Bases 表格或清單。可用範圍見 README。原廠功能和匯出格式分別驗證。第三方登入不屬於 EXE 的執行依賴。

## 逐項依賴證據與用戶端收據

新版封裝的 `evidence/gnu-dependency-redistribution.json` 綁定來源提交和 EXE。它逐項列出 Cargo 目標與建置角色、Cargo.lock 記下的 registry 封存雜湊、套件宣告授權、包內通知原文的路徑與雜湊，以及可從第一份最終 GNU link map 辨認的非空 Rust 封存物件。建置器使用 Cargo 實際的 library target 名稱。跨版本同名物件留在未歸屬清單。沒有觀察到的項目只代表本次 map 未歸屬，可能受連結最佳化或巨集展開影響。MinGW、GCC 與 Rust 執行階段另見 GNU runtime 成員收據。Windows OS DLL 只列直接匯入。

`alloc-stdlib 0.2.4` 的 crate 封存沒有可收錄的授權檔名。封裝另附官方固定上游版本的原文與對應說明，並以雜湊核對。其他缺件列為未解，不能把開放原始碼宣告當成原始通知。這份清冊提供建置和通知證據。發行者仍要以實際最終產物審查適用條款與未辨識元件。

沿用相同 build 腳本產出新包後，可用 `package-windows-gnu-audit.py verify --require-dependency-evidence` 與其既有必要參數重新核對下載檔。舊包仍可用一般 `verify` 查驗其原有證據。若要做真正的用戶端斷網測試，使用 [Windows GNU 候選的斷網驗收](../qa/windows-client-offline/GNU-CANDIDATE.md)。Windows Server CI 不填入用戶端結果欄。
