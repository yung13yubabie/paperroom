# Real native CLI harness for shared note, relationship rollups and recoverable hierarchy.
function Save-Rollup28([string]$Role,[bool]$Reverse,[bool]$Sum,[string]$Name) {
    Native-Menu 120 27 '編輯資料列的關聯欄位…'
    Choose-CollectionRow "待處理　$($notes27[$Role].title)　[#$($ids27[$Role])]"
    $direction=if ($Reverse) {'反向'} else {'正向'}
    Choose-CollectionRow "$direction　關聯　｜ "
    if ($Sum) {
        Native-Menu 120 5 '儲存數值加總公式…' -MenuOpen
        Choose-CollectionRow 'Duration'
    } else {
        Native-Menu 120 4 '儲存關聯計數公式…' -MenuOpen
    }
    Collection-Prompt '儲存集合彙總公式' $Name
    $null=Wait-For 'native calculation saved' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存集合彙總公式。'
    }
}
function Compare-Rollup28([string]$Formula,[hashtable]$Rows,[int]$Occurrences=1) {
    Native-Menu 120 5 '檢視公式結果…'
    Choose-CollectionRow $Formula
    $dialog=Wait-For 'formula values visibly shown' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','集合公式結果',1)
    }
    $text=[PaperroomHighlightGui]::Read((Control $dialog 101))
    foreach ($name in $Rows.Keys) {
        $pattern='(?m)^'+[regex]::Escape($name)+'\s+'+[regex]::Escape([string]$Rows[$name])+'\r?$'
        if ([regex]::Matches($text,$pattern).Count -ne $Occurrences) { throw "Wrong visible $Formula result or duplicate count for $name. Window showed: $text" }
    }
    Finish-Revision $dialog 1
}
function Open-RollupView28 {
    Collection27 'Source'
    Open-SortNamedView 1 'RollupRanked'
}
function TargetDuration28([string]$Old,[string]$New) {
    Collection27 'Target'
    Native-Menu 120 18 '編輯資料列的型別欄位…'
    Choose-CollectionRow "待處理　場景  ($($ids27.exact))"
    Choose-CollectionRow "Duration　$Old"
    Rule-Prompt 'Duration' $Old $New
    $null=Wait-For 'typed target field committed' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新資料列欄位。'
    }
}
function Open-NativeBoard28([string]$Title) {
    if ([PaperroomHighlightGui]::Read((Control $window 119)) -cne '返回筆記' -or
        [PaperroomHighlightGui]::Read((Control $window 120)) -cne '白板操作') {
        Select-Note '共同筆記'
        Click-WorkspaceButton 119
    }
    $null=Wait-For 'native board controls ready before board selection' {
        if ([PaperroomHighlightGui]::Read((Control $window 119)) -cne '返回筆記' -or
            [PaperroomHighlightGui]::Read((Control $window 120)) -cne '白板操作') { return $false }
        try {
            $area=[PaperroomHighlightGui]::CanvasArea($window,$process.Id)
            $button=[PaperroomHighlightGui]::ButtonPoint($window,$process.Id,(Control $window 120))
            return $button.Y -lt $area.Top
        } catch { return $false }
    }
    if (!$Title) { return }
    Native-Menu 120 2 '切換白板…'
    Choose-CollectionRow $Title
    $null=Wait-For "native board $Title opened" {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Title)
    }
}
function Unfold-Native28([switch]$Decline) {
    Native-Menu 120 10 '子白板與文字…'
    Native-Menu 120 11 '將子白板卡片展回上層…' -MenuOpen
    $confirm=Wait-For 'native unfold confirmation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認展回上層',7)
    }
    if ($Decline) { Finish-Revision $confirm 7; return }
    Finish-Revision $confirm 6
    $null=Wait-For 'unfolded both cards and revealed the original child tile' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已展回 2 張卡片，並定位原子白板入口。'
    }
}
function Structural-Proof28([string]$Database,[string]$Stage,[string]$Prior='') {
    $state=@'
from contextlib import closing
import sqlite3,json,sys
from pathlib import Path
path,stage,prior,id0,id1,exact,decoy=sys.argv[1:]
with closing(sqlite3.connect(Path(path).as_uri()+"?mode=ro",uri=True)) as c:
    parent,=c.execute("select id from boards where title='Research'").fetchone()
    child,=c.execute("select id from boards where title='Child'").fetchone()
    tile,x,y=c.execute("select id,x,y from board_objects where board_id=? and child_id=?",(parent,child)).fetchone()
    notes=sorted((int(id0),int(id1)))
    placements=c.execute("""select note_id,board_id,x,y,width,height from board_card_geometry
             where board_id in (?,?) order by note_id""",(parent,child)).fetchall()
    expected=parent if stage=="unfolded" else child
    assert [(row[0],row[1]) for row in placements]==[(note,expected) for note in notes],placements
    rules,=c.execute("select rules_json from collection_views where name='RollupRanked'").fetchone()
    assert json.loads(rules)==[False,[["DurationTotal","formula-number",4,"0"]],[["DurationTotal","formula-number",True]]],rules
    terms=dict(c.execute("select name,expression from collection_formulas"))
    field_id,=c.execute("select id from relation_fields where name='關聯'").fetchone()
    assert terms["DurationTotal"]==f'field_sum("{field_id}","out","Duration")',terms
    assert terms["RelatedCount"]==f'field_count("{field_id}","out")',terms
    assert terms["IncomingCount"]==f'field_count("{field_id}","in")',terms
    for note,value in [(int(exact),"12"),(int(decoy),"1")]:
        assert c.execute("select value from note_properties where note_id=? and key='Duration'",(note,)).fetchone()==(value,)
    assert c.execute("select source_id,target_id from note_relations where name='關聯'").fetchall()==[(int(id0),int(decoy))]
    marker=dict(tile=[parent,child,tile],position=[x,y],geometry=[[row[0],*row[2:]] for row in placements])
    if prior:
        before=json.loads(Path(prior).read_text(encoding="utf-8"))
        assert before["tile"]==marker["tile"] and before["geometry"]==marker["geometry"]
        assert (before["position"]!=marker["position"])==(stage=="unfolded")
print(json.dumps(marker))
'@ | python -B - $Database $Stage $Prior $ids27.canonical $ids27.other $ids27.exact $ids27.decoy
    if ($LASTEXITCODE -ne 0 -or !$state) { throw "Native $Stage snapshot changed a stable identity, placement, or saved rule." }
    return [string]$state
}
function Identity-Proof28([string]$Database,[switch]$First) {
    $arguments=@($checker27,$Database,$manifest27)
    if (!$First) { $arguments+=$prior27 }
    $receipt=& python -B @arguments
    if ($LASTEXITCODE -ne 0 -or !$receipt) { throw 'Independent note, attachment, and edge witness failed.' }
    if ($First) { [IO.File]::WriteAllText($prior27,($receipt -join ([Environment]::NewLine)),$utf27) }
}

# An old same-name edge points to a different note. The typed field ignores it.
Select-Note '共同筆記'
Native-Menu 113 7 '筆記關聯…'
Native-Menu 113 1 '新增關聯…' -MenuOpen
Choose-CollectionRow "場景  ($($ids27.decoy))"
Collection-Prompt '新增關聯' '關聯'
Collection27 'Source'
BranchRelation27 canonical $false 1 exact
BranchRelation27 other $false 1 decoy
Save-Rollup28 'canonical' $false $true 'DurationTotal'
Save-Rollup28 'canonical' $false $false 'RelatedCount'
Compare-Rollup28 'DurationTotal' @{'共同筆記'=8;'對照筆記'=1}
Compare-Rollup28 'RelatedCount' @{'共同筆記'=1;'對照筆記'=1}
Collection27 'Target'
Save-Rollup28 'exact' $true $false 'IncomingCount'
Compare-Rollup28 'IncomingCount' @{'場景'=1} 2

Collection27 'Source'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 1 '儲存新檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'RollupRanked'
Native-Menu 120 2 '表格' -MenuOpen
Collection-Prompt '集合檢視'
Collection-Prompt '集合檢視'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
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

TargetDuration28 '8' '0'
Collection27 'Source'
Compare-Rollup28 'DurationTotal' @{'共同筆記'=0;'對照筆記'=1}
Compare-Rollup28 'RelatedCount' @{'共同筆記'=1;'對照筆記'=1}
Open-RollupView28
Assert-RankedRows @('對照筆記')

Collection27 'Target'
BranchRelation27 exact $true 2 canonical
Collection27 'Source'
Compare-Rollup28 'RelatedCount' @{'共同筆記'=0;'對照筆記'=1}
Collection27 'Target'
BranchRelation27 exact $true 1 canonical
TargetDuration28 '0' '12'
Open-RollupView28
Assert-RankedRows @('共同筆記','對照筆記')

Open-NativeBoard28 ''
Native-Menu 120 3 '新增白板…'
Collection-Prompt '新增白板' 'Research'
foreach ($role in 'canonical','other') {
    Native-Menu 120 1 '放入筆記…'
    Choose-CollectionRow $notes27[$role].title
}
foreach ($slot in 0,1) { [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,$slot,$false,$false,$true) }
Native-Menu 120 10 '子白板與文字…'
Native-Menu 120 10 '將選取的卡片收進新子白板…' -MenuOpen
Collection-Prompt '將卡片收進新子白板' 'Child'
$confirm=Wait-For 'confirm the child move' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認收進子白板',6)
}
Finish-Revision $confirm 6
$null=Wait-For 'wrapped child tile visibly selected' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將 2 張卡片收進「Child」，並定位子白板入口。'
}
$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=@('exact','decoy');edges=@(@('canonical','exact'),@('other','decoy'))}}
[IO.File]::WriteAllText($manifest27,($spec | ConvertTo-Json -Depth 8),$utf27)
Identity-Proof28 $db27 -First
$priorShape=Join-Path $root 'wrapped-positions.json'
[IO.File]::WriteAllText($priorShape,(Structural-Proof28 $db27 'wrapped'),$utf27)
Open-NativeBoard28 'Child'
Unfold-Native28 -Decline
$null=Structural-Proof28 $db27 'wrapped' $priorShape
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$actualBackup=(Get-FileHash -LiteralPath $backup27).Hash
Copy-Item -LiteralPath $backup27 -Destination (Join-Path $copy27 'paperroom-data/notes.sqlite3')
foreach ($folder in $home27,$copy27) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    $database=Join-Path $folder 'paperroom-data/notes.sqlite3'
    Identity-Proof28 $database
    $null=Structural-Proof28 $database 'wrapped' $priorShape
    Open-NativeBoard28 'Child'
    Unfold-Native28
    $null=Structural-Proof28 $database 'unfolded' $priorShape
    Identity-Proof28 $database
    Open-RollupView28
    Assert-RankedRows @('共同筆記','對照筆記')
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    Identity-Proof28 $database
    $null=Structural-Proof28 $database 'unfolded' $priorShape
}
if ((Get-FileHash -LiteralPath $backup27).Hash -cne $actualBackup) { throw 'Opening the isolated copy changed the backup bytes.' }
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;scenarios=@('exact forward sum and count','reverse count','persistent numeric filter and descending sort','zero target with retained relationship','reverse remove and re-add','same-name attachment identity and hash','declined unfold no writes','reopen and isolated restore','unfold exact placements and preserve child tile ID')} |
    ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: native typed rollup creation, derived filter/sort, and reversible shared-note child hierarchy including isolated restore.'

