# Microsoft CRT 和 Windows SDK 建置來源

紙間 x64 草稿連結稽核以同一份 EXE 的 SHA-256 為準。包內 `evidence/msvc-link-inventory.json` 記錄工具版本和 MAP 觀察，`evidence/msvc-crt-archives.json` 把可辨識的 MAP 成員與原生連結器搜尋到的指定 CRT 封存檔比對。若相同成員名出現在多個候選，報告會保留不確定性。`evidence/msvc-link-probe.json` 另記原生連結診斷與實際安裝範圍內可見的條款檔指紋。

| 原生封存檔 | Microsoft 說明的用途 | 供應來源 |
| --- | --- | --- |
| `libcmt.lib` | 原生 CRT 啟動及結束程式碼 | Visual C++ 工具集 |
| `libvcruntime.lib` | 編譯器相依的靜態 VCRuntime | Visual C++ 工具集 |
| `libucrt.lib` | 靜態 Universal CRT | Windows SDK 的 `Lib/<版本>/ucrt/x64` |

[Microsoft CRT 函式庫表](https://learn.microsoft.com/en-us/cpp/c-runtime-library/crt-library-features?view=msvc-170)

## 再散布條款來源

- Windows SDK 的官方 REDIST 清單列出正式發佈 SDK 的 `Lib/<版本>/ucrt/x64` 靜態程式庫。該項適用於納入程式建置的未經修改程式碼，並受 SDK 授權條款約束。應將這一項對照同次建置實際使用的 SDK 版本和適用條款。[Windows SDK REDIST](https://learn.microsoft.com/en-us/legal/windows-sdk/redist)
- Visual Studio 2022 Enterprise 的原始條款在「Distributable Code」下引用 Distributable List，另有散布要求及限制。官方 VS 2022 網頁列有 VC Redistributable、安裝用合併模組和 redist 目錄。它沒有逐名列出本表的 `libcmt.lib` 和 `libvcruntime.lib`，因此仍要核對實際工具集適用條款。公開條款 DOCX 核對 SHA-256 為 `9c0cd52b20db9d081854c75bd1b50c75514b8f8cb09c8cad15e89d90b97b5bf3`。[Visual Studio Enterprise 條款](https://visualstudio.microsoft.com/license-terms/vs2022-ga-proenterprise/) · [VS 2022 REDIST](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution)
- 主機的 Visual Studio 版本與 SDK 路徑用於核對建置來源。發佈者仍需確認適用的使用、授權與散布條件。[GitHub 託管執行器](https://docs.github.com/en/actions/concepts/runners/github-hosted-runners)

這是針對紙間草稿的來源與條款指引。草稿包只附紙間執行檔、列明的通知與整理過的證據。Microsoft 的工具、靜態封存檔、原始連結紀錄及整套 SDK 條款留在各自來源。完整再散布審查仍待完成。
