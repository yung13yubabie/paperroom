# Dot-sourced by windows-highlight-workflow.ps1. GUI is the sole workspace writer.
$home27=Join-Path $root 'typed-branch'
$copy27=Join-Path $root 'typed-branch-restored'
$db27=Join-Path $home27 'paperroom-data/notes.sqlite3'
$backup27=Join-Path $root 'typed-branch-backup.sqlite3'
$manifest27=Join-Path $root 'typed-branch-manifest.json'
$prior27=Join-Path $root 'typed-branch-identities.json'
$checker27=if ($env:PAPERROOM_UNIFIED_CHECKER) {$env:PAPERROOM_UNIFIED_CHECKER} else {Join-Path $PSScriptRoot 'check_unified_workspace.py'}
if (!(Test-Path -LiteralPath $checker27)) {throw "Independent SQLite checker is missing: $checker27"}
$utf27=[Text.UTF8Encoding]::new($false)
$notes27=[ordered]@{
    canonical=@{title='共同筆記';marker='PAPERROOM-CANON'}
    other=@{title='對照筆記';marker='PAPERROOM-OTHER'}
    exact=@{title='場景';marker='TARGET-EXACT'}
    decoy=@{title='場景';marker='TARGET-DECOY'}
}
$ids27=@{}
$assets27=@{}
function NoteId27([string]$marker) {
    $result=@'
import sqlite3,sys
from pathlib import Path
with sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as c:
    ids=c.execute("select id from notes where instr(body,?)>0 and trash_root is null",(sys.argv[2],)).fetchall()
    assert len(ids)==1,ids
    print(ids[0][0])
'@ | python - $db27 $marker
    if ($LASTEXITCODE -ne 0 -or $result -notmatch '^\d+$') {throw "Missing unique imported note for $marker"}
    [int]$result
}
function Collection27([string]$name,[switch]$Create) {
    Native-Menu 109 10 '資料集合與看板…'
    if ($Create) {
        Native-Menu 109 2 '新增集合…' -MenuOpen
        Collection-Prompt '新增集合' $name
    } else {
        Native-Menu 109 1 '開啟集合看板…' -MenuOpen
        Choose-CollectionRow $name
    }
    $null=Wait-For "collection $name" { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
}
function BranchRelation27([string]$role,[bool]$reverse,[int]$action,[string]$target) {
    Native-Menu 120 27 '編輯資料列的關聯欄位…'
    Choose-CollectionRow "待處理　$($notes27[$role].title)　[#$($ids27[$role])]"
    $direction=if ($reverse) {'反向'} else {'正向'}
    Choose-CollectionRow "$direction　關聯　｜ "
    $label=@('','加入關聯筆記…','移除關聯筆記…','開啟關聯筆記…')[$action]
    Native-Menu 120 $action $label -MenuOpen
    Choose-CollectionRow "$($notes27[$target].title)　[#$($ids27[$target])]"
    if ($action -eq 2) {
        $confirm=Wait-For 'exact reverse edge removal confirmation' {
            [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','移除集合關聯',6)
        }
        foreach ($id in $ids27[$role],$ids27[$target]) {
            if (![PaperroomHighlightGui]::DialogContains($confirm,$process.Id,"[#$id]")) {throw 'Wrong removal endpoints'}
        }
        Finish-Revision $confirm 6
    }
    $status=@('','已加入集合關聯筆記。','已移除集合關聯筆記。','已開啟集合關聯筆記。')[$action]
    $null=Wait-For $status {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq $status}
}
function Witness27([string]$database,[switch]$Initial) {
    $paths=@($checker27,$database,$manifest27)
    if (!$Initial) {$paths+=$prior27}
    $json= & python @paths
    if ($LASTEXITCODE -ne 0 -or !$json) {throw "Identity witness failed for $database"}
    # Independent contract checks canonical identity; additionally verify both moved
    # placements and both sort terms, even when a coincidental title order matches.
    @'
import json,pathlib,sqlite3,sys
with sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as c:
    cards=c.execute("select b.title,p.note_id from board_cards p join boards b on b.id=p.board_id where b.title in ('Child','Research') order by p.note_id").fetchall()
    assert cards==[("Child",n) for n in sorted(map(int,sys.argv[2:]))], cards
    raw,=c.execute("select rules_json from collection_views where name='Ranked'").fetchone()
    sorts=json.loads(raw)[2]
    assert [(rule[0],rule[2]) for rule in sorts]==[("Batch",False),("Rank",False)], sorts
'@ | python - $database $ids27.canonical $ids27.other
    if ($LASTEXITCODE -ne 0) {throw 'Board membership or saved multi-sort differs'}
    if ($Initial) {[IO.File]::WriteAllText($prior27,($json -join "`n"),$utf27)}
}
foreach ($dir in $home27,$copy27) {
    [void][IO.Directory]::CreateDirectory((Join-Path $dir 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $dir 'Paperroom.exe')
}
$process=Start-Notebook $home27
$window=$process.MainWindowHandle
foreach ($role in $notes27.Keys) {
    $spec=$notes27[$role]
    $folder=Join-Path $root "typed-fixture-$role"
    [void][IO.Directory]::CreateDirectory($folder)
    $src=Join-Path $folder ($spec.title+'.md')
    [IO.File]::WriteAllText($src,"# $($spec.title)`r`n`r`n$($spec.marker)",$utf27)
    Native-File 1 '匯入 Markdown…' $src '已匯入 1 篇 Markdown'
    $ids27[$role]=NoteId27 $spec.marker
    if ($role -in 'canonical','other') {
        foreach ($key in 'Batch','Rank') {
            $value=if ($key -eq 'Batch') {'A'} elseif ($role -eq 'canonical') {'20'} else {'10'}
            Native-Menu 113 4 '設定文字屬性…'
            Collection-Prompt '文字屬性' $key
            Collection-Prompt '設定文字屬性' $value
        }
    }
    if ($role -in 'canonical','decoy') {
        $asset=Join-Path $folder 'same.txt'
        [IO.File]::WriteAllText($asset,"synthetic asset $role",$utf27)
        $assets27[$role]=@{name='same.txt';sha256=(Get-FileHash -LiteralPath $asset).Hash.ToLowerInvariant()}
        Native-Menu 113 8 '附件與 PDF 摘錄…'
        Native-Menu 113 1 '加入本機附件…' -MenuOpen
        Native-File 1 '加入本機附件…' $asset '附件已保存到本機工作區。' -AlreadyOpened
    }
}
foreach ($name in 'Source','Target') {
    Collection27 $name -Create
    $roles=if ($name -eq 'Source') {@('canonical','other')} else {@('exact','decoy')}
    foreach ($role in $roles) {
        Native-Menu 120 1 '加入筆記…'
        Choose-CollectionRow "$($notes27[$role].title)  ($($ids27[$role]))"
        Collection-Prompt '加入集合'
    }
}
Collection27 'Source'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 1 '儲存新檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'Ranked'
Native-Menu 120 2 '表格' -MenuOpen
Collection-Prompt '集合檢視'
Collection-Prompt '集合檢視'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
foreach ($key in 'Batch','Rank') {
    Native-Menu 120 2 '新增排序…' -MenuOpen
    Choose-CollectionRow '其他文字欄位…'
    Collection-Prompt '文字欄位' $key
    Native-Menu 120 1 '升冪' -MenuOpen
}
Native-Menu 120 5 '儲存變更' -MenuOpen
Assert-RankedRows @('對照筆記','共同筆記')
Select-Note '共同筆記'
Native-Menu 120 60001 '具名關聯的來源與目標…'
Native-Menu 120 3 '編輯集合關聯屬性…' -MenuOpen
Choose-CollectionRow '新增關聯屬性…'
Choose-CollectionRow 'Source　[#'
Choose-CollectionRow 'Target　[#'
Collection-Prompt '新增集合關聯屬性' '關聯'
Collection27 'Source'
BranchRelation27 canonical $false 1 exact
Collection27 'Target'
BranchRelation27 exact $true 2 canonical
# Observe the intermediate persisted removal, rather than merely the final re-add.
$edgeCount=@'
import pathlib,sqlite3,sys
with sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as c:
    print(c.execute("select count(*) from relation_field_values").fetchone()[0])
'@ | python - $db27
if ($LASTEXITCODE -ne 0 -or "$edgeCount" -cne '0') {throw 'Reverse removal left an edge'}
BranchRelation27 exact $true 1 canonical
Collection27 'Source'
Native-Menu 120 26 '資料列附件…'
Choose-CollectionRow "待處理　共同筆記　[#$($ids27.canonical)]　1 個附件"
Choose-CollectionRow 'same.txt　[#'
Native-Menu 120 1 '儲存此資料列的附件副本…' -MenuOpen
$outfile=Join-Path $root 'typed-export.txt'
Native-File 1 '資料列附件' $outfile "已儲存資料列 #$($ids27.canonical) 的附件副本。" -SaveName 'same.txt' -AlreadyOpened
if ((Get-FileHash -LiteralPath $outfile).Hash.ToLowerInvariant() -cne $assets27.canonical.sha256) {throw 'Wrong attachment bytes'}
Native-Menu 120 3 '開啟資料列…'
Choose-CollectionRow "待處理　共同筆記  ($($ids27.canonical))"
Click-WorkspaceButton 119
Native-Menu 120 3 '新增白板…'
Collection-Prompt '新增白板' 'Research'
foreach ($role in 'canonical','other') {
    Native-Menu 120 1 '放入筆記…'
    Choose-CollectionRow $notes27[$role].title
}
foreach ($slot in 0,1) {[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,$slot,$false,$false,$true)}
Native-Menu 120 10 '子白板與文字…'
Native-Menu 120 10 '將選取的卡片收進新子白板…' -MenuOpen
Collection-Prompt '將卡片收進新子白板' 'Child'
$confirm=Wait-For 'confirm creation and move into child' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認收進子白板',6)}
Finish-Revision $confirm 6
$null=Wait-For 'parent shows focused branch tile' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將 2 張卡片收進「Child」，並定位子白板入口。'}
$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=@('exact','decoy');edges=,@('canonical','exact')};board=@{parent='Research';child='Child';role='canonical';placements=@('Child')}}
[IO.File]::WriteAllText($manifest27,($spec | ConvertTo-Json -Depth 8),$utf27)
Witness27 $db27 -Initial
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
Witness27 $backup27
$hash27=(Get-FileHash -LiteralPath $backup27).Hash
Copy-Item -LiteralPath $backup27 -Destination (Join-Path $copy27 'paperroom-data/notes.sqlite3')
foreach ($dir in $home27,$copy27) {
    $process=Start-Notebook $dir
    $window=$process.MainWindowHandle
    Collection27 'Source'
    Open-SortNamedView 1 'Ranked'
    Assert-RankedRows @('對照筆記','共同筆記')
    Witness27 (Join-Path $dir 'paperroom-data/notes.sqlite3')
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash -LiteralPath $backup27).Hash -cne $hash27) {throw 'Backup changed while verifying independent copy'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;notes=$ids27;backup_sha256=$hash27;attachment_sha256=$assets27.canonical.sha256;scenarios=@('two-sort-ui-order','scoped-forward-add','exact-reverse-remove-and-add','row-attachment-download','physical-ctrl-pair-to-child','reopen','isolated-backup-restore')} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
