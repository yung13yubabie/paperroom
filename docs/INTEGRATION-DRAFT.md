# 紙間直接整合草稿

固定的公開基準為 `ff801a87df513d77c7c4e9af7c29824618d274e6`。產品依 [#29 固定來源](https://github.com/yung13yubabie/paperroom/tree/3dcd51c33ef172a7dec9cb9aef8b98f07176303b) 整合，測試腳本沿用 [#29 驗收提交](https://github.com/yung13yubabie/paperroom/tree/ae2be426071e55946a385f06ae1fe7b33f455c41)。此草稿以公開基準作為父提交，既有草稿與原生驗收紀錄可分別查閱。

## 基準樹的差異

- 以 #29 驗收樹比較，保留原有 Rust、Cargo、操作說明、封裝與可重用 GUI 測試腳本。去重後相對公開基準有 112 個繼承的變更路徑。
- 將 `.github/workflows/draft-windows-acceptance.yml` 還原為公開基準版本，移除 #29 堆疊才有的十支 `windows-gnu-*-draft.yml`。這些流程依賴舊分支、工件或測試來源；歷史結果仍可從原 PR 與 Actions 查看。
- 公開基準原有的工作流程保留原 blob。此分支的例行驗收使用同一次建置的工件與 SHA-256。發行申請、標籤、簽章與上傳 Release 資產屬另一操作。
- 整合來源保留公開且為合成的測試夾具及可重跑程式。掃描範圍內無新增的私用工作區、原廠私有快照、執行檔或 Release 封裝。

## 重做比對

```bash
base=ff801a87df513d77c7c4e9af7c29824618d274e6
product=3dcd51c33ef172a7dec9cb9aef8b98f07176303b
qa=ae2be426071e55946a385f06ae1fe7b33f455c41
git diff --name-status "$base" "$qa" -- .github/workflows
git diff --name-status "$product" "$qa" -- src Cargo.toml Cargo.lock scripts
git diff --stat "$base" "$qa" -- . ':!.github/workflows'
git diff --name-status "$qa" HEAD
```

依同一候選檢查正常使用、關閉重開和隔離還原，並另記可再散布依據、真實 Windows 10／11 客戶端斷網結果、原廠測試用匯出與合法取樣狀態。[完整堆疊](DRAFT-STACK-REVIEW.md) · [Windows 斷網驗收](../qa/windows-client-offline/GNU-CANDIDATE.md)
