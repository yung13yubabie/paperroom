# Native source note + link rewrite, cancellation, neighboring same-name note and backup.
$vault=Join-Path $root 'SyntheticHeadingVault'
$homeA=Join-Path $root 'heading-app'
$homeB=Join-Path $root 'heading-restored'
$backup=Join-Path $root 'heading-backup.sqlite'
$utf8=[Text.UTF8Encoding]::new($false)
$old='舊小節'
$new='新小節'
$inputs=[ordered]@{
    'A/鏡頭.md'="## 舊小節`n目標正文`n[本篇](#舊小節)"
    'B/入口.md'="[[A/鏡頭#舊小節|原別名]]`n[路徑](../A/鏡頭.md#舊小節)`n"+'`[[A/鏡頭#舊小節|原別名]]`'
    'B/鏡頭.md'="## 舊小節`n另一篇"
}
$otherSections="`n## 舊小節相似`n相似段落`n## 重名`n歧義甲`n## 重名`n歧義乙"
$inputs['A/鏡頭.md']+=$otherSections
. (Join-Path $PSScriptRoot 'windows-vault-heading-canvas.ps1')
$diskHashes=@{}
foreach ($pair in $inputs.GetEnumerator()) {
    $file=Join-Path $vault $pair.Key
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $file))
    [IO.File]::WriteAllText($file,$pair.Value,$utf8)
    $diskHashes[$pair.Key]=(Get-FileHash -LiteralPath $file).Hash
}
foreach ($folder in $homeA,$homeB) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
function Read-HeadingState([string]$Db) {
    $raw=@'
import json,sqlite3,sys
from pathlib import Path
from contextlib import closing
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    records=db.execute("""select p.relative_path,n.id,n.version,n.body,i.token from vault_paths p join notes n on n.id=p.note_id
      join note_link_identities i on i.note_id=n.id join notes root on root.id=p.import_root
      where root.title=? and n.trash_root is null order by p.relative_path""",("匯入：SyntheticHeadingVault",)).fetchall()
    assert len(records)==3
    print(json.dumps({k:dict(id=i,version=v,body=b,token=t) for k,i,v,b,t in records},sort_keys=True))
'@ | python -B - $Db
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Could not witness the three exact imported note identities'}
    return [string]$raw
}
function Pick-HeadingSource {
    Native-Menu 109 24 '快速切換筆記…'
    Collection-Prompt '快速切換筆記' 'A/鏡頭.md'
    $labels=@(Collection-MenuLabels)
    $matches=@(for ($n=0;$n -lt $labels.Count;$n++) {
        if ($labels[$n].EndsWith("[#$targetId]",[StringComparison]::Ordinal) -and $labels[$n].Contains(' / A/鏡頭.md')) {$n+1}
    })
    if ($matches.Count -ne 1) {throw "Exact synthetic heading source ambiguous: $labels"}
    Native-Menu 109 $matches[0] $labels[$matches[0]-1] -MenuOpen
}
function Rename-Heading([int]$Decision) {
    Pick-HeadingSource
    Native-Menu 120 60005 '重新命名 Vault 小節…'
    $headings=@(Collection-MenuLabels)
    $index=[Array]::IndexOf($headings,$old)
    if ($index -lt 0) {throw "Exact old heading absent: $headings"}
    Native-Menu 120 ($index+1) $old -MenuOpen
    Rule-Prompt '重新命名 Vault 小節' $old $new
    $preview=Wait-For 'native heading rewrite preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認重新命名小節',1)
    }
    $report=[PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$report.Contains("「$old」 → 「$new」") -or !$report.Contains('2 篇筆記、3 個連結')) {
        throw "Unexpected heading rename preview: $report"
    }
    Finish-Revision $preview $Decision
}
$process=Start-Notebook $homeA
$window=$process.MainWindowHandle
$db=Join-Path $homeA 'paperroom-data/notes.sqlite3'
Native-VaultFolder $vault '匯入 3 篇筆記'
$before=Read-HeadingState $db
$initial=$before | ConvertFrom-Json -AsHashtable
$targetId=$initial['A/鏡頭.md'].id
Initialize-HeadingCanvas
$canvasIdentity=Read-CanvasHeadingWitness $db $old
Test-HeadingCardBoundaries
Test-HeadingCard $old
Rename-Heading 2
if ((Read-HeadingState $db) -cne $before) {throw 'Declining the preview changed a source note'}
if ((Read-CanvasHeadingWitness $db $old) -cne $canvasIdentity) {throw "Cancelled rename changed Canvas binding"}
Test-HeadingCard $old -Selected
Rename-Heading 1
$null=Wait-For 'heading rename committed' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已重新命名 Vault 小節並更新引用。'}
$after=Read-HeadingState $db
if ((Read-CanvasHeadingWitness $db $new) -cne $canvasIdentity) {throw "Applied rename changed Canvas binding"}
$state=$after | ConvertFrom-Json -AsHashtable
$fragment=[Uri]::EscapeDataString($new)
$wanted=@{
    'A/鏡頭.md'="## 新小節`n目標正文`n[本篇](#$fragment)"
    'B/入口.md'="[[A/鏡頭#新小節|原別名]]`n[路徑](../A/鏡頭.md#$fragment)`n"+'`[[A/鏡頭#舊小節|原別名]]`'
    'B/鏡頭.md'=$inputs['B/鏡頭.md']
}
$wanted['A/鏡頭.md']+=$otherSections
Test-HeadingCard $new
Test-HeadingCard $new -Selected
Test-HeadingCardBoundaries
foreach ($key in $inputs.Keys) {
    if ($state[$key].body -cne $wanted[$key] -or $state[$key].id -ne $initial[$key].id -or $state[$key].token -cne $initial[$key].token) {
        throw "Unexpected heading rewrite or identity at $key"
    }
    if ((Get-FileHash -LiteralPath (Join-Path $vault $key)).Hash -cne $diskHashes[$key]) {throw "External source changed: $key"}
}
Native-File 4 '備份工作區…' $backup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash $backup).Hash
Copy-Item $backup (Join-Path $homeB 'paperroom-data/notes.sqlite3')
foreach ($folder in $homeA,$homeB) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    Pick-HeadingSource
    $null=Wait-For 'renamed original note appears after reopening' {
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains($new)
    }
    Test-HeadingCard $new -Selected
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    if ((Read-CanvasHeadingWitness (Join-Path $folder 'paperroom-data/notes.sqlite3') $new) -cne $canvasIdentity) {throw 'Reopened Canvas binding differs'}
    if ((Read-HeadingState (Join-Path $folder 'paperroom-data/notes.sqlite3')) -cne $after) {throw 'Reopened workspace differs'}
}
foreach ($key in $inputs.Keys) {
    if ((Get-FileHash -LiteralPath (Join-Path $vault $key)).Hash -cne $diskHashes[$key]) {throw "External source changed after reopen: $key"}
}
if ((Get-FileHash -LiteralPath $canvasPath).Hash -cne $sourceCanvasHash) {throw 'The external Canvas source bytes changed'}
if ((Get-FileHash $backup).Hash -cne $backupHash) {throw 'Backup bytes changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupHash;canvas_input_sha256=$sourceCanvasHash;scenarios=@('native selected and listed Canvas heading cards before and after rename','same-title, similar and ambiguous destinations','declined preview','same-title note isolation','Markdown and aliased wiki anchors','inline code preserved','external sources unchanged','reopened original and isolated restore')} |
    ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
