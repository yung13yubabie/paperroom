# Run under windows-highlight-workflow.ps1 with a fresh root and the exact downloaded EXE.
$baseApp=Join-Path $root 'descending-live'
$baseVault=Join-Path $root 'DescendingVault'
$baseDb=Join-Path $baseApp 'paperroom-data/notes.sqlite3'
$baseBackup=Join-Path $root 'descending-backup.sqlite3'
$baseFile=Join-Path $root 'Grouped.base'
foreach ($folder in @((Split-Path $baseDb -Parent),$baseVault)) { [void][IO.Directory]::CreateDirectory($folder) }
Copy-Item -LiteralPath $Exe -Destination (Join-Path $baseApp 'Paperroom.exe')
@'
from pathlib import Path
import sys
v,b=map(Path,sys.argv[1:])
d=v/'甲'
d.mkdir()
for title,cost in [('高值','10'),('中值','2'),('低值','-1'),('缺欄',None),('空值','null'),('壞值','"oops"')]:
    data='---\n'+('other: keep' if cost is None else 'cost: '+cost)+'\n---\n'+title+'正文'
    if title=='中值': data+='\n[附件](same.txt)'
    (d/(title+'.md')).write_bytes(data.encode())
(d/'same.txt').write_bytes(b'Paperroom-local-asset-28\r\n')
b.write_bytes("""formulas:
  over: 'note.cost > 8'
properties:
  note.cost: {displayName: 群組工時}
  formula.over: {displayName: 高成本}
views:
  - type: table
    name: 型別表
    groupBy: {property: note.cost, direction: DESC}
    order: [note.cost, formula.over]
  - type: table
    name: 門檻
    groupBy: {property: formula.over, direction: DESC}
    order: [note.cost, formula.over]
""".encode())
'@ | python - $baseVault $baseFile
if ($LASTEXITCODE -ne 0) { throw 'Could not create the synthetic DESC Vault.' }
$inputHashes=@{}
foreach($p in @((Get-ChildItem -LiteralPath $baseVault -Recurse -File).FullName)+@($baseFile)) { $inputHashes[$p]=(Get-FileHash -LiteralPath $p).Hash }
function Import-TypedBase([string]$Name) {
    Open-LinkedFixture '中值' '甲'
    Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
    $file=Join-Path $root "$Name.base"
    $confirm=Native-File 40 '匯入 Obsidian Bases 檢視…' $file '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
    Finish-Revision $confirm 6
    Native-Menu 109 2 '連結工作區欄位（可編輯）' -MenuOpen
    $null=Wait-For "typed Base $Name imported" {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 2 個 Bases 檢視。'
    }
}
function Open-TypedBase([string]$Name,[string]$View='型別表') {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Choose-CollectionRow "$Name／$View"
    $null=Wait-For "typed collection $Name active" {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' -and
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq "$Name／$View"
    }
    Open-SortNamedView 1 $View
    $null=Wait-For 'actual typed table active' { [PaperroomHighlightGui]::Read((Control $window 115)) -ceq $View }
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
function Assert-FormulaGroupOrder([bool]$Edited) {
    Open-TypedBase Grouped '門檻'
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    $labels=@(Collection-MenuLabels)
    $wanted=if ($Edited) { @('true','true','false') } else { @('true','false','false') }
    $wanted+=@('（缺少）','null','（型別或格式不符）')
    $heads=@($labels | ForEach-Object { ($_ -split '　',2)[0] })
    $correct=($heads -join '|') -ceq ($wanted -join '|')
    $group=if ($Edited) {'true'} else {'false'}
    if (!$correct -or !@($labels | Where-Object { $_.StartsWith("$group　中值  (",[StringComparison]::Ordinal) }).Count) {
        throw "Computed grouping has an unexpected order or stale target membership: $($labels -join ' | ')"
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null=Wait-For 'computed group picker closed' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0) -eq [IntPtr]::Zero
    }
    Write-Host "Verified native formula groups: $($labels -join ' | ')."
}
function Edit-TypedBase([string]$Alias,[string]$Title,[string]$Expected,[string]$Draft) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 6 '編輯連結的布林或數值…' -MenuOpen
    Choose-CollectionRow $Alias
    Choose-CollectionRow "$Title (#"
    Rule-Prompt '編輯工作區型別欄位' $Expected $Draft
}
function Witness-GroupDb([string]$Db,[string]$Expected,[string]$Prior='') {
    $rows=@'
from pathlib import Path
from contextlib import closing
import sqlite3,sys
db,cost,prior=sys.argv[1:]
with closing(sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True)) as c:
    c.execute("BEGIN")
    rows=c.execute("""SELECT n.id,i.token,n.body,a.id,a.content FROM vault_paths p
    JOIN notes n ON n.id=p.note_id JOIN note_link_identities i ON i.note_id=n.id
    JOIN attachments a ON a.note_id=n.id WHERE p.relative_path=? AND a.name=?""",('甲/中值.md','same.txt')).fetchall()
    assert len(rows)==1,rows
    note,token,body,asset,data=rows[0]
    assert data==b'Paperroom-local-asset-28\r\n' and body==f'---\ncost: {cost}\n---\n中值正文\n[附件](same.txt)',(data,body)
    assert c.execute('SELECT count(*) FROM obsidian_base_descending_groups').fetchone()==(2,)
    assert c.execute('PRAGMA integrity_check').fetchall()==[('ok',)]
    proof=f"{note}:{token}:{asset}"
    assert not prior or prior==proof
    print(proof)
'@ | python - $Db $Expected $Prior
    if ($LASTEXITCODE -ne 0 -or !$rows) {throw 'Stored group direction, source identities or attachment bytes differ'}
    return [string]$rows
}
$process=Start-Notebook $baseApp
$window=$process.MainWindowHandle
Native-VaultFolder $baseVault '附件綁定 1 筆，未解析 0 筆'
Import-TypedBase Grouped
$tail=@('（缺少）　缺欄  (','null　空值  (','（型別或格式不符）　壞值  (')
Assert-LiveGroupOrder (@('10　高值  (','2　中值  (','-1　低值  (')+$tail)
Assert-FormulaGroupOrder $false
Open-TypedBase Grouped
$id=Witness-GroupDb $baseDb '2'
Edit-TypedBase '群組工時' '中值' '2' '14'
$after=@('14　中值  (','10　高值  (','-1　低值  (')+$tail
Assert-LiveGroupOrder $after
Assert-FormulaGroupOrder $true
$null=Witness-GroupDb $baseDb '14' $id
Native-File 4 '備份工作區…' $baseBackup '工作區備份已完成'
$backupHash=(Get-FileHash -LiteralPath $baseBackup).Hash
Close-Notebook $process
$process.Dispose()
$process=$null
$restore=Join-Path $root 'isolated-descending'
$restoreDb=Join-Path $restore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoreDb -Parent))
Copy-Item -LiteralPath $baseBackup -Destination $restoreDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restore 'Paperroom.exe')
foreach ($pair in @(@($baseApp,$baseDb),@($restore,$restoreDb))) {
    $process=Start-Notebook $pair[0]
    $window=$process.MainWindowHandle
    Assert-LiveGroupOrder $after
    Assert-FormulaGroupOrder $true
    $null=Witness-GroupDb $pair[1] '14' $id
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
foreach($p in $inputHashes.Keys) { if ((Get-FileHash -LiteralPath $p).Hash -cne $inputHashes[$p]) {throw "Source changed: $p"} }
if ((Get-FileHash -LiteralPath $baseBackup).Hash -cne $backupHash) {throw 'Backup bytes changed'}
@{exe_sha256=$ExeSha256; os=(Get-CimInstance Win32_OperatingSystem).Caption; base_sha256=$inputHashes[$baseFile]; backup_sha256=$backupHash; expected_native_group_rows=$after; formula_group_threshold='note.cost > 8: false to true'; source='synthetic'; identity=$id} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: DESC groups, missing/null/invalid lanes, edited membership order, source and attachment preservation, reopen and isolated restore.'
