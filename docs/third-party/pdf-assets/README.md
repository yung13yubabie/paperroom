# 第三方補充通知：PDF 資產

「檔案 → 關於紙間／第三方補充通知」顯示下列已核對的原始文字，不要求連網，也不讀寫筆記資料。這不是完整再發佈清單。其他 Rust 依賴、實際 MSVC 連結內容、工具鏈與最終發行包仍另行核對。

| 本目錄文字 | 原始來源（以鎖定套件／公布來源的原始位元為準） | SHA-256 |
| --- | --- | --- |
| `pdfium-foxit.txt` | `hayro-font 0.5.0: assets/LICENSE_FOXIT`；`assets/README.md` 另提供 Foxit 原始碼署名，已列在視窗內 | `b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7` |
| `adobe-cmap.txt` | `hayro-cmap 0.1.1: assets/LICENSE.txt`，Adobe CMaps | `742665db9c8e1bc72603c6d319ca3e90b83bd6d95202f0cd4ef11068a07a9c29` |
| `icc-cc0.txt` | `hayro-interpret 0.9.0: assets/CGATS_LICENSE.txt` 與 `hayro-jpeg2000 0.5.0: assets/LICENSE.txt` 內容相同；各自的 `assets/README.md` 說明 CGATS001、LAB 及 ProPhoto（僅 jpeg2000）的範圍 | `469d87b5fa48f8aaf69e25f4fcac3a309c97dbb7d561b917638abdb640e004d9` |
| `alloc-stdlib.txt` | `alloc-stdlib 0.2.4` 套件檔無根目錄授權文字；該套件 `.cargo_vcs_info.json` 指向 [上游發布當時的 LICENSE](https://github.com/dropbox/rust-alloc-no-stdlib/blob/ae42d22078b98549e987d2f03d12df7b984fde47/LICENSE)（Git blob `cd496ba72eb37e83a44958358d1f89a8a28cbc15`） | `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae` |

四個 Hayro 套件的來源為同一 [Git 修訂](https://github.com/LaurenzV/hayro/tree/3f3825c3e50bb41ed6ec6c3f4b6007978bd13def)。更新相關依賴時，先核對新的套件來源、所含資產與授權，再調整此頁和離線視窗。此處不決定 Paperroom 自身的授權。
