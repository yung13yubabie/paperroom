# Runs with the exact EXE unpacked and checked by the Windows artifact job.
$columnLf = [string][char]10
$columnCr = [string][char]13
function Open-ColumnsCollection([int]$Index, [string]$Name) {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Native-Menu 109 $Index $Name -MenuOpen
    $null = Wait-For 'collection canvas displayed' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
}

function Set-DisplayColumns([string]$Before, [string]$After, [switch]$Cancel, [switch]$Reject) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 2 '設定顯示順序…' -MenuOpen
    $dialog = Wait-For 'native multiline column editor' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','集合欄位顯示',1)
    }
    $edit = Control $dialog 101
    if ([PaperroomHighlightGui]::Read($edit).Replace($columnCr,'') -cne $Before) { throw 'Unexpected initial column order.' }
    if ($Before -cne $After) {
        $point = [PaperroomHighlightGui]::FilenamePoint($dialog,$process.Id,$edit)
        [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
        $null = Wait-For 'column editor focus' { [PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$dialog,$edit) }
        [void][PaperroomHighlightGui]::Send($edit,0x00B1,[IntPtr]::Zero,[IntPtr](-1))
        if ($After) { [PaperroomHighlightGui]::TypeEditorText($window,$process.Id,$edit,$After) }
        else { [PaperroomHighlightGui]::TapKey($window,$process.Id,0x2E) }
        $null = Wait-For 'exact physically entered column order' {
            [PaperroomHighlightGui]::Read($edit).Replace($columnCr,'') -ceq $After
        }
    }
    if ($Cancel) {
        Finish-Revision $dialog 2
    } else {
        Finish-Revision $dialog 1
        if ($Reject) { Rule-Error '欄位顯示未儲存（可複製）' '每個欄位只能出現一次' }
        else {
            $null = Wait-For 'column settings saved' {
                [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存集合欄位顯示。'
            }
        }
    }
}

function Read-DisplayColumns([string[]]$Expected) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    $labels = @(Collection-MenuLabels)
    if ($labels.Count -ne 1 -or !$labels[0].Contains('Shared  (')) { throw 'Wrong rows in the current saved view.' }
    Choose-CollectionRow '待處理　Shared  ('
    $dialog = Wait-For 'native visible row projection' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
    }
    $actual = [PaperroomHighlightGui]::Read((Control $dialog 101)).Replace($columnCr,'')
    $wanted = (@('筆記標題　Shared','狀態　待處理') + $Expected) -join $columnLf
    if ($actual -cne $wanted) { throw "Wrong visible columns. Found [$actual], expected [$wanted]." }
    Finish-Revision $dialog 1
    Write-Host "Observed actual collection projection: $($Expected -join ' | ')."
}

function Set-UpColumnCollection([string]$Name, [string[]]$Titles) {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 2 '新增集合…' -MenuOpen
    Collection-Prompt '新增集合' $Name
    $null = Wait-For 'new collection canvas' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
    foreach ($title in $Titles) {
        Native-Menu 120 1 '加入筆記…'
        Choose-CollectionRow "$title  ("
        Collection-Prompt '加入集合'
    }
}

function Test-DisplayDb([string]$Db) {
    @'
from contextlib import closing
from pathlib import Path
import json, sqlite3, sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute('PRAGMA user_version').fetchone() == (38,)
    assert db.execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    settings = {name:json.loads(raw) for name,raw in db.execute(
      'SELECT c.name,d.columns_json FROM collection_display_columns d JOIN collections c ON c.id=d.collection_id')}
    assert settings == {'A-Columns':['\u5de5\u6642','\u526a\u8f2f'],'B-Columns':['\u5099\u8a3b']}, settings
    values = dict(db.execute("SELECT p.key,p.value FROM notes n JOIN note_properties p ON p.note_id=n.id WHERE n.title='Shared'"))
    assert values == {'Name':'Shared','\u526a\u8f2f':'\u7c97\u526a','\u5de5\u6642':'003.50','\u5099\u8a3b':'\u539f\u7a3f'},values
    assert db.execute("SELECT count(*) FROM collection_notes m JOIN notes n ON n.id=m.note_id WHERE n.title='Shared'").fetchone() == (2,)
    assert db.execute("SELECT filter_key,filter_kind,filter_value FROM collection_views WHERE name='A-Table'").fetchone() == ('\u5de5\u6642','number','3.5')
print('Verified separate display orders, original raw values, shared note identity and the existing filter.')
'@ | python - $Db
    if ($LASTEXITCODE -ne 0) { throw 'Expected column settings or source data differ in SQLite.' }
}

$columnHome = Join-Path $root 'column-display'
$columnRestore = Join-Path $root 'column-restore'
$columnDb = Join-Path $columnHome 'paperroom-data\notes.sqlite3'
$columnRestoreDb = Join-Path $columnRestore 'paperroom-data\notes.sqlite3'
$columnBackup = Join-Path $root 'columns-backup.sqlite3'
$columnCsv = Join-Path $root 'columns.csv'
[IO.File]::WriteAllLines($columnCsv,[string[]]@('Name,剪輯,工時,備註','Shared,粗剪,003.50,原稿','Other,完成,8.0,不同'),[Text.UTF8Encoding]::new($false))
$columnCsvHash = (Get-FileHash -LiteralPath $columnCsv).Hash
foreach ($folder in $columnHome,$columnRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
$process = Start-Notebook $columnHome
$window = $process.MainWindowHandle
Native-File 9 '匯入 CSV 表格…' $columnCsv '已從 CSV 匯入 2 列'
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Collection-Prompt '設定工作區屬性型別' '工時'
Native-Menu 109 1 '數字' -MenuOpen
Set-UpColumnCollection 'A-Columns' @('Shared','Other')
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 5 '儲存含屬性篩選的新檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'A-Table'
Native-Menu 120 2 '表格' -MenuOpen
Collection-Prompt '集合檢視'
Collection-Prompt '集合檢視'
Native-Menu 120 1 '工時　' -MenuOpen -CaptionPrefix
Collection-Prompt '屬性完全相符' '3.5'
Read-DisplayColumns @()

Native-Menu 120 24 '表格與清單欄位…'
Native-Menu 120 1 '檢視可用屬性…' -MenuOpen
Collection-Prompt '可用的集合屬性' '剪'
$available = Wait-For 'available property report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','可用的集合屬性',1)
}
if ([PaperroomHighlightGui]::Read((Control $available 101)) -cne '剪輯') { throw 'The available property search has incorrect matches.' }
Finish-Revision $available 1

$firstOrder = '剪輯','工時' -join $columnLf
$finalOrder = '工時','剪輯' -join $columnLf
Set-DisplayColumns '' $firstOrder
Read-DisplayColumns @('剪輯　粗剪','工時　003.50')
Set-DisplayColumns $firstOrder ('工時','工時' -join $columnLf) -Reject
Set-DisplayColumns $firstOrder '備註' -Cancel
Set-DisplayColumns $firstOrder $finalOrder
Read-DisplayColumns @('工時　003.50','剪輯　粗剪')
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 10 '儲存清單檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'A-List'
Collection-Prompt '集合清單'
Collection-Prompt '集合清單' 'Shared'
Read-DisplayColumns @('工時　003.50','剪輯　粗剪')
$board = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
$scale = [PaperroomHighlightGui]::GetDpiForWindow($window) / 96.0
$point = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,($board.Left+[int](48*$scale)),($board.Top+[int](57*$scale)))
[PaperroomHighlightGui]::MouseAction($window,$process.Id,$window,$point,0)
$null = Wait-For 'physical click on the first custom list row opened its original note' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'Shared' -and
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟清單筆記'
}

Set-UpColumnCollection 'B-Columns' @('Shared')
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 1 '儲存新檢視…' -MenuOpen
Collection-Prompt '儲存新檢視' 'B-Table'
Native-Menu 120 2 '表格' -MenuOpen
Collection-Prompt '集合檢視'
Collection-Prompt '集合檢視'
Read-DisplayColumns @()
Set-DisplayColumns '' '備註'
Read-DisplayColumns @('備註　原稿')
Set-DisplayColumns '備註' ''
Read-DisplayColumns @()
Native-Menu 120 24 '表格與清單欄位…'
Native-Menu 120 3 '恢復原有欄位' -MenuOpen
Set-DisplayColumns '' '備註'
Test-DisplayDb $columnDb

Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $columnHome
$window = $process.MainWindowHandle
Open-ColumnsCollection 1 'A-Columns'
Open-SortNamedView 2 'A-Table'
Read-DisplayColumns @('工時　003.50','剪輯　粗剪')
Open-ColumnsCollection 2 'B-Columns'
Open-SortNamedView 1 'B-Table'
Read-DisplayColumns @('備註　原稿')
Native-File 4 '備份工作區…' $columnBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$columnSourceHash = (Get-FileHash -LiteralPath $columnDb).Hash
$columnBackupHash = (Get-FileHash -LiteralPath $columnBackup).Hash
Copy-Item -LiteralPath $columnBackup -Destination $columnRestoreDb
if ((Get-FileHash -LiteralPath $columnRestoreDb).Hash -cne $columnBackupHash) { throw 'The isolated copy differs from the UI backup.' }

$process = Start-Notebook $columnRestore
$window = $process.MainWindowHandle
Test-DisplayDb $columnRestoreDb
Open-ColumnsCollection 1 'A-Columns'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
Native-Menu 120 1 'A-List（清單）' -MenuOpen
Read-DisplayColumns @('工時　003.50','剪輯　粗剪')
Open-ColumnsCollection 2 'B-Columns'
Open-SortNamedView 1 'B-Table'
Read-DisplayColumns @('備註　原稿')
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-FileHash -LiteralPath $columnDb).Hash -cne $columnSourceHash -or
    (Get-FileHash -LiteralPath $columnBackup).Hash -cne $columnBackupHash -or
    (Get-FileHash -LiteralPath $columnCsv).Hash -cne $columnCsvHash) { throw 'The source workspace, backup or CSV changed during isolated restore.' }
Write-Host "PASS: native ordered collection display of shared CSV properties in table and list, filtered membership, hide, reject, cancel, default reset, list click, reopen and isolated backup. EXE SHA-256 $ExeSha256."
