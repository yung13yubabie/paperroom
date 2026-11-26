# Windows 簽章準備

紙間固定的 Windows GNU 草稿為未簽章 EXE。Windows 驗收紀錄顯示 Authenticode 狀態為 `NotSigned`，PE 憑證表為空。檔案內容已有產品名稱和 Cargo 套件版本。 [固定候選與封裝](WINDOWS-GNU-0118861-REDISTRIBUTION.md)

正式版先由發行者決定是否採用簽章。若選擇簽章，核准後再對已審核來源的產物執行，並為簽章後的新 EXE 和 ZIP 重新驗雜湊及正常下載首啟。乾淨 Windows 10／11 的斷網驗收另有結果欄。 [Windows 斷網手冊](../qa/windows-client-offline/GNU-CANDIDATE.md)

## 申請前由誰決定什麼

| 事項 | 待辦 |
| --- | --- |
| 原始碼與素材 | 所有者確認有權對外授權的內容，為專案選擇 OSI 認可的授權，加入根目錄授權全文。依賴及素材逐項見 [再散布清單](REDISTRIBUTION.md)。上游作者的聲明與授權須隨適用產物提供。 |
| 公開發行與維護紀錄 | 公開功能、下載說明、安裝與移除方式，以及可下載的未簽章版本。Foundation 會另外檢視維護狀況、專案聲譽、儲存庫控制權及發行物。核准由 Foundation 決定。 |
| 團隊與政策 | 由所有者指定能提交、審查及核准簽章的成員，啟用多因素驗證。申請時按正式規則準備「Code signing policy」以及資料傳輸與隱私說明；獲准使用服務後再刊登適用的贊助標示。 |

條款也涵蓋商業雙重授權、專有元件、系統函式庫例外及簽署上游程式的限制。申請人應逐項比對當時的版本。[SignPath Foundation 條款](https://signpath.org/terms)

## 現階段如何產生候選檔

草稿 PR #30 使用 [Windows GNU 草稿流程](../.github/workflows/windows-gnu-candidate.yml)。[手動未簽章 MSVC 流程](../.github/workflows/unsigned-windows.yml) 有自己的建置和再散布證據。審查時保留完整 commit、run、attempt、工件 ID、EXE SHA-256 與 GitHub 外層工件 digest。簽署決定應對應實際要發行的工具鏈與工件。

## 獲准後如何交付簽章版

1. 在 GitHub 審查來源、建置腳本、工作流程與鎖檔，確認正式 tag 指向通過審查的 commit。由 GitHub-hosted runner 從該 commit 建置並上傳未簽章工件。記錄同一 run 的 artifact ID；本機交叉編譯的候選檔只供測試。
2. 獲准的管理員將 GitHub trusted build system 綁到正確的專案。依核准政策限定儲存庫、工作流程、ref 和 runner；需要稽核日誌時依官方指引安裝 GitHub App。配置時限制 ZIP 內允許簽署的檔名、產品名及其版本。簽署目標只有紙間自己的執行檔。[SignPath 的 GitHub 來源驗證](https://docs.signpath.io/trusted-build-systems/github)
3. 由未來新增且已審查的簽章工作傳入上一步的 artifact ID。核准人逐次對照來源、版本、測試記錄及簽署對象後手動批准。啟用 SHA-256 Authenticode 與可信的 RFC 3161 時間戳；從簽章服務取回成品，保留 request ID、核准紀錄與時間戳。
4. 在可檢查憑證鏈及撤銷狀態的 Windows 上，用 Windows SDK 執行下列檢查。若有警告、驗證失敗、發行者與已核准政策不符，或時間戳無法驗證，先停住發佈。保存簽署者、憑證指紋、時間戳與完整驗證輸出。[Microsoft SignTool 說明](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool)

   ~~~powershell
   signtool verify /pa /all /tw /v .\Paperroom.exe
   if ($LASTEXITCODE -ne 0) { throw "簽章或時間戳驗證有誤" }
   Get-AuthenticodeSignature -LiteralPath .\Paperroom.exe |
       Format-List Status, SignerCertificate, TimeStamperCertificate
   Get-FileHash -Algorithm SHA256 -LiteralPath .\Paperroom.exe
   ~~~

5. 用已簽署的同一份 EXE 建立新版本的 Windows ZIP，連同該版本需要的授權資料打包，重新產生 EXE 和 ZIP 的 SHA-256。解壓後對照單獨的 EXE；另核對原始碼 ZIP 所指向的 tag。將簽署前後的兩組記錄分開保存。
6. 從實際 Release 以瀏覽器下載 EXE 及 ZIP，在乾淨 Windows 標準帳戶保留下載標記及預設防護。記錄系統版本、SHA-256、發行者及 SmartScreen 的提示原文。有效簽章的程式也可能因信譽尚待累積而出現提示。另用相同雜湊在無網卡的乾淨環境測首啟、繁中筆記儲存及搜尋、備份還原、強制關閉後重開；若離線系統缺少信任鏈或無法查詢撤銷狀態，分開記錄驗簽與應用程式結果。[Microsoft SmartScreen 說明](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
