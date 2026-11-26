# 紙間 `0118861` Windows GNU 草稿的再散布查核

查核以 GitHub 重新下載的工件及公開固定來源為準。下文列出的發行判斷由發行者確認。

| 物件 | SHA-256 或提交 |
| --- | --- |
| [應用程式來源](https://github.com/yung13yubabie/paperroom/tree/01188615f779a1ba16c11e2cf5ae467aa5dc22b5) | `01188615f779a1ba16c11e2cf5ae467aa5dc22b5` |
| [外層 GitHub ZIP](https://github.com/yung13yubabie/paperroom/actions/runs/45806067708/artifacts/15109342382) | `b7ec30a048e69c180194b1ea7ddd9d26fc40f912699bdba0c15592dd0f483f69` |
| 內層 `Paperroom-windows-x64-gnu-draft.zip` | `029c77fc67ba1eaa547fd6b41d9c1c4861b942d25a14ae65d4d95c87d5be7c25` |
| [獨立 EXE 工件](https://github.com/yung13yubabie/paperroom/actions/runs/45806067708/artifacts/15109851758) | 外層 `79fe308e044e2035ac28a9fab0e7d618de77a2e15357b4483fac6d5d36346404` |
| 兩份工件內的 `Paperroom.exe` | `22d2ca9607daaf121e27f0f7c084b50d4ae947abdae853fcf9c42961ce44f8bd` |

外層和內層 ZIP 均通過 CRC。兩份 EXE 同位元。內層共 199 個項目，198 個有清冊內各自的 SHA-256，第 199 個為清冊本身。`licenses/` 內有 188 個項目，其中 177 項是與鎖定 crate 原檔逐位元相符的通知。原包 `GNU-AUDIT.md` 的 GCC 歷史例子引用另一提交；本頁所有數字取自本次固定下載物。 [封裝與兩次獨立建置](https://github.com/yung13yubabie/paperroom/actions/runs/45806067708/job/139861532698)

## 實際封入項目與條款原文

| 項目 | 實際封入和來源對照 | 現有通知、補件或交付條件 |
| --- | --- | --- |
| Paperroom 0.1.2 | Rust 主程式和版本資源。固定來源可下載，`Cargo.lock` 與包內清冊一致。 | 固定來源根目錄及 `Cargo.toml` 未提供專案授權。由權利人確定應用程式的散布條款和授權範圍。 |
| 91 個 crates.io 目標依賴 | 60 個有非空 GNU 連結成員，31 個只在目標依賴圖觀察到。逐一核對 `.crate` 原檔、`Cargo.lock` 和通知；附錄另含 21 個只在主機建置圖的 crate。 | 90 個有原封存檔的通知。`alloc-stdlib 0.2.4` 的 BSD-3-Clause 原文自封存 VCS 記錄所指的上游取得，與 ZIP 及 EXE 內文字相符。 |
| PDFium／Foxit 字型 | `hayro-font 0.5.0` 的 14 個 PFB 原始位元組均找到於 EXE。 | BSD 形式的二進位散布條款要求版權、條件及免責通知。ZIP 含 `licenses/pdf-assets/pdfium-foxit.txt` 和 crate 內原檔。EXE 還可找到通知本文與 Foxit 額外原始碼署名。 |
| Adobe CMaps | `hayro-cmap 0.1.1` 的 `cmaps.brotli` 原始位元組在 EXE。 | Adobe 的二進位通知條款要求版權、條件及免責。ZIP 的 `licenses/pdf-assets/adobe-cmap.txt` 與 crate 原檔和 EXE 文字比對一致。 |
| ICC 色彩檔 | `hayro-interpret 0.9.0` 與 `hayro-jpeg2000 0.5.0` 共引用 5 筆，合計 3 組不同內容。CGATS001、LAB、ProPhoto 都在 EXE 找到。 | 原 crate 的兩份 CC0 文本相同，包內與 EXE 都有 `icc-cc0.txt` 的文字。保留已核對的來源註記。 |
| SQLite 3.54.1 | `libsqlite3-sys 0.40.0` 的 `sqlite3.o` 進入最終 rlib 成員，EXE 可找到與鎖定 amalgamation 相同的 source ID。 | `libsqlite3-sys` 及 `rusqlite` 的 MIT 文本在 ZIP。SQLite 上游核心來源的公有領域聲明另有原文。 [SQLite 聲明](https://www.sqlite.org/copyright.html) |
| GCC 14.2.0 | `crtbegin.o`、`crtend.o`、`libgcc_eh.a(unwind-seh.o)` 三項靜態輸入與固定 Debian 二進位來源相符。上游標籤對應檔案有 GCC Runtime Library Exception 說明。 | ZIP 內 GPLv3、Runtime Library Exception 3.1 與上游標籤原件同位元。Debian cross package copyright 亦隨包。正式發行須確認實際流程及獨立模組符合例外條件。 [FSF 原條款](https://www.gnu.org/licenses/gcc-exception-3.1.html) |
| MinGW-w64 12.0.0-5 | 147 筆，有 65 筆是靜態物件或靜態成員，82 筆含 `.idata$` 匯入相關段落。`libmsvcrt.a` 同時有靜態相容程式與匯入成員。 | ZIP 有同版 Debian copyright。上游另建議將 `COPYING.MinGW-w64-runtime.txt` 隨靜態連結產物交付，固定 ZIP 沒有這份獨立原文，已在本草稿補件。ZPL-2.1 二進位條款要求重印版權、條件與免責，其中部分 CRT 原檔另列公有領域說明。 |
| Rust 1.100.0 sysroot | 14 個 archive 的 17 筆靜態成員。逐檔雜湊與固定 Rust 工具鏈相符。 | ZIP 的 `COPYRIGHT-library.html` 與固定工具鏈同位元。裡面有庫內歸屬與第三方通知，庫內通則及例外涉及 MIT、Apache 2.0、Unicode、LLVM 例外等。已補同修訂 `LICENSES` 原文。 |
| Windows 系統 DLL | `objdump -p` 讀得 18 個直接描述符，折疊重複及大小寫後為 14 個 DLL。延遲匯入及 Authenticode 憑證表皆空。ZIP 只有 `Paperroom.exe` 一個 Windows 二進位，未附 DLL、`.lib` 或 `.a`。 | `msvcrt.dll` 屬直接系統匯入，MinGW 靜態成員另查。清冊和 ZIP 顯示的是 GNU 這一路徑，正式包仍以自己的實際內容判定。 |

原始檔框架及例外範圍以版本發布物為準。SQLite source ID 為 `2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76`。PDF 資產源於 [固定 Hayro 修訂](https://github.com/LaurenzV/hayro/tree/3f3825c3e50bb41ed6ec6c3f4b6007978bd13def)，可由附錄中的已驗 `.crate` 重取原檔。alloc-stdlib 的 [固定上游通知](https://github.com/dropbox/rust-alloc-no-stdlib/blob/ae42d22078b98549e987d2f03d12df7b984fde47/LICENSE) 也已重新下載並比對。

## 這份固定包應隨附的補件

下表是由原始版工具鏈／源碼抽出的原文，可以與既有 ZIP 的 `licenses/rust-toolchain/COPYRIGHT-library.html` 及 MinGW Debian copyright 一起交付。Rust MIT 標準原文保留上游模板，其權利人及庫內各例外依上游 HTML 的對應歸屬。固定候選以首表雜湊辨認。

| 原文 | SHA-256 |
| --- | --- |
| [COPYING.MinGW-w64-runtime.txt](third-party/windows-gnu-0118861/COPYING.MinGW-w64-runtime.txt) | `e9b2dc02451ea29092a1f25fa0f3c07207ed421f1807dffb0c4e6dce69dee7bd` |
| [Apache-2.0.txt](third-party/windows-gnu-0118861/Apache-2.0.txt) | `074e6e32c86a4c0ef8b3ed25b721ca23aca83df277cd88106ef7177c354615ff` |
| [MIT.txt](third-party/windows-gnu-0118861/MIT.txt) | `b85dcd3e453d05982552c52b5fc9e0bdd6d23c6f8e844b984a88af32570b0cc0` |
| [Unicode-3.0.txt](third-party/windows-gnu-0118861/Unicode-3.0.txt) | `f5062c9a188d81dfe66b56db4182dcf9e4b17c0d9b0d311a8e20b3a1b075c443` |
| [LLVM-exception.txt](third-party/windows-gnu-0118861/LLVM-exception.txt) | `e34c58338bd89d43e709e226610d8f32b3e3c47f4ad9a99a8dc1d4ac7842488e` |
| [BSD-2-Clause.txt](third-party/windows-gnu-0118861/BSD-2-Clause.txt) | `f32fb3b417a194167cfad068223fc975ba96c5960513a10f66a3c28720aec1df` |

## 來源取得方式

- Paperroom 的 [固定來源 tar.gz](https://codeload.github.com/yung13yubabie/paperroom/tar.gz/01188615f779a1ba16c11e2cf5ae467aa5dc22b5) 已實際下載，這次 SHA-256 為 `a39c9a5f78db72a20030914a5c2c34327796f1f27b2df54051935ecd6cf38eff`。含 `Cargo.lock`、建置配方與應用程式程式碼。固定 Windows ZIP 內沒有源碼。實際發行說明可用同提交來源連結並保留取得方式。
- 附錄的 crate 名稱連到對應版本的官方 `.crate` 下載。91 個目標封存檔已全部讀取並對照 lock。每列在 `evidence/gnu-dependency-redistribution.json` 與 `evidence/registry-notices.json` 有鎖定來源及通知雜湊。SQLite amalgamation 可從 `libsqlite3-sys` 取出。 [SQLite 原始碼取得](https://www.sqlite.org/getthecode.html)
- MinGW 的 [Debian 12.0.0-5 來源描述檔](https://deb.debian.org/debian/pool/main/m/mingw-w64/mingw-w64_12.0.0-5.dsc) 已下載，原始 tar 與 Debian patch tar 都符合其中 SHA-256。GCC cross package 的 `Built-Using` 指向 GCC 14.2.0-19。具體檔名和 SHA-256 在 [Debian GCC 14.2.0-19 來源描述檔](https://deb.debian.org/debian/pool/main/g/gcc-14/gcc-14_14.2.0-19.dsc)；兩個來源下載網址可達，本輪對 GCC 的實際取樣為描述檔、上游三個執行階段來源與條款原文。 [GCC 14.2.0 執行階段](https://github.com/gcc-mirror/gcc/tree/releases/gcc-14.2.0/libgcc) · [Debian GCC 14 版權](https://metadata.ftp-master.debian.org/changelogs/main/g/gcc-14/gcc-14_14.2.0-19_copyright)
- Rust 固定 `rustc` 修訂為 `0ef1bc386059596dada8fb65b50425aab2ea637a`。庫內可看對應的標準庫目錄。各 sysroot 的外部依賴來源從 `COPYRIGHT-library.html` 的對應 crate 版本取得。 [Rust 固定來源](https://github.com/rust-lang/rust/tree/0ef1bc386059596dada8fb65b50425aab2ea637a/library)

## 發行者需確認

1. 確定 Paperroom 的發行權與對收件者適用的專案條款。若提供獨立 EXE，下載入口也要交付相配的完整第三方通知與必要的來源取得方式。已核對四份 PDF／依賴通知內嵌於 EXE，其他通知主要由 ZIP 和本頁補件提供。
2. 對雙授權或多授權依賴決定採用範圍，保留須隨附的版權、授權文字與歸屬。MIT 保留版權與許可通知，Apache-2.0 依第 4 節保留授權、適用來源通知、修改註記及上游有提供的 NOTICE，BSD-3-Clause 的二進位條款需要版權、條件和免責。Unicode-3.0 的原通知須隨軟體或文件提供；Zlib 的原文件條件另載來源說明與修改識別。`OR` 與 `AND` 的條件分開核對。 [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) · [Unicode-3.0](https://www.unicode.org/license.txt) · [Zlib](https://www.zlib.net/zlib_license.html)
3. 驗證操作配方符合 GCC 例外的合格編譯流程。現有配方含 Rust／LLVM、固定 GCC 及 GNU ld，關閉跨 crate LTO 並使用 `-static-libgcc`。若變動編譯、連結或選擇另散布工具鏈元件，依實際產物重新核對。 [GCC 例外原文](https://www.gnu.org/licenses/gcc-exception-3.1.html)
4. 發行前把已確認的 ZIP、補件和來源入口一起再下載核對。已列入的補件與雜湊供發行者審閱，本草稿沒有正式核准紀錄。

## 乾淨 Windows 10／11 交接

可用的已授權環境只有 Linux 雲端及目前離線的桌面裝置。本次沒有可執行驗收的乾淨 Windows 10 或 11 標準帳戶。Windows Server 的原生操作與合成來源標記測試各自有紀錄。

兩個作業系統各用一台乾淨、隔離、標準帳戶客體。保留正常下載與 Windows 解壓的來源標記收據，斷網前後依 [QA 手冊](../qa/windows-client-offline/GNU-CANDIDATE.md) 記錄。若普通開啟顯示提示或阻擋，保存該次結果並停止該路徑。用下列固定參數備妥腳本展開的查核資料，Windows 11 改用 `-ExpectedFamily 'Windows 11'` 和一個新目的地。

```powershell
& .\qa\windows-client-offline\stage-gnu-candidate.ps1 -OuterZip .\download.zip `
  -OuterSha256 b7ec30a048e69c180194b1ea7ddd9d26fc40f912699bdba0c15592dd0f483f69 `
  -InnerSha256 029c77fc67ba1eaa547fd6b41d9c1c4861b942d25a14ae65d4d95c87d5be7c25 `
  -ExeSha256 22d2ca9607daaf121e27f0f7c084b50d4ae947abdae853fcf9c42961ce44f8bd `
  -SourceCommit 01188615f779a1ba16c11e2cf5ae467aa5dc22b5 -RunId 45806067708 -ArtifactId 15109342382 `
  -ExpectedFamily 'Windows 10' -Destination .\paperroom-client-w10
```

保存 `stage.json`、正常下載的來源標記收據、四個檢查點及 [驗收紀錄](../qa/windows-client-offline/EVIDENCE.md)。客戶端資格、預設與可攜工作區、關閉再開、備份另開和離線操作均需實測。 [Windows Server 操作](https://github.com/yung13yubabie/paperroom/actions/runs/45806067708) · [合成標記 QA](https://github.com/yung13yubabie/paperroom/actions/runs/45815092756/job/139892649832)

## 附錄一　逐一核對的 Cargo 依賴

ZIP 內的 crate 原文位於 `licenses/crates-io/<名稱>/<版本>/`。連結欄的「有成員」指包內原始最終 GNU link map 清冊觀察到非空成員。「只在目標圖」的項目仍保留通知，泛型單體化、最佳化及巨集展開可能改變可歸屬方式。只在主機建置圖的版本可由連結取得；本輪未逐一下載它們的封存檔，也未把圖中角色當成對生成輸出的授權判定。

`brotli-decompressor 5.0.3` 的原裝 `Cargo.toml` 寫 `BSD-3-Clause/MIT`，包內 Cargo metadata 欄位空白；ZIP 的 `LICENSE` 已逐位元對上該版本的 BSD-3-Clause 文本。`memchr` 和 `byteorder-lite` 的 `UNLICENSE` 沒收入固定包，兩者的 MIT 原文都有；發行者若選用另一授權途徑，須依其實際原文辦理。`linebender_resource_handle` 的 `AUTHORS` 在原封存檔，兩份授權已隨包；該檔是貢獻者清單，本輪維持原條款與查核證據供核對。

| 來源與版本 | 連結證據 | 原始封存宣告或清冊 | ZIP 內原文 |
| --- | --- | --- | --- |
| [ab_glyph_rasterizer 0.1.10](https://static.crates.io/crates/ab_glyph_rasterizer/ab_glyph_rasterizer-0.1.10.crate) | 有成員 | Apache-2.0 | `LICENSE` |
| [adler2 2.0.1](https://static.crates.io/crates/adler2/adler2-2.0.1.crate) | 只在目標圖 | 0BSD OR MIT OR Apache-2.0 | `LICENSE-0BSD`, `LICENSE-APACHE`, `LICENSE-MIT` |
| [alloc-no-stdlib 2.0.4](https://static.crates.io/crates/alloc-no-stdlib/alloc-no-stdlib-2.0.4.crate) | 只在目標圖 | BSD-3-Clause | `LICENSE` |
| [alloc-stdlib 0.2.4](https://static.crates.io/crates/alloc-stdlib/alloc-stdlib-0.2.4.crate) | 只在目標圖 | BSD-3-Clause | `licenses/pdf-assets/alloc-stdlib.txt` |
| [arraydeque 0.5.1](https://static.crates.io/crates/arraydeque/arraydeque-0.5.1.crate) | 只在目標圖 | MIT/Apache-2.0 | `LICENSE` |
| [arrayvec 0.7.8](https://static.crates.io/crates/arrayvec/arrayvec-0.7.8.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [autocfg 1.5.1](https://static.crates.io/crates/autocfg/autocfg-1.5.1.crate) | 只在主機建置圖 | Apache-2.0 OR MIT | — |
| [bitflags 2.13.2](https://static.crates.io/crates/bitflags/bitflags-2.13.2.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [brotli 8.0.4](https://static.crates.io/crates/brotli/brotli-8.0.4.crate) | 有成員 | BSD-3-Clause AND MIT | `LICENSE.BSD-3-Clause`, `LICENSE.MIT` |
| [brotli-decompressor 5.0.3](https://static.crates.io/crates/brotli-decompressor/brotli-decompressor-5.0.3.crate) | 有成員 | BSD-3-Clause/MIT（原封存），清冊空白 | `LICENSE` |
| [bytemuck 1.25.2](https://static.crates.io/crates/bytemuck/bytemuck-1.25.2.crate) | 有成員 | Zlib OR Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE-ZLIB` |
| [bytemuck_derive 1.12.1](https://static.crates.io/crates/bytemuck_derive/bytemuck_derive-1.12.1.crate) | 只在主機建置圖 | Zlib OR Apache-2.0 OR MIT | — |
| [byteorder-lite 0.1.0](https://static.crates.io/crates/byteorder-lite/byteorder-lite-0.1.0.crate) | 只在目標圖 | Unlicense OR MIT | `LICENSE-MIT` |
| [cc 1.11.3](https://static.crates.io/crates/cc/cc-1.11.3.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [cfg-if 1.0.5](https://static.crates.io/crates/cfg-if/cfg-if-1.0.5.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [color 0.3.3](https://static.crates.io/crates/color/color-0.3.3.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [constant_time_eq 0.6.1](https://static.crates.io/crates/constant_time_eq/constant_time_eq-0.6.1.crate) | 有成員 | CC0-1.0 OR MIT-0 OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-CC0`, `LICENSE-MIT0` |
| [crc32fast 1.5.5](https://static.crates.io/crates/crc32fast/crc32fast-1.5.5.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [equivalent 1.0.3](https://static.crates.io/crates/equivalent/equivalent-1.0.3.crate) | 只在目標圖 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [euclid 0.22.14](https://static.crates.io/crates/euclid/euclid-0.22.14.crate) | 只在目標圖 | MIT OR Apache-2.0 | `COPYRIGHT`, `LICENSE-APACHE`, `LICENSE-MIT` |
| [fallible-iterator 0.3.0](https://static.crates.io/crates/fallible-iterator/fallible-iterator-0.3.0.crate) | 只在目標圖 | MIT/Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [fallible-streaming-iterator 0.1.9](https://static.crates.io/crates/fallible-streaming-iterator/fallible-streaming-iterator-0.1.9.crate) | 只在目標圖 | MIT/Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [fastrand 2.5.0](https://static.crates.io/crates/fastrand/fastrand-2.5.0.crate) | 只在主機建置圖 | Apache-2.0 OR MIT | — |
| [fdeflate 0.3.7](https://static.crates.io/crates/fdeflate/fdeflate-0.3.7.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [fearless_simd 0.7.0](https://static.crates.io/crates/fearless_simd/fearless_simd-0.7.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [fearless_simd 1.2.0](https://static.crates.io/crates/fearless_simd/fearless_simd-1.2.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [find-msvc-tools 0.1.15](https://static.crates.io/crates/find-msvc-tools/find-msvc-tools-0.1.15.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [flate2 1.1.11](https://static.crates.io/crates/flate2/flate2-1.1.11.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [foldhash 0.2.0](https://static.crates.io/crates/foldhash/foldhash-0.2.0.crate) | 有成員 | Zlib | `LICENSE` |
| [font-types 0.12.7](https://static.crates.io/crates/font-types/font-types-0.12.7.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [getrandom 0.4.3](https://static.crates.io/crates/getrandom/getrandom-0.4.3.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [glifo 0.4.0](https://static.crates.io/crates/glifo/glifo-0.4.0.crate) | 只在目標圖 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [guillotiere 0.7.0](https://static.crates.io/crates/guillotiere/guillotiere-0.7.0.crate) | 有成員 | MIT/Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hashbrown 0.17.2](https://static.crates.io/crates/hashbrown/hashbrown-0.17.2.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hashlink 0.12.2](https://static.crates.io/crates/hashlink/hashlink-0.12.2.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro 0.9.0](https://static.crates.io/crates/hayro/hayro-0.9.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro-ccitt 0.5.0](https://static.crates.io/crates/hayro-ccitt/hayro-ccitt-0.5.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro-cmap 0.1.1](https://static.crates.io/crates/hayro-cmap/hayro-cmap-0.1.1.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT`, `assets/LICENSE.txt` |
| [hayro-font 0.5.0](https://static.crates.io/crates/hayro-font/hayro-font-0.5.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT`, `assets/LICENSE_FOXIT` |
| [hayro-interpret 0.9.0](https://static.crates.io/crates/hayro-interpret/hayro-interpret-0.9.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro-jbig2 0.4.0](https://static.crates.io/crates/hayro-jbig2/hayro-jbig2-0.4.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro-jpeg2000 0.5.0](https://static.crates.io/crates/hayro-jpeg2000/hayro-jpeg2000-0.5.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT`, `assets/LICENSE.txt` |
| [hayro-postscript 0.2.0](https://static.crates.io/crates/hayro-postscript/hayro-postscript-0.2.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [hayro-syntax 0.9.0](https://static.crates.io/crates/hayro-syntax/hayro-syntax-0.9.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [image 0.25.10](https://static.crates.io/crates/image/image-0.25.10.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [indexmap 2.14.2](https://static.crates.io/crates/indexmap/indexmap-2.14.2.crate) | 只在目標圖 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [itoa 1.0.18](https://static.crates.io/crates/itoa/itoa-1.0.18.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [kurbo 0.13.1](https://static.crates.io/crates/kurbo/kurbo-0.13.1.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [libsqlite3-sys 0.40.0](https://static.crates.io/crates/libsqlite3-sys/libsqlite3-sys-0.40.0.crate) | 有成員 | MIT | `LICENSE` |
| [linebender_resource_handle 0.1.1](https://static.crates.io/crates/linebender_resource_handle/linebender_resource_handle-0.1.1.crate) | 只在目標圖 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [log 0.4.35](https://static.crates.io/crates/log/log-0.4.35.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [memchr 2.8.3](https://static.crates.io/crates/memchr/memchr-2.8.3.crate) | 有成員 | Unlicense OR MIT | `COPYING`, `LICENSE-MIT` |
| [miniz_oxide 0.8.9](https://static.crates.io/crates/miniz_oxide/miniz_oxide-0.8.9.crate) | 只在目標圖 | MIT OR Zlib OR Apache-2.0 | `LICENSE`, `LICENSE-APACHE.md`, `LICENSE-MIT.md`, `LICENSE-ZLIB.md` |
| [miniz_oxide 0.9.1](https://static.crates.io/crates/miniz_oxide/miniz_oxide-0.9.1.crate) | 只在目標圖 | MIT OR Zlib OR Apache-2.0 | `LICENSE`, `LICENSE-APACHE.md`, `LICENSE-MIT.md`, `LICENSE-ZLIB.md` |
| [moxcms 0.8.1](https://static.crates.io/crates/moxcms/moxcms-0.8.1.crate) | 有成員 | BSD-3-Clause OR Apache-2.0 | `LICENSE-APACHE.md`, `LICENSE.md` |
| [num-traits 0.2.19](https://static.crates.io/crates/num-traits/num-traits-0.2.19.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [once_cell 1.21.4](https://static.crates.io/crates/once_cell/once_cell-1.21.4.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [paperroom 0.1.2](https://github.com/yung13yubabie/paperroom/tree/01188615f779a1ba16c11e2cf5ae467aa5dc22b5) | 程式本體 | 無套件宣告 | — |
| [peniko 0.6.1](https://static.crates.io/crates/peniko/peniko-0.6.1.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [phf 0.13.1](https://static.crates.io/crates/phf/phf-0.13.1.crate) | 只在目標圖 | MIT | `LICENSE` |
| [phf_generator 0.13.1](https://static.crates.io/crates/phf_generator/phf_generator-0.13.1.crate) | 只在主機建置圖 | MIT | — |
| [phf_macros 0.13.1](https://static.crates.io/crates/phf_macros/phf_macros-0.13.1.crate) | 只在主機建置圖 | MIT | — |
| [phf_shared 0.13.1](https://static.crates.io/crates/phf_shared/phf_shared-0.13.1.crate) | 只在目標圖 | MIT | `LICENSE` |
| [pic-scale 0.7.14](https://static.crates.io/crates/pic-scale/pic-scale-0.7.14.crate) | 有成員 | BSD-3-Clause OR Apache-2.0 | `LICENSE-APACHE.md`, `LICENSE.md` |
| [pkg-config 0.3.34](https://static.crates.io/crates/pkg-config/pkg-config-0.3.34.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [png 0.18.1](https://static.crates.io/crates/png/png-0.18.1.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [polycool 0.4.0](https://static.crates.io/crates/polycool/polycool-0.4.0.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [proc-macro2 1.0.107](https://static.crates.io/crates/proc-macro2/proc-macro2-1.0.107.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [pulldown-cmark 0.13.6](https://static.crates.io/crates/pulldown-cmark/pulldown-cmark-0.13.6.crate) | 有成員 | MIT | `LICENSE` |
| [pxfm 0.1.30](https://static.crates.io/crates/pxfm/pxfm-0.1.30.crate) | 有成員 | BSD-3-Clause OR Apache-2.0 | `LICENSE-APACHE.md`, `LICENSE.md` |
| [quote 1.0.47](https://static.crates.io/crates/quote/quote-1.0.47.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [read-fonts 0.41.0](https://static.crates.io/crates/read-fonts/read-fonts-0.41.0.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [read-fonts 0.46.0](https://static.crates.io/crates/read-fonts/read-fonts-0.46.0.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [rusqlite 0.43.0](https://static.crates.io/crates/rusqlite/rusqlite-0.43.0.crate) | 有成員 | MIT | `LICENSE` |
| [rustc-hash 2.1.3](https://static.crates.io/crates/rustc-hash/rustc-hash-2.1.3.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [rustversion 1.0.23](https://static.crates.io/crates/rustversion/rustversion-1.0.23.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [serde_core 1.0.229](https://static.crates.io/crates/serde_core/serde_core-1.0.229.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [serde_json 1.0.151](https://static.crates.io/crates/serde_json/serde_json-1.0.151.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [shlex 2.0.1](https://static.crates.io/crates/shlex/shlex-2.0.1.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [simd-adler32 0.3.10](https://static.crates.io/crates/simd-adler32/simd-adler32-0.3.10.crate) | 有成員 | MIT | `LICENSE.md` |
| [siphasher 1.0.4](https://static.crates.io/crates/siphasher/siphasher-1.0.4.crate) | 只在目標圖 | MIT OR Apache-2.0 | `COPYING`, `LICENSE-APACHE`, `LICENSE-MIT` |
| [skrifa 0.44.0](https://static.crates.io/crates/skrifa/skrifa-0.44.0.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [skrifa 0.49.0](https://static.crates.io/crates/skrifa/skrifa-0.49.0.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [smallvec 1.16.6](https://static.crates.io/crates/smallvec/smallvec-1.16.6.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [stable_deref_trait 1.2.1](https://static.crates.io/crates/stable_deref_trait/stable_deref_trait-1.2.1.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [syn 2.0.119](https://static.crates.io/crates/syn/syn-2.0.119.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [syn 3.0.10](https://static.crates.io/crates/syn/syn-3.0.10.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [synstructure 0.14.0](https://static.crates.io/crates/synstructure/synstructure-0.14.0.crate) | 只在主機建置圖 | MIT | — |
| [thiserror 2.0.21](https://static.crates.io/crates/thiserror/thiserror-2.0.21.crate) | 只在目標圖 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [thiserror-impl 2.0.21](https://static.crates.io/crates/thiserror-impl/thiserror-impl-2.0.21.crate) | 只在主機建置圖 | MIT OR Apache-2.0 | — |
| [typed-path 0.13.0](https://static.crates.io/crates/typed-path/typed-path-0.13.0.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [unicase 2.10.0](https://static.crates.io/crates/unicase/unicase-2.10.0.crate) | 有成員 | MIT OR Apache-2.0 | `LICENSE-APACHE`, `LICENSE-MIT` |
| [unicode-ident 1.0.27](https://static.crates.io/crates/unicode-ident/unicode-ident-1.0.27.crate) | 只在主機建置圖 | (MIT OR Apache-2.0) AND Unicode-3.0 | — |
| [vcpkg 0.2.15](https://static.crates.io/crates/vcpkg/vcpkg-0.2.15.crate) | 只在主機建置圖 | MIT/Apache-2.0 | — |
| [vello_common 0.3.0](https://static.crates.io/crates/vello_common/vello_common-0.3.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [vello_cpu 0.3.0](https://static.crates.io/crates/vello_cpu/vello_cpu-0.3.0.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [windows-link 0.2.1](https://static.crates.io/crates/windows-link/windows-link-0.2.1.crate) | 只在目標圖 | MIT OR Apache-2.0 | `license-apache-2.0`, `license-mit` |
| [windows-sys 0.59.0](https://static.crates.io/crates/windows-sys/windows-sys-0.59.0.crate) | 只在目標圖 | MIT OR Apache-2.0 | `license-apache-2.0`, `license-mit` |
| [windows-sys 0.61.2](https://static.crates.io/crates/windows-sys/windows-sys-0.61.2.crate) | 有成員 | MIT OR Apache-2.0 | `license-apache-2.0`, `license-mit` |
| [windows-targets 0.52.6](https://static.crates.io/crates/windows-targets/windows-targets-0.52.6.crate) | 只在目標圖 | MIT OR Apache-2.0 | `license-apache-2.0`, `license-mit` |
| [windows_x86_64_gnu 0.52.6](https://static.crates.io/crates/windows_x86_64_gnu/windows_x86_64_gnu-0.52.6.crate) | 只在目標圖 | MIT OR Apache-2.0 | `license-apache-2.0`, `license-mit` |
| [yaml-rust2 0.13.2](https://static.crates.io/crates/yaml-rust2/yaml-rust2-0.13.2.crate) | 有成員 | MIT OR Apache-2.0 | `Apache-LICENSE`, `MIT-LICENSE` |
| [yoke 0.8.4](https://static.crates.io/crates/yoke/yoke-0.8.4.crate) | 只在目標圖 | Unicode-3.0 | `LICENSE` |
| [yoke-derive 0.8.4](https://static.crates.io/crates/yoke-derive/yoke-derive-0.8.4.crate) | 只在主機建置圖 | Unicode-3.0 | — |
| [zerofrom 0.1.8](https://static.crates.io/crates/zerofrom/zerofrom-0.1.8.crate) | 只在目標圖 | Unicode-3.0 | `LICENSE` |
| [zerofrom-derive 0.1.8](https://static.crates.io/crates/zerofrom-derive/zerofrom-derive-0.1.8.crate) | 只在主機建置圖 | Unicode-3.0 | — |
| [zeroize 1.9.2](https://static.crates.io/crates/zeroize/zeroize-1.9.2.crate) | 有成員 | Apache-2.0 OR MIT | `LICENSE-APACHE`, `LICENSE-MIT` |
| [zeroize_derive 1.5.0](https://static.crates.io/crates/zeroize_derive/zeroize_derive-1.5.0.crate) | 只在主機建置圖 | Apache-2.0 OR MIT | — |
| [zip 9.8.2](https://static.crates.io/crates/zip/zip-9.8.2.crate) | 有成員 | MIT | `LICENSE` |
| [zlib-rs 0.6.8](https://static.crates.io/crates/zlib-rs/zlib-rs-0.6.8.crate) | 有成員 | Zlib | `LICENSE` |
| [zmij 1.0.23](https://static.crates.io/crates/zmij/zmij-1.0.23.crate) | 有成員 | MIT | `LICENSE-MIT` |
| [zune-core 0.5.3](https://static.crates.io/crates/zune-core/zune-core-0.5.3.crate) | 有成員 | MIT OR Apache-2.0 OR Zlib | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE-ZLIB` |
| [zune-jpeg 0.5.15](https://static.crates.io/crates/zune-jpeg/zune-jpeg-0.5.15.crate) | 有成員 | MIT OR Apache-2.0 OR Zlib | `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE-ZLIB` |

## 附錄二　逐一核對的 GNU／Rust 輸入

每一筆 archive 成員都在 SHA-256 相符的原 archive 重新查到。表格的每行是相同來源檔的分組，逐成員名稱、原檔 SHA-256 在固定 ZIP 的 `evidence/gnu-runtime-members.json`。含 `.idata$` 的部件分為匯入相關成員；靜態相容程式另外計數。GNU 的 6 個 Debian 安裝封存逐一比對 `evidence/debian-toolchain.txt`，Rust 工具鏈檔案與原件逐一比對。

| 原始輸入 | 來源 | 成員數 | 方式 | 原件 SHA-256 |
| --- | --- | --- | --- | --- |
| `crtbegin.o` | Debian GCC Runtime | 1 | 1 靜態物件 | `8daf9758739b291e05ed8602668afab3ba78df7fe6dd43be066d5e8c87d196cd` |
| `crtend.o` | Debian GCC Runtime | 1 | 1 靜態物件 | `3e98eb786c79afb4f4149c96f7a7c68aa5f88141d40a62f4b906979b2767ca8d` |
| `libgcc_eh.a` | Debian GCC Runtime | 1 | 1 靜態成員 | `3a4a7443c9d34d1ec2e19bc7a41d86a3db6ab3b81ebb4e46856fd4dc8bc7212c` |
| `crt2.o` | Debian MinGW-w64 | 1 | 1 靜態物件 | `aec37203a7a5cb2d5cddd32ebec708580b6ef6fb342dc20cde4bd8cb93b372c4` |
| `libkernel32.a` | Debian MinGW-w64 | 30 | 30 含匯入段落 | `7e374e069e6a78de2bbc8cf56bbf2fe804d87589f14a566525f71faf914a3dbd` |
| `libmingw32.a` | Debian MinGW-w64 | 18 | 18 靜態成員 | `4dde9b41d962ef3e0bd2543fa67b535eba0dddcb20081b9e083f0e3928292319` |
| `libmingwex.a` | Debian MinGW-w64 | 10 | 10 靜態成員 | `f74cc313748a8cbe14966959f50d91c1d6c4f2ae695f9f8f50a27db51665254d` |
| `libmsvcrt.a` | Debian MinGW-w64 | 85 | 36 靜態成員，49 含匯入段落 | `da905cfd203c9b0217feacfcb8fce727c61e2375591c37d7b1ff7c3d392841bf` |
| `libntdll.a` | Debian MinGW-w64 | 3 | 3 含匯入段落 | `d5af01025b4459e45360a651502b9fcda04b4f49084b273404616a5c81591fae` |
| `libaddr2line-d27dc4682ffee726.rlib` | Rust sysroot | 1 | 1 靜態成員 | `c0a4c3df47dd96ff57d7d12fa1f4758cedf880581c5e12807e6c9253322f8deb` |
| `libadler2-23df02d5efded178.rlib` | Rust sysroot | 1 | 1 靜態成員 | `090a279c40b865a099716b3a6da7fe274b6a3cb06e16c02ca747014e7cb4e541` |
| `liballoc-5d7cfa4a50b78852.rlib` | Rust sysroot | 1 | 1 靜態成員 | `206ccf5ea9fa87f0e88761be190bbb60d1840ebe9c44e1ab48b4ea0164c8c440` |
| `libcompiler_builtins-361d9253e271bd9c.rlib` | Rust sysroot | 4 | 4 靜態成員 | `8f5d51a005f7b9f2e6fa3b679a90014be9bec5352f15630c7bd3d2176415f77d` |
| `libcore-912afa5aef0a8d58.rlib` | Rust sysroot | 1 | 1 靜態成員 | `47ad6b09ebe282ea0e77d7f7575a6c0513003fef34df1097b06323ded5933845` |
| `libgimli-9f70e64c8106496b.rlib` | Rust sysroot | 1 | 1 靜態成員 | `b7c5e69cef21262724794f9af6382c740f268214c30fbe11bb23f98c30bd6b43` |
| `libhashbrown-e9bafb53c79123be.rlib` | Rust sysroot | 1 | 1 靜態成員 | `449d8e54c414aa0799bccaeba70cade253b8d59a29c390dda3459ee591d881ea` |
| `libmemchr-854def77705fb016.rlib` | Rust sysroot | 1 | 1 靜態成員 | `12956ec20ef3ec70b68f2ab1babd222e5c6319fba0c67169af369b607988b4a5` |
| `libminiz_oxide-6252ea0d0e4f6535.rlib` | Rust sysroot | 1 | 1 靜態成員 | `c226527420303f1c2d299ad19e7aab0871d8f602ad03825c47536ade35f8c1d0` |
| `libobject-dc2cc558bf711338.rlib` | Rust sysroot | 1 | 1 靜態成員 | `cdf184f567fe1c0ff5aaaa37e1f157f6512fe4b31fa08068f5f75e2c62571748` |
| `libpanic_unwind-4939ca1bd89407cf.rlib` | Rust sysroot | 1 | 1 靜態成員 | `351af1bbde5ca331e56162239e77dc2b9ed9054ce16631a108a82960f25c0cfd` |
| `librustc_demangle-4ce83fa1e4a815bc.rlib` | Rust sysroot | 1 | 1 靜態成員 | `26636ae3fde7dc7663dfeef07a663b9dc595a9d12311afb287854bed406cb6e8` |
| `libstd-68a98b1e3aff2557.rlib` | Rust sysroot | 1 | 1 靜態成員 | `4ce3d0e7ac39cbe4d4fb03d01cd74ff174fe16c1bce45db801a7b1a664ca36a5` |
| `libstd_detect-4d07676a7837d2d2.rlib` | Rust sysroot | 1 | 1 靜態成員 | `e87be3a0734a49bd6d24f9679a6823735be16148777ab2cf7eb15bfb46e80057` |

<details>
<summary>MinGW 65 個靜態輸入的來源檔</summary>

路徑相對於 MinGW 12.0.0 原檔的 `mingw-w64-crt/`。來源關聯依物件命名、來源檔與 x64 Makefile 規則核對；檔頭聲明及上游整體通知一起保留。

| 物件 | 來源檔 | 原檔通知 |
| --- | --- | --- |
| `crt2.o` | `crt/crtexe.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-CRT_fp10.o` | `crt/CRT_fp10.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-_newmode.o` | `crt/_newmode.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-cinitexe.o` | `crt/cinitexe.c` | 原檔無獨立授權，參照上游 runtime 通知 |
| `lib64_libmingw32_a-crt_handler.o` | `crt/crt_handler.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-dllargv.o` | `crt/dllargv.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-gccmain.o` | `crt/gccmain.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-merr.o` | `crt/merr.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-mingw_helpers.o` | `crt/mingw_helpers.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-natstart.o` | `crt/natstart.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-pesect.o` | `crt/pesect.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-pseudo-reloc.o` | `crt/pseudo-reloc.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-tlsmcrt.o` | `crt/tlsmcrt.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-tlssup.o` | `crt/tlssup.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-tlsthrd.o` | `crt/tlsthrd.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-usermatherr.o` | `crt/usermatherr.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-wildcard.o` | `crt/wildcard.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-xncommod.o` | `crt/xncommod.c` | 原檔公有領域聲明 |
| `lib64_libmingw32_a-xtxtmode.o` | `crt/xtxtmode.c` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-cosl_internal.o` | `math/x86/cosl_internal.S` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-cossin.o` | `math/x86/cossin.c` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-exp2l.o` | `math/x86/exp2l.S` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-hypotf.o` | `math/hypotf.c` | 原檔無獨立授權，參照上游 runtime 通知 |
| `lib64_libmingwex_a-internal_logl.o` | `math/x86/internal_logl.S` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-ldexp.o` | `math/x86/ldexp.c` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-log2l.o` | `math/x86/log2l.S` | 原檔公有領域聲明 |
| `lib64_libmingwex_a-mingw_matherr.o` | `misc/mingw_matherr.c` | 原檔無獨立授權，參照上游 runtime 通知 |
| `lib64_libmingwex_a-powi.o` | `math/powi.c` | ZPL-2.1 |
| `lib64_libmingwex_a-sinl_internal.o` | `math/x86/sinl_internal.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-acrt_iob_func.o` | `stdio/acrt_iob_func.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-atan2.o` | `math/x86/atan2.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-atan2f.o` | `math/x86/atan2f.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-cbrt.o` | `math/cbrt.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-cbrtf.o` | `math/cbrtf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-ceil.o` | `math/x86/ceil.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-ceilf.o` | `math/x86/ceilf.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-cos.o` | `math/x86/cos.c` | ZPL-2.1 |
| `lib64_libmsvcrt_common_a-cosf.o` | `math/x86/cosf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-exp.o` | `math/x86/exp.c` | ZPL-2.1 |
| `lib64_libmsvcrt_common_a-exp2f.o` | `math/x86/exp2f.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-expf.o` | `math/expf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-floor.o` | `math/x86/floor.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-floorf.o` | `math/x86/floorf.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-fma.o` | `math/fma.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-fmod.o` | `math/x86/fmod.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-fmodf.o` | `math/x86/fmodf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-frexp.o` | `math/frexp.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-hypot.o` | `math/hypot.c` | 原檔無獨立授權，參照上游 runtime 通知 |
| `lib64_libmsvcrt_common_a-log.o` | `math/x86/log.c` | ZPL-2.1 |
| `lib64_libmsvcrt_common_a-log10f.o` | `math/log10f.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-log2.o` | `math/x86/log2.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-log2f.o` | `math/x86/log2f.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-logf.o` | `math/logf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-pow.o` | `math/x86/pow.c` | ZPL-2.1 |
| `lib64_libmsvcrt_common_a-powf.o` | `math/powf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-rintf.o` | `math/x86/rintf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-round.o` | `math/round.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-roundf.o` | `math/roundf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-sin.o` | `math/x86/sin.c` | ZPL-2.1 |
| `lib64_libmsvcrt_common_a-sinf.o` | `math/x86/sinf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-tanf.o` | `math/x86/tanf.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_common_a-truncf.o` | `math/x86/truncf.S` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_extra_a-__p__commode.o` | `misc/__p__commode.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_extra_a-__p__fmode.o` | `misc/__p__fmode.c` | 原檔公有領域聲明 |
| `lib64_libmsvcrt_extra_a-invalid_parameter_handler.o` | `misc/invalid_parameter_handler.c` | 原檔無獨立授權，參照上游 runtime 通知 |

</details>

PE 的 14 個直接 DLL 名稱如下。

`advapi32.dll`、`api-ms-win-core-synch-l1-2-0.dll`、`bcryptprimitives.dll`、`comctl32.dll`、`comdlg32.dll`、`gdi32.dll`、`kernel32.dll`、`msvcrt.dll`、`ntdll.dll`、`ole32.dll`、`shell32.dll`、`user32.dll`、`userenv.dll`、`ws2_32.dll`。

## 附錄三　EXE 內核對的資料資產

下列資產均能在 EXE 內找到完整原位元組。同一份 ICC 在不同來源 crate 出現時列兩次。

| 鎖定來源 | 內嵌原件 | 位元組 | SHA-256 |
| --- | --- | --- | --- |
| hayro-font 0.5.0 | `FoxitDingbats.pfb` | 29513 | `845c752392b6c914fb989c75a08b7792b88f542d2499042ef2889f8c814a16ed` |
| hayro-font 0.5.0 | `FoxitFixed.pfb` | 17597 | `b6c8fe53f134b8b6d4578cd2d544df4cee9624c4efa8d51a560fb40ea296101b` |
| hayro-font 0.5.0 | `FoxitFixedBold.pfb` | 18055 | `f1b7159702973f54fc86254ea38bcf3712b2a736eb3a0995751e9f4bb45ad603` |
| hayro-font 0.5.0 | `FoxitFixedBoldItalic.pfb` | 19151 | `8a000945843bd31add06aee63bf9fd41b7578b4f3242a4b7cd46349ad9d24d4d` |
| hayro-font 0.5.0 | `FoxitFixedItalic.pfb` | 18746 | `5007faf8320fa1fcc08b8894b356ad976f60b992ba29ee5272578bbdebaf3876` |
| hayro-font 0.5.0 | `FoxitSans.pfb` | 15025 | `a02339e95d20db14cbdfc1c6baf359cd36a091d5434925c3a4040f60e1a1af9e` |
| hayro-font 0.5.0 | `FoxitSansBold.pfb` | 16344 | `7bdfa36de1d5757dcfaf8b8a4be95c6e41c40fa70dbed9d78bf979a7b9959054` |
| hayro-font 0.5.0 | `FoxitSansBoldItalic.pfb` | 16418 | `555ce45924338e2e4b32821910221ce85da691fd59a9723c4c971a7c5627fc8b` |
| hayro-font 0.5.0 | `FoxitSansItalic.pfb` | 16339 | `7751cbefeda351708dae8c4a88db043411bc39ab2ae2097685da845d4d16f307` |
| hayro-font 0.5.0 | `FoxitSerif.pfb` | 19469 | `4f57d2b9d884af8f907bf22df6019b52d86cbf6214fdbd02b1ae05472a543f35` |
| hayro-font 0.5.0 | `FoxitSerifBold.pfb` | 19395 | `0bdf4b04e964139818d51eda03d566ba999fc3ba2421b1c6c51f9dc969022e80` |
| hayro-font 0.5.0 | `FoxitSerifBoldItalic.pfb` | 20733 | `a406cac82583bf98175cb62c87ed5e95c45fbb34eece63a10b0a13e793cb2e10` |
| hayro-font 0.5.0 | `FoxitSerifItalic.pfb` | 21227 | `610ae0687198045c4db5d1a6650fd5e92536631706382eb8b72e38df578d0ae9` |
| hayro-font 0.5.0 | `FoxitSymbol.pfb` | 16729 | `47967d055530e7357088a08403115425643ec2cdfd6201ba8af0fbd7116c1539` |
| hayro-cmap 0.1.1 | `cmaps.brotli` | 250740 | `ddf37af521d904d5646c2fc37b2b7e912dc1b56f0d210190b61f598154758e46` |
| hayro-interpret 0.9.0 | `CGATS001Compat-v2-micro.icc` | 8464 | `73e1ba37d2bad5bab2a964f40a9eed96209666efc067c3322626214bbef234a0` |
| hayro-interpret 0.9.0 | `LAB.icc` | 484 | `a6efc5c30ece826eba2a83ddf5aa7026289b0c5ffa90acb129bc59ab8d7f5c59` |
| hayro-jpeg2000 0.5.0 | `CGATS001Compat-v2-micro.icc` | 8464 | `73e1ba37d2bad5bab2a964f40a9eed96209666efc067c3322626214bbef234a0` |
| hayro-jpeg2000 0.5.0 | `LAB.icc` | 484 | `a6efc5c30ece826eba2a83ddf5aa7026289b0c5ffa90acb129bc59ab8d7f5c59` |
| hayro-jpeg2000 0.5.0 | `ProPhoto-v2-micro.icc` | 496 | `00b0ac58f99189b071bfca63b5434fd9f48ad08c92b17e6ce525e54b9d1a8319` |
