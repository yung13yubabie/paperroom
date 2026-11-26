# 紙間草稿堆疊與發行驗收

## 本輪的基準

[公開 main](https://github.com/yung13yubabie/paperroom/tree/ff801a87df513d77c7c4e9af7c29824618d274e6) 至 [#28 驗收分支](https://github.com/yung13yubabie/paperroom/tree/38d4bf0b9ef26370da101e00a731b169d095b330) 共有 37 個獨立提交，沒有分歧或合併提交。逐一核對 GitHub 的 base 與 head 後，#18 至 #28 各段新增提交數為 1、8、2、3、2、2、2、4、3、7、3。取兩棵 tree 直接比較，共 118 個變更路徑，增加 13,330 行、刪除 342 行。這些數字以固定提交為準。

- 14 個提交涉及產品或執行路徑，23 個涉及驗收或測試維護。58 個 Rust 檔、2 個 Cargo 檔、36 個 scripts、10 個工作流程、9 個 docs、2 個離線驗收檔與 README 有變更。
- 產品差異包括筆記身分、跨視圖關聯與彙總、Vault 型別欄位和 Canvas 綁定、附件與子白板，以及路徑、拖曳和 SQLite WAL 的資料保護。新增的直接依賴為 `yaml-rust2 0.13.2`，鎖檔另增 `arraydeque 0.5.1`。
- 已核對本段新增檔及歷史路徑。沒有新增公開執行檔、發行 ZIP、筆記資料庫、Git bundle 或原廠／私人工作區。兩份主線已有的 Notion 合成測資 ZIP 保持原 blob。正式公開前仍需逐份檢查將交付的原始碼、打包與中繼資料。
- #28 打包來源為 `6f0f8c99c566503f0b743ebd0add6b770cd1cd3e`。其後 `6a4835c` 與 `38d4bf0` 只改該輪工作流程和 GUI 驗收腳本。兩次不同驗收紀錄對應同一原件。[打包及驗收](https://github.com/yung13yubabie/paperroom/actions/runs/45776087463) · [原生 GUI 重跑](https://github.com/yung13yubabie/paperroom/actions/runs/45776787005)

## 正式候選的驗收範圍

- 保留可重跑的 Rust、封裝、遷移、GUI、備份還原與依賴檢查。九支歷史用 `windows-gnu-*-draft.yml` 綁定舊草稿分支及提交。正式發行 PR 應先逐支審閱；可保留測試本體、先前收據與工件連結，讓正式驗收走對應新候選的單一建置流程。
- 每個候選分別核對固定來源、同位元重建、ZIP／獨立 EXE 及雜湊、Windows Server 原生 GUI、v0.1.1 升級與完整工作區備份。凡產品、封裝或通知變動都要重新對應候選。
- 公開發行前另驗乾淨 Windows 10／11 x64 標準帳戶斷網、免安裝、備份還原及 Windows 安全提示。逐依賴的最終封裝證據可核對收錄的原始通知及連結來源；再散布條款還需發行者確認。[離線驗收手冊](../qa/windows-client-offline/GNU-CANDIDATE.md)
- Notion 原廠匯出 ZIP 及 Heptabase 全量備份只有在獲准隔離來源可得時才能驗收。這兩項保真度屬產品相容性證據，與 Windows 客戶端離線結果分開記錄。

可重跑的比對：

```bash
main=ff801a87df513d77c7c4e9af7c29824618d274e6
baseline=38d4bf0b9ef26370da101e00a731b169d095b330
git merge-base --is-ancestor "$main" "$baseline"
git rev-list --left-right --count "$main...$baseline"
git rev-list --count --merges "$main..$baseline"
git log --reverse --format='%h %p %s' "$main..$baseline"
git diff --stat "$main" "$baseline"
git diff --name-status "$main" "$baseline"
git diff --name-only 6f0f8c99c566503f0b743ebd0add6b770cd1cd3e "$baseline"
```
