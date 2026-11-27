# Dot-sourced after native import, relation-field declaration and Phase 28 helper loading.
# Only the downloaded Paperroom executable writes workspace state.
function Open-Review29 {
    Collection27 'Source'
    Open-SortNamedView 1 'SharedReview'
}
function New-Board29([string]$Name) {
    Open-NativeBoard28 ''
    Native-Menu 120 3 '新增白板…'
    Collection-Prompt '新增白板' $Name
    $null=Wait-For "board $Name" {[PaperroomHighlightGui]::Read((Control $window 115)).Contains($Name)}
}
function Add-ExactBoardNote29([string]$Role) {
    # The board picker displays titles only, in notes.id ascending order. Validate its
    # entire observed sequence against this disposable database before selecting a duplicate.
    $records = @'
import json,pathlib,sqlite3,sys
with sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+"?mode=ro",uri=True) as c:
    print(json.dumps(c.execute("select id,title from notes where trash_root is null order by id").fetchall()))
'@ | python -B - $db27
    if ($LASTEXITCODE -ne 0) {throw 'Could not witness board picker identities'}
    $expected=ConvertFrom-Json -InputObject $records
    Native-Menu 120 1 '放入筆記…'
    $labels=@(Collection-MenuLabels)
    if ($labels.Count -ne $expected.Count) {throw 'Board source choices changed'}
    $selected=0
    for ($i=0; $i -lt $labels.Count; $i++) {
        if ($labels[$i] -cne $expected[$i][1]) {throw 'Board source identity order changed'}
        if ($ids27[$Role] -eq $expected[$i][0]) {$selected=$i+1}
    }
    if (!$selected) {throw "Board source missing: $Role"}
    Native-Menu 120 $selected $notes27[$Role].title -MenuOpen
}
function Wait-Marker29([string]$Role) {
    $null=Wait-For "exact original $Role title and Markdown marker" {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $notes27[$Role].title -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains($notes27[$Role].marker) -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
}
function Set-FromNote29([string]$Key,[string]$Old,[string]$New) {
    Native-Menu 113 4 '設定文字屬性…'
    Collection-Prompt '文字屬性' $Key
    Rule-Prompt '設定文字屬性' $Old $New
}
function Board-Jump29 {
    if ([PaperroomHighlightGui]::Read((Control $window 119)) -ceq '返回筆記') {
        Click-WorkspaceButton 119
        $null=Wait-For 'return to note before resolving an exact placement' {
            [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板'
        }
    }
    Select-Note '共同筆記'
    Wait-Marker29 'canonical'
    Native-Menu 120 60003 '查看此筆記的原生白板位置…'
    $originChoices=@(Collection-MenuLabels)
    $index=[Array]::IndexOf($originChoices,'Research')
    if ($originChoices.Count -ne 2 -or $index -lt 0) {throw "Missing unique parent origin: $originChoices"}
    Native-Menu 120 ($index+1) 'Research' -MenuOpen
    $null=Wait-For 'parent card focused before right-click' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位白板上的筆記卡片。' -and
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains('Research')
    }
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
    Native-Menu 120 8 '查看這張卡片的其他白板位置…' -MenuOpen
    $locations=@(Collection-MenuLabels)
    if ($locations.Count -ne 1 -or !$locations[0].Contains('Research › Child')) {
        throw "The selected note has unexpected locations: $($locations -join '|')"
    }
    Choose-CollectionRow $locations[0]
    $null=Wait-For 'cross-level original note precisely located' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains('Child') -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位白板上的筆記卡片。'
    }
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
    Wait-Marker29 'canonical'
}
function Open-Explanation29([string]$Role,[string[]]$Expected,[string]$Target,[int]$Total) {
    Collection27 'Source'
    Native-Menu 120 5 '檢視公式結果…'
    Choose-CollectionRow 'DurationTotal'
    $results=Wait-For 'native result action' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','集合公式結果',6)}
    $rows=[PaperroomHighlightGui]::Read((Control $results 101))
    if ($rows -cnotmatch ('(?m)^'+[regex]::Escape($notes27[$Role].title)+'\s+'+$Total+'\r?$')) {throw "Unexpected total for $Role in $rows"}
    Finish-Revision $results 6
    Choose-CollectionRow "$($notes27[$Role].title)　[#$($ids27[$Role])]"
    $report=Wait-For 'native immutable rollup explanation' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','彙總來源明細',6)}
    $body=[PaperroomHighlightGui]::Read((Control $report 101))
    if (!$body.Contains("結果：$Total") -or !$body.Contains("$($notes27[$Role].title)　[#$($ids27[$Role])]")) {throw "Wrong saved rollup row or total: $body"}
    foreach ($key in @('exact','decoy')) {
        $needle="場景　[#$($ids27[$key])]"
        if ($Expected -contains $key) {
            $value=if ($key -eq 'exact') {12} else {1}
            if (!$body.Contains("$needle　｜ $value")) {throw "Missing exact contribution $needle ($value): $body"}
        } elseif ($body.Contains($needle)) {throw "Unexpected same-title contributor: $body"}
    }
    Finish-Revision $report 6
    $options=@(Collection-MenuLabels)
    if ($options.Count -ne $Expected.Count) {throw "Wrong contributor count in $($options -join '|')"}
    Choose-CollectionRow "場景　[#$($ids27[$Target])]"
    Wait-Marker29 $Target
}
function Snapshot29([string]$Database,[switch]$First) {
    $arguments=@((Join-Path $PSScriptRoot 'check_shared_note_workflow.py'),$Database,$manifest27)
    if (!$First) {$arguments+=$prior27}
    $json=& python -B @arguments
    if ($LASTEXITCODE -ne 0 -or !$json) {throw 'Shared-note SQLite proof failed'}
    if ($First) {[IO.File]::WriteAllText($prior27,($json -join [Environment]::NewLine),$utf27)}
}

if ($SharedFixtureCase -ne 'shared-workflow') {
    . (Join-Path $PSScriptRoot 'windows-phase31-shared-fixture.ps1')
    return
}

Collection27 'Source'
BranchRelation27 canonical $false 1 exact
BranchRelation27 canonical $false 1 decoy
BranchRelation27 other $false 1 decoy
Save-Rollup28 'canonical' $false $true 'DurationTotal'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 1 '儲存新檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'SharedReview'
Native-Menu 120 2 '表格' -MenuOpen
Collection-Prompt '集合檢視'
Collection-Prompt '集合檢視'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
Native-Menu 120 1 '新增篩選…' -MenuOpen
Choose-CollectionRow '其他文字欄位…'
Collection-Prompt '文字欄位' 'Batch'
Native-Menu 120 1 '等於' -MenuOpen
Collection-Prompt '等於' 'A'
Native-Menu 120 1 '新增篩選…' -MenuOpen
Choose-CollectionRow '公式結果…'
Choose-CollectionRow 'DurationTotal'
Native-Menu 120 1 '數字結果' -MenuOpen
Native-Menu 120 5 '大於／晚於' -MenuOpen
Collection-Prompt '大於／晚於' '0'
Native-Menu 120 2 '新增排序…' -MenuOpen
Choose-CollectionRow '公式結果…'
Choose-CollectionRow 'DurationTotal'
Native-Menu 120 1 '數字結果' -MenuOpen
Native-Menu 120 2 '降冪' -MenuOpen
Native-Menu 120 5 '儲存變更' -MenuOpen
Assert-RankedRows @('共同筆記','對照筆記')

New-Board29 'ExactSource'
Add-ExactBoardNote29 'exact'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
Wait-Marker29 'exact'
Set-FromNote29 'Duration' '8' '-0.5'
Open-Review29
Assert-RankedRows @('對照筆記','共同筆記')
Open-NativeBoard28 'ExactSource'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
Wait-Marker29 'exact'
Set-FromNote29 'Duration' '-0.5' '-1'
Open-Review29
Assert-RankedRows @('對照筆記')
Open-NativeBoard28 'ExactSource'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
Wait-Marker29 'exact'
Set-FromNote29 'Duration' '-1' '12'
New-Board29 'DecoySource'
Add-ExactBoardNote29 'decoy'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
Native-Menu 120 8 '查看這張卡片的其他白板位置…' -MenuOpen
$null=Wait-For 'same-title decoy has no other board placement' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這張卡片沒有其他原生白板位置。'
}
New-Board29 'Research'
Add-ExactBoardNote29 'canonical'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
Native-Menu 120 7 '從此卡建立子白板…' -MenuOpen
Collection-Prompt '從此卡建立子白板' 'Child'
$null=Wait-For 'new child has same card' {[PaperroomHighlightGui]::Read((Control $window 115)).Contains('Child')}
Board-Jump29
Set-FromNote29 'Batch' 'A' 'Hold'
Open-Review29
Assert-RankedRows @('對照筆記')
Board-Jump29
Set-FromNote29 'Batch' 'Hold' 'A'
Open-Review29
Assert-RankedRows @('共同筆記','對照筆記')
Open-Explanation29 canonical @('exact','decoy') exact 13
Open-Explanation29 other @('decoy') decoy 1

$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=@('exact','decoy');edges=@(@('canonical','exact'),@('canonical','decoy'),@('other','decoy'))};board=@{role='canonical';parent='Research';child='Child';placements=@('Research','Child')}}
[IO.File]::WriteAllText($manifest27,($spec | ConvertTo-Json -Depth 8),$utf27)
Snapshot29 $db27 -First
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash -LiteralPath $backup27).Hash
Copy-Item -LiteralPath $backup27 -Destination (Join-Path $copy27 'paperroom-data/notes.sqlite3')
foreach ($folder in @($home27,$copy27)) {
    if ((Get-FileHash (Join-Path $folder 'Paperroom.exe')).Hash -ine $ExeSha256) {throw 'Different application bytes in isolated home'}
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    Board-Jump29
    Open-Review29
    Assert-RankedRows @('共同筆記','對照筆記')
    Open-Explanation29 canonical @('exact','decoy') exact 13
    Open-Explanation29 other @('decoy') decoy 1
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    Snapshot29 (Join-Path $folder 'paperroom-data/notes.sqlite3')
}
if ((Get-FileHash $backup27).Hash -cne $backupHash) {throw 'Opening the isolated copy altered the source backup'}
@{exe_sha256=$ExeSha256;backup_sha256=$backupHash;os=(Get-CimInstance Win32_OperatingSystem).Caption;identity=(Get-Content $prior27 -Raw | ConvertFrom-Json);scenarios=@('two persistent AND conditions and numeric descending sort','editable shared board note and typed target property','cross-level placement to original note','same-title board isolation','sum from both named contributors and second row excluding same-title sibling','source and restored workspace reopen')} |
    ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: named compound view, source/board editing, precise contributions and isolated restore.'
