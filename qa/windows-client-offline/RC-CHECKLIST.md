# 整合候選的 Windows 10／11 斷網驗收單

每個用戶端版本各用一份空白紀錄。只有完成以下步驟的乾淨 Windows x64 標準帳戶可填實測結果。初始狀態為「未執行」。

## 帶到測試機的檔案

在準備機從本次整合提交的 GitHub Actions run 下載候選及本頁和同版 checkpoint.ps1。登記完整 commit、run／job、artifact ID。分別記錄 GitHub 外層下載 ZIP、裡面的 Windows 發行 ZIP、以及從內層直接解出的 Paperroom.exe 的 SHA-256。將實測值與同一 run 的封裝產物清單及下載驗證紀錄逐項核對，記下每個預期值的來源。外層如僅有本機實測值，標成「收件指紋」，另記可獨立核對的內層及 EXE 雜湊。比對不符即停止；勿用歷史候選雜湊、重建檔或自行重壓的 ZIP 代替。

在已授權的乾淨用戶端，使用全新合成目錄，內層 ZIP 分別解到 default、portable 和 restore。只用已隔離、未預裝紙間、無既有紙間工作區的標準帳戶。記錄 Windows 版本、版別、完整 build／UBR、取得檔案的方式及是否保留來源標記；用戶名和機器名免填。全程維持離線，毋須安裝或登入。若 Windows 安全政策、SmartScreen 或腳本執行原則攔阻，記錄階段、提示原文及畫面後停止，不解除封鎖或調整防護。

於上述資料夾的父目錄開啟 64 位元 Windows PowerShell。將兩個預期值從同一 run 的驗證紀錄抄入，zip 路徑填內層 ZIP。於四個階段各執行一次以下函式。保存各次 JSON 與退出碼，檢查非迴路網卡啟用數為零、無可用網路、標準帳戶且 EXE 與 ZIP 雜湊相符。檢查點只反映當下，另記測試期間的隔離方式。

```powershell
$zip = Read-Host 'Windows 發行 ZIP 的路徑'
$expectedZip = Read-Host '同一 run 的內層 ZIP SHA-256'
$expectedExe = Read-Host '同一 run 的 EXE SHA-256'
$checkpoint = Read-Host '交接包內 checkpoint.ps1 的路徑'
function Check-Client($phase, $exe) {
    & $checkpoint -Phase $phase -Exe $exe -Archive $zip -ExpectedArchiveSha256 $expectedZip -ExpectedExeSha256 $expectedExe
    if ($LASTEXITCODE -ne 0) { throw "停止於 $phase" }
}
Check-Client before .\default\Paperroom.exe
```

## 在同一份 EXE 實際操作

1. **預設與可攜分離。** 雙擊 default 下的 EXE，新增「預設專用 RC-01」，儲存、退出、重開後搜尋。退出後執行 `.\portable\Paperroom.exe --portable`，確認搜不到該詞；新增「可攜專用 RC-02」並儲存。確認兩處資料庫分別在預設的本機應用程式資料目錄及 portable 下的 paperroom-data。
2. **集合規則與拒絕寫入。** 在可攜工作區用「檔案 → 屬性型別」宣告數字「時長」。建立測試集合，在「看板操作 → 集合欄位規則」指定「時長」必填、預設 12。「新增資料列」時核對帶入 12，建「鏡頭甲」；再建「鏡頭乙」並改為 0。記錄列數及值。在「鏡頭甲」的型別欄位將 12 改成 abc 並提交，保存拒絕提示與編輯框仍有 abc 的畫面。取消後核對值仍為 12。再新增「不得存在 RC-03」，清空必填值並提交，確認被拒、取消後列數不變。
3. **重開與備份。** 正常退出，執行 `Check-Client reopen .\portable\Paperroom.exe`。重開可攜 EXE，確認兩列值 12、0，搜尋不到 RC-03；再用新增列對話框核對預設 12 後取消。透過「檔案 → 備份工作區」另存全新 synthetic-backup.sqlite3，正常退出。記錄備份及 portable 工作區已關閉之 notes.sqlite3 雜湊。若 SQLite sidecar 仍在，先記錄情形，不刪檔，也不將單個主檔判作完整靜止的資料庫。
4. **隔離還原。** 確認 restore 下尚無 paperroom-data，再新建它，把剛產生的備份複製為其中的 notes.sqlite3。比對複本與備份 SHA-256，執行 `Check-Client restore .\restore\Paperroom.exe`。用 `.\restore\Paperroom.exe --portable` 重開，比對 RC-02、兩列值及預設／必填規則。僅在還原副本建立「還原專用 RC-04」，正常退出。核對原工作區主檔與備份雜湊均與第 3 步相同，再回原可攜區搜尋，應看不到 RC-04。對缺資料、程式中止或非預期變動，保留失敗紀錄及合成檔。
5. **結束。** 於實際候選的「檔案 → 關於紙間／第三方補充通知」開啟內嵌通知並與同版發行 ZIP 內文件對照，記錄可以讀取的項目。關閉程式，執行 `Check-Client after .\restore\Paperroom.exe`。如要覆蓋 PDF 摘錄回跳等其他功能，個別填寫操作與實際證據。

## 實測紀錄（每部用戶端複製一份）

| 項目 | 實際值或結果 |
|---|---|
| 日期 UTC／版本版別／完整 build／乾淨來源／標準帳戶／隔離方式 | |
| commit／run／job／artifact ID 與下載網址 | |
| 外層 ZIP 實測 SHA-256／搬運及來源標記 | |
| 內層 ZIP 與 EXE 各自的預期值、來源及實測 SHA-256 | |
| 四次檢查點 JSON 與退出碼／安全提示原文或未見 | |
| 預設啟動、持久性及可攜隔離 | 未執行 |
| 必填與預設、零值、失敗輸入保留、取消後列數與原值 | 未執行 |
| 重開、備份、隔離還原、原工作區及備份雜湊 | 未執行 |
| About 通知、安裝或連線要求、首個障礙與其餘未執行項目 | 未執行 |

每列填「通過／失敗／受阻／未執行」，附觀察值、檔案雜湊或合成畫面編號。發布前遮蔽路徑中的個資。
