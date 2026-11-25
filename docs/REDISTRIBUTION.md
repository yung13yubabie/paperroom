# 第三方元件與再發佈盤點

此文件是下一版打包前的來源紀錄，依 v0.1.2 候選鎖檔、本機 Cargo registry、Windows x64 依賴樹、連結 map 及既有 v0.1.1 ZIP 檢查。v0.1.2 沿用相同的 registry 鎖定套件，並新增直接建置依賴 `cc`。此清單不是法律意見；套件是否實際連入二進位與授權條件，應按最終建置及實際發佈內容再次核對。本專案自身授權尚未指定，本文件不替著作權人選擇授權條款。

## 現有檔案

v0.1.1 Windows ZIP 的 `licenses/DEPENDENCIES.txt` 列出 Windows 依賴，並附個別授權檔、Rust 工具鏈隨附的版權與授權資料以及 MinGW-w64 發行套件的 copyright 檔。直接讀取發行 ZIP 後確認清單標題為 0.1.1；本機未打包的舊副本標為 0.1.0，不能用來判斷已發佈品。下一版仍應隨最終依賴重生清單。獨立 EXE 本身不包含旁附檔；若單獨發佈，應同時提供容易取得的第三方通知及授權文字。已檢查的 v0.1.1 source ZIP 沒有 `licenses/`；原始碼套件不含 registry 第三方原始碼，但應指向對應的通知與鎖檔，若日後附 vendored 原始碼，必須一併保留各原始授權及通知。

## 依賴與建置

- `Cargo.lock` 列出多平台套件；並非每一筆都用於 Windows x64。針對該目標的依賴樹中，`pulldown-cmark`、`rusqlite`、`zip` 和 `windows-sys` 是直接依賴；`cc`、`pkg-config`、`vcpkg` 及巨集套件等參與建置或程式碼產生，不能只憑鎖檔推定全部進入 EXE。各套件版本、registry 來源、宣告授權與目標樹角色見[依賴清單](third-party/dependency-inventory.tsv)；授權以各來源檔原文為準。
- SQLite 3.54.1 由 `libsqlite3-sys` 的 bundled 功能編入。SQLite 官方將其交付程式碼[奉獻至公有領域](https://www.sqlite.org/copyright.html)；Rust 包裝層 `libsqlite3-sys` 與 `rusqlite` 分別保留其 MIT 授權，不能把整個依賴樹當成 SQLite 公有領域。`unicode-ident` 同時宣告 Unicode-3.0 條件，既有 ZIP 包含其 Unicode 授權檔。
- Rust 標準函式庫及編譯器可能攜帶自己的第三方材料；既有 ZIP 附有工具鏈 `COPYRIGHT-library.html` 和授權檔。應以實際工具鏈版本更新這些檔案，而非只複製 crate 授權。
- MinGW-w64 用於 Windows 交叉編譯；既有 ZIP 附發行套件 copyright 資料。Windows PE 匯入表使用系統 DLL，包括 `msvcrt.dll`，並非將 Windows DLL 一併打包。對 v0.1.2 GNU 候選額外產生連結 map 的重建結果，與候選 EXE 的 SHA-256 `b0f9b1dd2bd4d041fe021d4f01772f40d69bc4430457a0dee1c747b4b806382e` 完全一致。非空執行階段物件包括 MinGW-w64 12.0.0-5 的 `crt2.o`、`libmingw32.a` 18 個成員及 `libmingwex.a` 2 個成員；GCC 14.2.0-19+27+b1 的 `crtbegin.o`、`crtend.o` 及 `libgcc_eh.a(unwind-seh.o)`。去除建置路徑的[物件清單](third-party/gnu-runtime/live-runtime-objects.tsv)可供查核。`libkernel32.a` 與 `libmsvcrt.a` 提供系統 DLL 匯入 stub，不隨包發佈系統 DLL；未見 libwinpthread 或 libgcc DLL。原始 [MinGW 通知](third-party/gnu-runtime/mingw-w64-copyright.txt)、[GCC 套件通知](third-party/gnu-runtime/gcc-mingw-w64-base-copyright.txt)、[GPL-3.0-or-later](third-party/gnu-runtime/GPL-3.0-or-later.txt)及 [GCC Runtime Library Exception 3.1](third-party/gnu-runtime/GCC-exception-3.1.txt) 已保留於候選。此證據限於 GNU 產物；GitHub 託管 MSVC 建置的 CRT 實際組成與再發佈授權尚未檢查，不能套用此結論。

## 圖片與字型

`docs/screenshots/first-launch.png`、`chinese-search.png` 是 Wine 測試中實際擷取的應用畫面，測試使用 Noto Sans CJK TC 替代字型；PNG 是畫面，發行包內未見字型檔。程式請求 Microsoft JhengHei UI；[Microsoft 字型說明](https://learn.microsoft.com/en-us/typography/font-list/microsoft-jhenghei)列明它由 Windows 提供及其再散布資訊。Wine 測試使用的 [Noto CJK](https://github.com/notofonts/noto-cjk) 套件所附授權為 SIL OFL 1.1；測試環境的套件 copyright 文件列出原始來源及授權。Windows 實際顯示仍依可用字型與替代機制而定，不應將 Wine 截圖視為 Windows 字型實測。若未來一併發佈字型檔，需另核對該字型版本與授權，並附要求的授權文字。

下版的 `docs/storage-layout.png` 是重新繪製的資料位置示意圖，不是程式截圖；其[可重建 DOT 原始檔](storage-layout.dot)使用文字、方框及箭頭，以 Graphviz 和環境提供的 Noto Sans CJK TC 繪製，未使用外部圖片。v0.1.1 原示意圖的製作來源未在可見資料中確認，因此下版使用此替代圖。畫面也應於公開前確認沒有個人筆記或其他不宜公開內容。

## 下版打包前待辦

1. 由權利人決定是否及如何授權紙間本身；不要把第三方套件授權當成專案授權。
2. 重新以最終鎖檔與 Windows 目標樹產生依賴及授權清單，保留各套件原有的版權、授權文字及適用的 NOTICE；更新版本標題。已有雙授權選項的套件，保留原文及表述，不自行更改作者。
3. 將第三方通知連同 ZIP 發佈，並讓獨立 EXE 的下載頁能直接取得同版本通知；若附第三方原始碼，補上相應原始通知。複核 Rust 標準函式庫及 GNU 執行階段通知是否確實隨最終資產發佈；另查託管 MSVC 建置的 CRT 組成與條款。
4. 核對截圖、示意圖及字型素材來源；若新增字型檔或外部圖片，依其實際授權處理。

鎖檔還包含未出現在該 Windows 目標樹的其他平台項目。Registry 快照中 `r-efi 6.0.0` 與 `rsqlite-vfs 0.2.0` 的套件根目錄未見獨立 LICENSE 檔，但有 Cargo manifest 授權宣告；它們不在目前選定目標樹中，不應直接推定其授權文字已完整。若將來發佈這些套件或將其加入目標，需先回到對應上游版本查明並保存適用授權與通知。

## 來源檔補充核查

已逐一檢查 Windows 選定依賴 registry 套件的巢狀檔名，未發現額外名為 LICENSE、NOTICE、COPYING 或 COPYRIGHT 的巢狀檔；這不是對所有原始碼內嵌聲明的完整保證。`zlib-rs 0.6.8` 套件根目錄的 Zlib 授權聲明列 Trifecta Tech Foundation，已包含於 v0.1.1 ZIP；其 README 稱為 zlib 的純 Rust 實作。未因名稱相同就推斷已包含原版 zlib 或補造第三方作者。若後續完整來源審查或其他建置的連結 map 證實額外組件，須保存其原文通知後再發佈。

## MSVC 候選建置證據

未簽章 Windows CI 另存一份建置證據，不改動只含 EXE 的測試 ZIP。證據包含該 EXE 的 SHA-256、Rust 版本與提交，以及 Cargo 解出的 MSVC 目標和主機建置相依。相依套件列出名稱、版號、授權宣告與授權檔名，省略建置路徑。若該 Rust 工具鏈帶有版權通知檔，證據會記錄相應雜湊。

發佈前仍須核對最終包實際連結的物件、適用的 MSVC／Windows SDK 條款，以及應附的 Rust、相依套件和內嵌素材通知原文。Cargo 相依圖與連結 MAP 計數各自只能證明其中一部分。

## 本次草稿的證據層級與待辦

從最終 MSVC 連結擷取兩種紀錄。MAP 顯示可辨識的符號與物件，`/VERBOSE:LIB` 顯示連結器選入的封存檔成員。草稿只附移除本機路徑後的摘要，並記下能辨認的封存檔、雜湊和工具鏈版本。未辨識的列會保留計數。Cargo 目標相依清單另列為建置輸入。連結最佳化可能移除或合併程式碼，僅靠這些紀錄無法完整歸屬 EXE 的每一個位元。[MSVC 連結器紀錄](https://learn.microsoft.com/en-us/cpp/build/reference/verbose-print-progress-messages?view=msvc-170)

草稿 ZIP 附四份內嵌的 PDF 補充通知與 Rust 標準函式庫版權原文。另從符合 Cargo.lock 雜湊的 crates.io 封存檔採集 Windows 目標相依的原文。清冊會列出沒有找到通知檔名的套件。讀取 Rust 1.100.0 的 `COPYRIGHT-library.html` 時，已核對內文沒有相對授權檔連結。它的外部連結仍需要網路。

從 [草稿 PR](https://github.com/yung13yubabie/paperroom/pull/1) 指定的 CI 取得 GitHub 外層 ZIP，再解開裡面的 `Paperroom-v0.1.2-windows-x64-draft.zip`。以 CI 紀錄核對這兩層 ZIP、內含 EXE 和通知清冊的 SHA-256。草稿可用來核查包內實際檔案。正式發佈仍需確認未辨識組件及這次使用的 MSVC／Windows SDK 的適用條款。[Microsoft 的 Visual C++ 再散布說明](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170)
