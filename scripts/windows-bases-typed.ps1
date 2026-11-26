# Native QA for the new typed Bases independently of the prior status-only scenario.
$baseApp=Join-Path $root 'bases-typed-app'
$baseVault=Join-Path $root 'TypedVault'
$baseDb=Join-Path $baseApp 'paperroom-data/notes.sqlite3'
$baseBackup=Join-Path $root 'bases-typed-backup.sqlite3'
foreach ($folder in @((Split-Path $baseDb -Parent),$baseVault)) { [void][IO.Directory]::CreateDirectory($folder) }
Copy-Item -LiteralPath $Exe -Destination (Join-Path $baseApp 'Paperroom.exe')
$fixture=@'
from pathlib import Path
import sys
vault,root=map(Path,sys.argv[1:])
notes={"零值":"---\napproved: false\ncost: 0\n---\n零值正文","高值":"---\napproved: true\ncost: 12\n---\n高值正文",
       "空值":"---\napproved: null\ncost: null\n---\n空值正文","缺欄":"---\ncontext: unchanged\n---\n缺欄正文",
       "小值":"---\napproved: true\ncost: 2\n---\n小值正文"}
(vault/"甲").mkdir()
for name,body in notes.items(): (vault/"甲"/f"{name}.md").write_bytes(body.encode("utf-8"))
for name,approved,cost in (("TypeA","核准","價格"),("TypeB","審核","費用"),("Cut","比對","成本"),("Grouped","排序核准","群組工時")):
    flt="    filters: 'note.cost >= 2.5'\n" if name=="Cut" else ""
    if name=="Grouped":
        flt="    groupBy: {property: note.cost, direction: ASC}\n"
    body=f"""formulas:
  approvedCheck: 'note.approved == true'
  over: 'note.cost >= 2.5'
properties:
  note.approved: {{displayName: {approved}}}
  note.cost: {{displayName: {cost}}}
  formula.approvedCheck: {{displayName: 已核准}}
  formula.over: {{displayName: 超過門檻}}
views:
  - type: table
    name: 型別表
{flt}    order: [note.approved, note.cost, formula.approvedCheck, formula.over]
"""
    (root/f"{name}.base").write_bytes(body.encode("utf-8"))
'@ | python - $baseVault $root
if ($LASTEXITCODE -ne 0) { throw 'Could not create the actual synthetic Vault source.' }
$inputHashes=@{}
foreach ($path in @((Get-ChildItem -LiteralPath $baseVault -File -Recurse).FullName) + @((Get-ChildItem -LiteralPath $root -Filter '*.base' -File).FullName)) {
    $inputHashes[$path]=(Get-FileHash -LiteralPath $path).Hash
}
function Import-TypedBase([string]$Name) {
    Open-LinkedFixture '零值' '甲'
    Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
    $file=Join-Path $root "$Name.base"
    $confirm=Native-File 40 '匯入 Obsidian Bases 檢視…' $file '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
    Finish-Revision $confirm 6
    Native-Menu 109 2 '連結工作區欄位（可編輯）' -MenuOpen
    $null=Wait-For "typed Base $Name imported" {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 1 個 Bases 檢視。'
    }
}
function Open-TypedBase([string]$Name) {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Choose-CollectionRow "$Name／型別表"
    $null=Wait-For "typed collection $Name active" {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' -and
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq "$Name／型別表"
    }
    Open-SortNamedView 1 '型別表'
    $null=Wait-For 'actual typed table active' { [PaperroomHighlightGui]::Read((Control $window 115)) -ceq '型別表' }
}
function Read-TypedBaseRow([string]$Title,[string[]]$Witness) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    Choose-CollectionRow "已匯入　$Title  ("
    $dialog=Wait-For 'native typed table values' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
    }
    $body=[PaperroomHighlightGui]::Read((Control $dialog 101))
    foreach ($part in $Witness) { if (!$body.Contains($part)) { throw "Expected exact typed evidence $part but got $body." } }
    Finish-Revision $dialog 1
    Write-Host "Confirmed actual typed table row $Title with $($Witness -join ', ')."
}
function Assert-LiveGroupOrder([string[]]$Ordered) {
    Open-TypedBase Grouped
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    $labels=@(Collection-MenuLabels)
    if ($labels.Count -ne $Ordered.Count) { throw "Typed grouping returned unexpected rows: $($labels -join ' | ')." }
    for ($index=0;$index -lt $Ordered.Count;$index++) {
        if (!$labels[$index].StartsWith($Ordered[$index],[StringComparison]::Ordinal)) {
            throw "Unexpected group order: $($labels -join ' | ')"
        }
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null=Wait-For 'group order picker closed' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0) -eq [IntPtr]::Zero
    }
    Write-Host "Verified native numeric group order $($Ordered -join ' | ')."
}
function Edit-TypedBase([string]$Alias,[string]$Title,[string]$Expected,[string]$Draft) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 6 '編輯連結的布林或數值…' -MenuOpen
    Choose-CollectionRow $Alias
    Choose-CollectionRow "$Title (#"
    Rule-Prompt '編輯工作區型別欄位' $Expected $Draft
}
function Assert-TypedBaseDb([string]$Db,[string]$Approved,[string]$Cost,[string]$Prior='') {
    $result=@'
from pathlib import Path
from contextlib import closing
import sqlite3,sys
db,approval,cost,prior=sys.argv[1:]
with closing(sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True)) as c:
    note,body,token=c.execute("select n.id,n.body,i.token from notes n join note_link_identities i on i.note_id=n.id where title='零值'").fetchone()
    assert body==f"---\napproved: {approval}\ncost: {cost}\n---\n零值正文",body
    assert dict(c.execute("select key,value from note_properties where note_id=?",(note,)))=={"價格":"保留原生值"}
    by_collection={}
    for name,title in c.execute("select c.name,n.title from collection_notes m join collections c on c.id=m.collection_id join notes n on n.id=m.note_id"):
        by_collection.setdefault(name,set()).add(title)
    every={"零值","高值","空值","缺欄","小值"}
    assert by_collection=={"Cut／型別表":{"高值"},"TypeA／型別表":every,"TypeB／型別表":every,"Grouped／型別表":every},by_collection
    assert c.execute("pragma user_version").fetchone()==(41,)
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    identity=f"{note}:{token}"
    assert not prior or identity==prior
    print(identity)
'@ | python - $Db $Approved $Cost $Prior
    if ($LASTEXITCODE -ne 0 -or !$result) { throw 'Typed Bases source, import-time filtered members or stable identity differed.' }
    return [string]$result
}
$process=Start-Notebook $baseApp
$window=$process.MainWindowHandle
Native-VaultFolder $baseVault '匯入 5 篇筆記'
Open-LinkedFixture '零值' '甲'
Native-Menu 113 4 '設定文字屬性…'
Collection-Prompt '文字屬性' '價格'
Collection-Prompt '設定文字屬性' '保留原生值'
foreach ($name in 'TypeA','TypeB','Cut','Grouped') { Import-TypedBase $name }
$sourceId=Assert-TypedBaseDb $baseDb 'false' '0'
Assert-LiveGroupOrder @('0　零值  (','2　小值  (','12　高值  (','（缺少）　缺欄  (','null　空值  (')
Open-TypedBase TypeA
Read-TypedBaseRow '零值' @('核准　false','價格　0','已核准　false','超過門檻　false')
Read-TypedBaseRow '空值' @('核准　null','價格　null','已核准　null','超過門檻　null')
Read-TypedBaseRow '缺欄' @('核准　（缺少）','價格　（缺少）','已核准　（缺少）','超過門檻　（缺少）')
Edit-TypedBase '價格' '零值' '0' 'not-a-number'
Rule-Error '欄位未儲存（可複製）' 'not-a-number'
$null=Assert-TypedBaseDb $baseDb 'false' '0' $sourceId
Edit-TypedBase '核准' '零值' 'false' 'true'
Open-TypedBase TypeB
Read-TypedBaseRow '零值' @('審核　true','費用　0','已核准　true','超過門檻　false')
Edit-TypedBase '費用' '零值' '0' '3.5'
Open-TypedBase TypeA
Read-TypedBaseRow '零值' @('核准　true','價格　3.5','已核准　true','超過門檻　true')
Open-TypedBase Cut
Read-TypedBaseRow '高值' @('成本　12','超過門檻　true')
$null=Assert-TypedBaseDb $baseDb 'true' '3.5' $sourceId
Assert-LiveGroupOrder @('2　小值  (','3.5　零值  (','12　高值  (','（缺少）　缺欄  (','null　空值  (')
Native-Menu 120 25 '群組收合…'
Choose-CollectionRow '收合 「2」'
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Rule-Prompt '設定工作區屬性型別' '' '群組工時'
Native-Menu 109 1 '數字' -MenuOpen
$null=Wait-For 'unrelated workspace type saved' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新工作區屬性型別，原有筆記值已保留。'
}
Assert-LiveGroupOrder @('3.5　零值  (','12　高值  (','（缺少）　缺欄  (','null　空值  (')
Native-File 4 '備份工作區…' $baseBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restore=Join-Path $root 'typed-bases-restore'
$restoreDb=Join-Path $restore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoreDb -Parent))
Copy-Item -LiteralPath $baseBackup -Destination $restoreDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restore 'Paperroom.exe')
foreach ($pair in @(@($baseApp,$baseDb),@($restore,$restoreDb))) {
    $process=Start-Notebook $pair[0]
    $window=$process.MainWindowHandle
    Open-TypedBase TypeB
    Read-TypedBaseRow '零值' @('審核　true','費用　3.5','已核准　true','超過門檻　true')
    $null=Assert-TypedBaseDb $pair[1] 'true' '3.5' $sourceId
    Assert-LiveGroupOrder @('3.5　零值  (','12　高值  (','（缺少）　缺欄  (','null　空值  (')
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
foreach ($path in $inputHashes.Keys) {
    if ((Get-FileHash -LiteralPath $path).Hash -cne $inputHashes[$path]) { throw 'Source Vault bytes changed.' }
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('false-zero-null-missing-visible','typed-comparisons','import-time-typed-filter','live-numeric-group-order','fold-isolated-from-native-same-alias','rejected-type-no-write','boolean-and-number-edits','same-canonical-note-under-aliases','existing-native-same-name-field-preserved','reopen','isolated-restore')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: exact EXE verifies typed Bases cells, comparisons, two aliases and isolated restore.'
