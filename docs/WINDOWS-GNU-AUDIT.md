# 紙間 v0.1.2 Windows x64 GNU 預覽

應用程式來源固定為 `9940e2f07380b546c06fbe3437891d58eddf26df`。本版整合 #1–#16，包括 Notion ZIP、Obsidian Vault、JSON Canvas、集合篩選與欄位配置，以及可還原屬性和附件的筆記歷史。操作請見 [快速指南](QUICKSTART.md)。批次屬性編輯留待後續版本。

[整合 PR #17](https://github.com/yung13yubabie/paperroom/pull/17) 記錄下載、SHA-256 及原生測試。請取附通知的 ZIP 和同次建置的 `SHA256SUMS.txt`。先用 `Get-FileHash -Algorithm SHA256` 核對 ZIP，完整解壓後再核對 `Paperroom.exe`。此版本未簽章。乾淨 Windows 10／11 的標準帳戶斷網與免安裝流程仍待實測；可依 [離線驗收清單](../qa/windows-client-offline/RC-CHECKLIST.md) 操作，並填入這次的來源與雜湊。

## 重建與通知

安裝 Rust 1.100.0 與 `x86_64-pc-windows-gnu` 目標，在 Linux x64 的 Git 工作目錄執行：

```sh
bash scripts/build-windows-gnu-audit.sh ./Paperroom-preview.zip 9940e2f07380b546c06fbe3437891d58eddf26df
```

建置固定 Debian MinGW-w64、GCC 與 binutils 的版本和封存雜湊。兩份獨立來源與輸出目錄必須建出位元相同的 EXE，連結圖也必須列出一致的 GNU 與 Rust 有效物件後才打包。`evidence/gnu-candidate.json` 記錄來源、PE 匯入與每個檔案的雜湊。

`licenses/` 附上與這次成品對應的 GNU、Rust、crates.io 套件及 PDF 素材通知。GCC Runtime Library Exception 3.1 涵蓋符合條件的編譯目標碼。原始工具鏈檔、Microsoft 靜態 VC CRT 和 Windows 系統 DLL 都未隨包附帶。正式發行的第三方條款與使用者環境驗收請見 [再散布清冊](REDISTRIBUTION.md)。
