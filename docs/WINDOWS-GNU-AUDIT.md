# Paperroom GNU Windows x64 稽核候選

這是免安裝 Windows 替代建置的測試包。應用程式原始碼固定在 73928b69f2773bed524afdec064211f1eb5d675a。候選的 EXE、ZIP、依賴和測試證據以同一次 GNU CI 為準。

## 建置與驗證

在 Linux x86_64 裝好 Rust 1.100.0 與 x86_64-pc-windows-gnu 目標後，執行 `bash scripts/build-windows-gnu-audit.sh <新的輸出 ZIP 路徑>`。腳本逐一核對 Debian MinGW-w64 12.0.0-5、GCC 14.2.0-19+27+b1 和 binutils 2.44-3+12+b1 的套件雜湊，再解在隔離暫存目錄。它從凍結提交解出兩份不同來源、不同輸出路徑，編譯並驗證兩個 EXE 完全相同。兩份 GNU 連結圖的有效執行階段物件也需一致。

`evidence/gnu-candidate.json` 列出 EXE 與包內各檔 SHA-256、PE x64 匯入、簽章表與通知範圍。 `evidence/gnu-runtime-members.json` 列出 GNU 連結圖有效區段的工具鏈物件和封存庫指紋。工具鏈程式、.a、DLL、PDB 及原始連結路徑留在建置環境。

Windows Server 原生驗收從下載 ZIP 解出並核對 SHA-256，再執行實際 v0.1.1 工作區副本升級、About 和完整 GUI 流程。乾淨 Windows 10／11 標準帳戶、完全斷網並免安裝的驗收另用 `qa/windows-client-offline/RC-CHECKLIST.md`，填入同一 GNU CI 的實際指紋。驗收前初始狀態為未執行。

## 隨附通知

- `licenses/gnu/` 收錄實際 Debian 工具鏈套件的 MinGW-w64、GCC 版權文件，及 GCC Runtime Library Exception 3.1 與 GPLv3 正文。GCC 例外的適用範圍是符合條件的編譯後目標碼。[GCC Runtime Library Exception](https://www.gnu.org/licenses/gcc-exception-3.1.html)
- `licenses/rust-toolchain/` 保存同版 Rust 標準函式庫版權文件及其本機連結所指的授權文件。 `licenses/crates-io/` 的通知來自經 Cargo.lock 驗證的目標套件封存檔。沒有符合通知檔名的個別套件須核對其宣告與原始檔。 `licenses/pdf-assets/` 保存內嵌 PDF 素材授權。
- PE 匯入清單用於辨識系統 DLL 及可能需另行安裝的執行階段。Microsoft 系統 DLL 由 Windows 提供。這份稽核包的授權範圍與實測結果應一併審閱。

## 凍結的 MSVC 候選

原候選 EXE SHA-256 為 `4dce41cfa1902b26cddf8b4725d026ae87e7a203cb235526bd5741d02bfabc4d`，內層 ZIP 為 `98636cb1a4f5e638ecd5f227f89c5bb27b6076c3b7f0eda9bec76419c9ae19dc`。同次連結有 592 筆可辨認行格式的 Loaded，437 筆可歸屬，包含 302 筆 CRT、123 筆系統匯入庫、9 筆 registry 與 3 筆 Rust 工具鏈。其餘 155 筆缺家族或成員的安全分類，或兩者都缺。私有原始記錄已清除，收據沒有保留逐筆名單。[原始供應鏈證據](https://github.com/yung13yubabie/paperroom/actions/runs/45657125258/artifacts/15043352593)

官方 SDK 26100 系列授權原文援引 REDIST，允許符合條件的 UCRT 靜態物件隨程式散布。放入 EXE 的許可應和單獨交付 .lib 區分，並核對託管建置實際取得的 SDK 條款。VS 2022 Enterprise 原文第 5 節援引 VS REDIST。目前缺少這次託管建置給發行者的權利文件、涵蓋 `libcmt.lib` 和 `libvcruntime.lib` 成員的適用條款，以及要轉交的通知。部署文件可用來辨識技術方式。[SDK 授權原文](https://aka.ms/WinSDKLicenseURL) · [SDK REDIST](https://learn.microsoft.com/en-us/legal/windows-sdk/redist) · [VS 2022 原文](https://visualstudio.microsoft.com/license-terms/vs2022-ga-proenterprise/) · [VS REDIST](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)
