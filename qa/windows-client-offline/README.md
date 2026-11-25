# Windows 10／11 x64 斷網驗收

每個作業系統用一台全新或剛還原的測試機，分開填寫 [驗收紀錄](EVIDENCE.md)。本目錄提供步驟與合成資料，實測結果由操作者填寫。

## 辨認候選版本

草稿為未簽章的 Paperroom v0.1.2。來源提交為 `d3fcba6f5cd206680e74084d16d595f7bf66d9f4`。[來源提交](https://github.com/yung13yubabie/paperroom/commit/d3fcba6f5cd206680e74084d16d595f7bf66d9f4)

| 物件 | SHA-256 |
|---|---|
| GitHub 下載的外層 artifact ZIP | `830fb828dea8020c7655b5f1d71eeb1a0d23ffaf8458457544cbc39a062fde0e` |
| 內層 Paperroom-v0.1.2-windows-x64-draft.zip | `85eced7fc8aaafa204f729c33aab4df309bbe3f90adbc510c5389328a0a34bca` |
| 內層解壓出的 Paperroom.exe | `61e6e5ffb694702827c7e6fe031c638429b90105590649b732728b65805665c4` |

這三筆來自 run `45573035561`。建置 job 為 `139064091837`，下載後在 Windows Server 驗收的 job 為 `139067187028`。此處要另行驗證 Windows 用戶端。[建置](https://github.com/yung13yubabie/paperroom/actions/runs/45573035561/job/139064091837) · [下載後驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45573035561/job/139067187028) · [下載 artifact 15002184653](https://github.com/yung13yubabie/paperroom/actions/runs/45573035561/artifacts/15002184653)

## 準備

1. 在可連線的準備機下載外層檔案，保留下載來源與安全標記。分別核對外層、內層雜湊，再將完整候選與本目錄移入已授權、已隔離的測試機。在使用者可寫的空白目錄建立 `default`、`portable`、`restore` 三個資料夾，每處解壓同一份內層 ZIP，令各資料夾直屬一份 EXE。觀察正常下載的 SmartScreen 需要記錄實際取得途徑；經離線媒體或沙箱轉送的檔案可能沒有保留來源標記。
2. 使用標準帳戶、非提升權限，在沒有預裝紙間、也沒有 `%LOCALAPPDATA%\Paperroom` 的乾淨 Windows 用戶端開啟 64 位元 Windows PowerShell。若遇既有資料，停止並換乾淨工作區。只使用 [合成 Markdown](fixture.md)。不要安裝額外 runtime、登入服務或更動防護。
3. 在上述三個資料夾的父目錄執行下列命令，將 `qa\windows-client-offline` 指向這份交接資料。若外層檔案曾改名，將其實際名稱代入。檢查點只觀察當時的狀態，不會停用網卡；需由測試環境在首啟前即提供斷網隔離。

```powershell
Get-FileHash .\outer.zip, .\Paperroom-v0.1.2-windows-x64-draft.zip, .\default\Paperroom.exe -Algorithm SHA256
& .\qa\windows-client-offline\checkpoint.ps1 -Phase before -Exe .\default\Paperroom.exe -Archive .\Paperroom-v0.1.2-windows-x64-draft.zip
```

從 CMD 執行同一檢查點可用 `powershell.exe -NoProfile -File .\qa\windows-client-offline\checkpoint.ps1 -Phase before -Exe .\default\Paperroom.exe`。保存 JSON；非零退出碼、未符合標準帳戶、無法讀取網路狀態或政策阻擋腳本時，填 BLOCKED。若雜湊與所選版本不同，停止啟動。不要解除封鎖、提升權限或繞過執行原則。

若測試其他已核對的候選，另開一份紀錄，先填該版來源、run、內層 ZIP 與 EXE 雜湊；各檢查點明確傳入 `-ExpectedArchiveSha256` 及 `-ExpectedExeSha256`。如要核對內層檔案本身，一併傳入 `-Archive`。保留本頁固定候選的原紀錄。

## 實際操作

1. **預設首啟**。執行 `.\default\Paperroom.exe`。記錄是否正常出現主視窗、有無安裝／登入／下載要求及任何安全提示。新增標題「預設模式筆記」，正文打入 `DEFAULTBODY583`，等顯示已儲存。按 Ctrl+F 搜尋正文中的這串字，確認能開回該篇。正常退出，確認 `%LOCALAPPDATA%\Paperroom\notes.sqlite3` 存在，EXE 旁沒有新建可攜資料庫。用同一 EXE 重開，查到原文後退出。
2. **可攜首啟**。執行 `.\portable\Paperroom.exe --portable`。確認搜尋不到 `DEFAULTBODY583`。用「檔案 → 匯入 Markdown」選取 fixture.md。搜尋正文的 `PRCLIENTBODY47`，確認找到「離線觀察筆記」。在編輯模式只選取 `中文🎬選字`，從「更多」切換選取文字的螢光標記。切到閱讀，檢查黃色範圍、兩側文字及字面 `==保留==`。正常退出。可攜 DB 應在 `portable\paperroom-data\notes.sqlite3`。
3. **重開與備份**。執行檢查點 `-Phase reopen -Exe .\portable\Paperroom.exe`。重開可攜模式，核對搜尋、標亮及未上色的相鄰文字。「檔案 → 備份工作區」另存新檔 `synthetic-backup.sqlite3`，正常退出。保留原工作區。
4. **隔離還原**。確認 `restore\paperroom-data` 尚不存在，才建立這個資料夾，將剛產生的備份複製至其中並命名為 `notes.sqlite3`。以 Get-FileHash 核對兩份資料庫；第一次啟動前兩者應相同。跑 `-Phase restore -Exe .\restore\Paperroom.exe`，再執行 `.\restore\Paperroom.exe --portable`。搜尋同一詞、核對原文與標亮。在還原工作區另建「還原測試專用」筆記；退出後重開原可攜工作區，應找不到這篇。
5. **離線 HTML**。在還原工作區用「檔案 → 匯出工作區 HTML ZIP」，解壓至空目錄，再用本機瀏覽器開 index.html。找到「離線觀察筆記」，確認僅選區標亮，外部連線無需啟用。記錄該頁連到的 Markdown 檔。匯入會將 fixture.md 的第一行 H1 抽成標題，下列 PowerShell 比對匯出正文；命令中的兩條路徑請換成實際檔名。

```powershell
$utf8 = [Text.UTF8Encoding]::new($false,$true)
$fixture = [IO.File]::ReadAllText((Resolve-Path .\qa\windows-client-offline\fixture.md),$utf8)
$body = ($fixture -replace "^# 離線觀察筆記\r?\n\r?\n",'').Replace("`r`n","`n")
$actual = [IO.File]::ReadAllBytes((Resolve-Path .\html\markdown\123.md))
[Convert]::ToBase64String($actual) -ceq [Convert]::ToBase64String($utf8.GetBytes($body))
```

6. **About 與結束**。在已正常關閉的還原工作區記錄 DB 雜湊，再開同一 EXE 的可攜模式。從「檔案 → 關於紙間／第三方補充通知」依序核對 Foxit、Adobe CMaps、ICC 色彩描述檔與 alloc-stdlib 四份通知，與內層候選 ZIP 的 `licenses/pdf-assets` 四份原文對照。正常退出，再記一次 DB 雜湊。若有活動中的 WAL 或 journal，記為未完成比對。在所有工作區均退出後執行 `-Phase after -Exe .\restore\Paperroom.exe`，保存 JSON。

## 如何填結果

PASS 需有該列的實際觀察及雜湊或合成資料證據。程式打開後結果不符，填 FAIL 並記下可重做的步驟。若前置條件、授權、裝置或安全提示阻止進行，填 BLOCKED 並記錄停止點；未執行項目留作未執行。觀察到沒有可用網路，只能證明當時不能連線，程式是否曾嘗試連線需要另外的事件追蹤。請於丟棄測試機前保留去識別的紀錄。
