# Paperroom 草稿 #12 發版準備驗證

這份紀錄涵蓋獨立整合分支的測試，以及既有凍結 GNU 候選的發版條件。兩個成品各自使用自己的提交、工作紀錄和校驗值。

## #1–#12 整合驗收

草稿 #12 的已驗 head 為 `721661e5fa777cf766953deb85c176dcc9d7013f`，其父提交 `ec3415d611f1e89851470794695b61a234afda1f` 包含 #1–#11 及完整工作區備份驗收。此分支新增測試、收據與紀錄，產品原始碼的行為沿用 #12。

Linux 及 Windows Rust 測試使用同一份合成定義。本機 SQLite 整合測試從 schema 30 升級。Windows Server 的原生測試從 GitHub 下載並校驗 ZIP，執行解出的同一份 EXE。兩條測試都在已有 Notion CSV 編輯的工作區匯入 `Boards/VaultRestoreScene.canvas`，明確選擇 Vault 模式。在 Vault 根目錄和 `Boards` 放入內容不同且相對路徑相同的 PNG。備份後先移開合成 Vault，再在隔離工作區還原，確認檔案卡與群組背景各自保存根目錄的位元組，並保留 CSV 的修改。Windows 另外從介面檢查兩張圖片實際繪製。

Windows 原生工作上傳 `vault-restore.json`，列出嘗試中的來源提交、實際 EXE／ZIP 雜湊、Windows Server 組建、備份與還原副本的雜湊、兩張圖片的實測雜湊和原 CSV 欄值。 [本分支 Actions](https://github.com/yung13yubabie/paperroom/actions?query=branch%3Adev%2Fintegration%2Fpr1-12-readiness-20261125)

## 凍結 v0.1.2 GNU

| 項目 | 查核結果 |
| --- | --- |
| 成品身分及可重建性 | 應用來源 `73928b6`，建置配方 `d9e4fca`。兩份原始 EXE 和內層 EXE 的 SHA-256 均為 `c8940de5da3ecb6785fcd0350cc371f8bd7fd2a8c5847028ca153a272cff4055`。內層 ZIP 為 `f1f0765532e0ac98dbda2cd927242f26d60d6c4f588cef8a2955315bb7d25b51`。194 筆檔案校驗值全數相符，另有清單檔本身。 |
| PE 與通知 | 確認 x64 PE、無延遲匯入及無 Authenticode 憑證表。包內有 GNU 工具鏈、Rust、依賴及 PDF 素材通知與連結清單。這次確認的是附帶資料，尚無最後的再散布權利核可。 |
| Windows Server 原生 | 原凍結工件的下載、工作區升級及原生 GUI 已驗。 [凍結 GNU 的原生驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45666319740/job/139391352883) |
| Windows 10 x64 乾淨斷網 | 未執行。缺可用且獲准的乾淨 x64 客戶端，需標準帳戶和可維持斷網的實際測試環境。 |
| Windows 11 x64 乾淨斷網 | 未執行。缺可用且獲准的乾淨 x64 客戶端，需標準帳戶和可維持斷網的實際測試環境。 |
| 下載後的政策與啟動 | 仍待客戶端實測。凍結 EXE 為未簽章，尚未記錄乾淨客戶端的 SmartScreen／執行政策結果。 |

[凍結 GNU 測試包](https://github.com/yung13yubabie/paperroom/actions/runs/45666319740/artifacts/15047837035) · [兩份原始 EXE](https://github.com/yung13yubabie/paperroom/actions/runs/45666319740/artifacts/15047377517)

沿用 `Paperroom-v0.1.2-GNU-client-handoff-d9e4fca.zip` 的固定交接包。其 SHA-256 是 `c350fbb62765770262553f78da4831152ddf8108991d9b31c7bd7c207fe1d050`。每個 Windows 客戶端各留一份版別與完整組建、標準帳戶、斷網方式、原件雜湊與四份檢查點。

## 可用客戶端路徑

- 目前連線的 managed workspace 是 Linux。沒有可用的 Windows 客戶端虛擬機工具或工作區內可用映像。已登記的 Windows 桌機離線，其版別、組建、乾淨狀態與可斷網方式均未確認。
- GitHub 目前的 x64 Windows 託管執行器使用 Server 映像。另有 Windows 11 ARM64 託管執行器，與固定的 x64 客戶端環境不同。此專案的現有工作流程未配置客戶端自管執行入口，也沒有已驗證可用的 x64 客戶端 runner。 [GitHub 託管執行器](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) · [Windows Server 2022 映像](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md)
- Microsoft 提供 Windows 10 ISO 及 Windows 11 Enterprise 評估版。ISO 仍需要獲准的主機與安裝環境，評估版也有註冊及安裝步驟。本輪未發現已備妥的可用執行環境。 [Windows 10 ISO](https://www.microsoft.com/en-us/software-download/windows10ISO) · [Windows 11 Enterprise 評估版](https://www.microsoft.com/en-us/evalcenter/download-windows-11-enterprise)
