# Windows GNU 候選的斷網驗收

Windows 10 和 Windows 11 x64 各做一次。只使用已獲准的空白測試機或剛還原的客體、從未使用紙間的標準帳戶，以及這份目錄的合成資料。這份手冊未記錄任何客戶端通過結果。

## 固定下載物

在準備機記下本次候選的完整來源提交、Actions run 與 artifact ID。從同一次建置逐一核對 GitHub 外層 ZIP、裡面的 Windows GNU ZIP、獨立 EXE 和內含 EXE。獨立 EXE 與內含 EXE 應同位元。預期雜湊來自已核對的下載收據，填入下列參數。將原外層 ZIP、本目錄和這組預期值離線轉入客體。保留來源標記和 Windows 安全提示的原況。

在首啟之前移除測試環境的網路連線，維持隔離直到全部結束。用 64 位元 Windows PowerShell 執行下列命令，將 JSON 提示及結果保存到交付資料夾。客體需有正常的 Windows 保護設定。遇到安全提示、執行原則或首啟要求時，記下時點、內容及畫面後停止。

```powershell
& .\qa\windows-client-offline\stage-gnu-candidate.ps1 -OuterZip .\download.zip -OuterSha256 '<外層 SHA-256>' -InnerSha256 '<內層 SHA-256>' -ExeSha256 '<EXE SHA-256>' -SourceCommit '<40 碼來源 SHA>' -RunId '<run ID>' -ArtifactId '<artifact ID>' -ExpectedFamily 'Windows 10' -Destination .\paperroom-client-w10
```

在 Windows 11 使用另一個空目錄並調整 `ExpectedFamily`。目的地選用測試帳戶可寫入的短本機路徑。此步驟驗雜湊、展開同一個包三次、複製合成資料，並保存 `stage.json` 與 `before.json`。它只觀察客戶端、帳戶與當時的網路狀況。操作者需另外記錄斷網方式及開始／結束時點。失敗時保留目錄和紀錄，重試用新目錄。

## 實際介面操作

1. **預設啟動及重開。** 直接開 `default/Paperroom.exe`，確認主視窗可用。建筆記「RC 預設」，正文填 `RC-DEFAULT-1227`。正常退出並重開，用正文搜尋。預設資料庫應在 `%LOCALAPPDATA%\Paperroom`。記錄任何安裝、登入、連線或 Windows 安全提示。
2. **可攜與隔離。** 開 `portable/Paperroom.exe --portable`，確認搜不到預設筆記。從 UI 匯入 `fixture.md`，搜尋 `PRCLIENTBODY47`，顯式標亮「中文🎬選字」，並確認相鄰內容與字面 `==保留==`。退出後紀錄可攜資料位置和重開前檢查點。再次開啟後確認原文、搜尋及標亮保留。
3. **備份與副本還原。** 從可攜工作區用「檔案 → 備份工作區」另存 `synthetic-backup.sqlite3`，正常退出並記下合成 DB 與備份雜湊。只在 `restore` 下建立新的 `paperroom-data`，複製備份成 `notes.sqlite3`，核對複本等於備份。寫入還原檢查點後啟動 `restore/Paperroom.exe --portable`。確認原筆記可用，只在副本新增「RC 還原專用」。關閉後核對備份仍等於原雜湊，再回原可攜工作區確認搜不到「RC 還原專用」。若 SQLite sidecar 仍存在，保留檔案及各自的雜湊，並記錄當時狀態。
4. **離線 HTML 和通知。** 在還原副本用「檔案 → 匯出工作區 HTML ZIP」另存至新的合成資料夾，用本機瀏覽器離線讀取 `index.html`。核對正文和標亮。從「檔案 → 關於紙間／第三方補充通知」核對 Foxit、Adobe CMaps、ICC 和 alloc-stdlib 四份通知，記錄可開啟的項目。正常關閉所有測試視窗，再寫入結束檢查點。

從驗收目錄重跑相同的 `checkpoint.ps1`，每次使用 `stage.json` 的內層 ZIP 及 EXE 預期值。以 `reopen`、`restore`、`after` 分別記錄，保留其 JSON 與退出碼，並不要覆蓋 `before.json`。

```powershell
& .\checkpoint.ps1 -Phase reopen -Exe '.\portable\Paperroom.exe' -Archive '.\<內層 ZIP>' -ExpectedArchiveSha256 '<內層 SHA-256>' -ExpectedExeSha256 '<EXE SHA-256>' | Set-Content .\reopen.json -Encoding UTF8
$LASTEXITCODE
```

逐項標記「通過／失敗／受阻／未執行」。保留合成來源、輸出、已關閉工作區的 DB 和 sidecar、檢查點、截圖及 SHA-256。測試資料之外的個人資料、識別碼與路徑請勿交付。Windows Server 結果可補充其對應場景，Windows 10／11 的標準帳戶斷網結果各自填寫。
