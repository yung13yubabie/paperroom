# Uses the exact downloaded EXE and prior native collection helpers.
function New-GroupView([string]$Name, [int]$Layout) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 7 '儲存分組與排序檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' $Name
    $caption = if ($Layout -eq 1) { '看板' } else { '表格' }
    Native-Menu 120 $Layout $caption -MenuOpen
    Collection-Prompt '分組' '分類'
    Collection-Prompt '排序'
}
function Set-GroupTitleFilter([string]$Previous, [string]$Next) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 17 '編輯狀態與標題篩選…' -MenuOpen
    Collection-Prompt '編輯集合篩選'
    Rule-Prompt '編輯集合篩選' $Previous $Next
}
function Open-GroupView([string]$Caption) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
    Native-Menu 120 1 $Caption -MenuOpen
}
function Assert-GroupRows([string[]]$Expected) {
    Native-Menu 120 3 '開啟資料列…'
    $actual = @(foreach ($text in @(Collection-MenuLabels)) {
        if ($text -cnotmatch '^待處理　(.+)  \(\d+\)$') { throw "Unexpected collection row: $text" }
        $Matches[1]
    })
    if (($actual -join '|') -cne ($Expected -join '|')) { throw "Wrong visible order: [$actual], expected [$Expected]" }
    $picker = [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0)
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null = Wait-For 'native group row picker dismissed' { ![PaperroomHighlightGui]::IsWindowVisible($picker) }
    Write-Host "Observed genuine visible group rows: $($actual -join ', ')."
}
function Click-GroupHeader([bool]$Table) {
    $bounds = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
    $scale = [PaperroomHighlightGui]::GetDpiForWindow($window) / 96.0
    $y = if ($Table) { 57 } else { 26 }
    $point = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,($bounds.Left+[int](48*$scale)),($bounds.Top+[int]($y*$scale)))
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$window,$point,0)
}
function Check-FoldState([string]$Db, [bool]$Reset) {
    @'
from contextlib import closing
from pathlib import Path
import json, sqlite3, sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute('PRAGMA user_version').fetchone() == (35,)
    assert db.execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    saved = db.execute("""
        SELECT c.name,v.name,grouping_key,grouping_kind,group_json
        FROM collection_group_folds f JOIN collection_views v ON v.id=f.view_id
        JOIN collections c ON c.id=v.collection_id ORDER BY c.name
    """).fetchall()
    expected = [] if sys.argv[2]=='True' else [
      ('A-Folds','A-Grouped','\u5206\u985e','text','null'),
      ('B-Folds','B-Grouped','\u5206\u985e','text',json.dumps('\u672a\u8a2d\u5b9a',ensure_ascii=False)),
    ]
    assert saved==expected,saved
    raw,keyword=db.execute("SELECT rules_json,keyword FROM collection_views WHERE name='A-Grouped'").fetchone()
    assert keyword=='' and json.loads(raw) == [False,[],[
        ['\u5de5\u6642','number',True],['\u5099\u8a3b',None,False]]],(raw,keyword)
    assert db.execute("SELECT columns_json FROM collection_display_columns").fetchone() == (json.dumps(['\u5de5\u6642'],ensure_ascii=False),)
print('Verified exact persisted group identities, untouched two-column sort and chosen display column.')
'@ | python - $Db ($Reset.ToString())
    if ($LASTEXITCODE -ne 0) { throw 'The group preference or existing view settings differ in SQLite.' }
}

$foldHome = Join-Path $root 'fold-groups'
$foldRestore = Join-Path $root 'fold-restore'
$foldDb = Join-Path $foldHome 'paperroom-data\notes.sqlite3'
$foldRestoreDb = Join-Path $foldRestore 'paperroom-data\notes.sqlite3'
$foldBackup = Join-Path $root 'fold-backup.sqlite3'
$foldCsv = Join-Path $root 'folds.csv'
[IO.File]::WriteAllLines($foldCsv,[string[]]@(
    'Name,分類,工時,備註','Empty,,3,c','Literal,未設定,2,b','RoadEarly,路線,1,z','RoadLate,路線,9,a'
),[Text.UTF8Encoding]::new($false))
$foldCsvHash = (Get-FileHash -LiteralPath $foldCsv).Hash
foreach ($folder in $foldHome,$foldRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
$process = Start-Notebook $foldHome
$window = $process.MainWindowHandle
Native-File 9 '匯入 CSV 表格…' $foldCsv '已從 CSV 匯入 4 列'
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Collection-Prompt '設定工作區屬性型別' '工時'
Native-Menu 109 1 '數字' -MenuOpen
Set-UpColumnCollection 'A-Folds' @('Empty','Literal','RoadEarly','RoadLate')
New-GroupView 'A-Grouped' 2
Set-DisplayColumns '' '工時'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
Native-Menu 120 2 '新增排序…' -MenuOpen
Choose-CollectionRow '工時　'
Native-Menu 120 2 '降冪' -MenuOpen
Native-Menu 120 2 '新增排序…' -MenuOpen
Choose-CollectionRow '其他文字欄位…'
Collection-Prompt '文字欄位' '備註'
Native-Menu 120 1 '升冪' -MenuOpen
Native-Menu 120 5 '儲存變更' -MenuOpen

Assert-GroupRows @('Empty','Literal','RoadLate','RoadEarly')
Click-GroupHeader $true
Assert-GroupRows @('Literal','RoadLate','RoadEarly')
Click-GroupHeader $true
Assert-GroupRows @('Empty','Literal','RoadLate','RoadEarly')
Click-GroupHeader $true
Set-GroupTitleFilter '' 'Road'
Assert-GroupRows @('RoadLate','RoadEarly')
Set-GroupTitleFilter 'Road' ''
Assert-GroupRows @('Literal','RoadLate','RoadEarly')

Set-UpColumnCollection 'B-Folds' @('Empty','Literal','RoadEarly','RoadLate')
New-GroupView 'B-Grouped' 1
Assert-GroupRows @('Empty','Literal','RoadEarly','RoadLate')
Click-GroupHeader $false
Assert-GroupRows @('Literal','RoadEarly','RoadLate')
Click-GroupHeader $false
Assert-GroupRows @('Empty','Literal','RoadEarly','RoadLate')
Native-Menu 120 25 '群組收合…'
Choose-CollectionRow '收合 「未設定」'
Assert-GroupRows @('Empty','RoadEarly','RoadLate')
Check-FoldState $foldDb $false
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $foldHome
$window = $process.MainWindowHandle
Open-ColumnsCollection 1 'A-Folds'
Open-GroupView 'A-Grouped（表格）'
Assert-GroupRows @('Literal','RoadLate','RoadEarly')
Open-ColumnsCollection 2 'B-Folds'
Open-GroupView 'B-Grouped（看板）'
Assert-GroupRows @('Empty','RoadEarly','RoadLate')
Native-File 4 '備份工作區…' $foldBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$foldSourceHash = (Get-FileHash -LiteralPath $foldDb).Hash
$foldBackupHash = (Get-FileHash -LiteralPath $foldBackup).Hash
Copy-Item -LiteralPath $foldBackup -Destination $foldRestoreDb
if ((Get-FileHash -LiteralPath $foldRestoreDb).Hash -cne $foldBackupHash) { throw 'The isolated copy differs from the UI backup.' }

$process = Start-Notebook $foldRestore
$window = $process.MainWindowHandle
Check-FoldState $foldRestoreDb $false
Open-ColumnsCollection 1 'A-Folds'
Open-GroupView 'A-Grouped（表格）'
Assert-GroupRows @('Literal','RoadLate','RoadEarly')
Native-Menu 120 24 '表格與清單欄位…'
Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
Choose-CollectionRow '路線　RoadLate  ('
$report = Wait-For 'native column projection after unfolding' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
}
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('工時　9')) { throw 'The chosen display column was not preserved.' }
Finish-Revision $report 1
Open-ColumnsCollection 2 'B-Folds'
Open-GroupView 'B-Grouped（看板）'
Assert-GroupRows @('Empty','RoadEarly','RoadLate')
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Collection-Prompt '設定工作區屬性型別' '分類'
Native-Menu 109 3 '日期' -MenuOpen
Open-ColumnsCollection 1 'A-Folds'
Open-GroupView 'A-Grouped（表格）'
Assert-GroupRows @('Empty','Literal','RoadLate','RoadEarly')
Open-ColumnsCollection 2 'B-Folds'
Open-GroupView 'B-Grouped（看板）'
Assert-GroupRows @('Empty','Literal','RoadEarly','RoadLate')
Check-FoldState $foldRestoreDb $true
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-FileHash -LiteralPath $foldDb).Hash -cne $foldSourceHash -or
    (Get-FileHash -LiteralPath $foldBackup).Hash -cne $foldBackupHash -or
    (Get-FileHash -LiteralPath $foldCsv).Hash -cne $foldCsvHash) { throw 'Source workspace, backup or CSV changed during isolated restore.' }
Write-Host "PASS: native group heading clicks and menu, independent blank and literal groups in two collections, filter edits, two sorts and chosen columns, restart, isolated backup and safe type change. EXE SHA-256 $ExeSha256."
