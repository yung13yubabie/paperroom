# Accept the exact EXE unpacked by the downloadable-artifact verification job.
function New-DueView([string]$Name, [int]$Operator, [string]$Label, [switch]$Formula) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 1 '儲存新檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' $Name
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '集合檢視'
    Collection-Prompt '集合檢視'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
    Native-Menu 120 1 '新增篩選…' -MenuOpen
    if ($Formula) {
        Choose-CollectionRow '公式結果…'
        Native-Menu 120 1 '公式截止' -MenuOpen
        Native-Menu 120 3 '日期結果' -MenuOpen
    } else {
        Choose-CollectionRow '截止日　'
    }
    Native-Menu 120 $Operator $Label -MenuOpen
    Native-Menu 120 5 '儲存變更' -MenuOpen
}

function Assert-DueRows([string[]]$Expected) {
    Native-Menu 120 3 '開啟資料列…'
    $rowLabels = @(Collection-MenuLabels)
    $actual = @(foreach ($entry in $rowLabels) {
        if ($entry -cnotmatch '^待處理　(.+)  \(\d+\)$') { throw "Unexpected date-filter row: $entry" }
        $Matches[1]
    })
    if ((($actual | Sort-Object) -join '|') -cne (($Expected | Sort-Object) -join '|')) {
        throw "Wrong date-filter membership: found [$($actual -join ', ')], expected [$($Expected -join ', ')]."
    }
    $picker = [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0)
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null = Wait-For 'native date-filter result picker dismissed' { ![PaperroomHighlightGui]::IsWindowVisible($picker) }
    Write-Host "Observed relative-date result through the actual row picker: $($actual -join ', ')."
}

function Iso-DueDate([datetime]$Date) {
    $Date.ToString('yyyy-MM-dd',[Globalization.CultureInfo]::InvariantCulture)
}

function Test-DueState([string]$Database, [string]$Expected) {
    @'
from contextlib import closing
from pathlib import Path
import json, sqlite3, sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute('PRAGMA user_version').fetchone() == (37,)
    rules = {name: json.loads(raw) for name,raw in db.execute('SELECT name,rules_json FROM collection_views')}
    assert rules == {
        'A-Week': [False,[['\u622a\u6b62\u65e5','date',17,'']],[]],
        'B-Next': [False,[['\u622a\u6b62\u65e5','date',23,'']],[]],
        'C-Late': [False,[['\u516c\u5f0f\u622a\u6b62','formula-date',24,'']],[]],
    },rules
    rows = db.execute("""SELECT n.title, p.value FROM collection_notes m JOIN notes n ON n.id=m.note_id
                         LEFT JOIN note_properties p ON n.id=p.note_id AND p.key=?""",('\u622a\u6b62\u65e5',)).fetchall()
    values = dict(rows)
    assert len(values)==9, values
    assert values['Eight']==sys.argv[2], values
    assert not values['Blank'] and values['Invalid']=='2028-02-30', values
print('Verified exact saved relative operator IDs, old invalid value, blank, and the edited date in SQLite.')
'@ | python - $Database $Expected
    if ($LASTEXITCODE -ne 0) { throw 'Date filter rules or values do not match in the real workspace.' }
}

$dueHome = Join-Path $root 'due-filters'
$dueRestore = Join-Path $root 'due-restore'
$dueDb = Join-Path $dueHome 'paperroom-data\notes.sqlite3'
$dueRestoreDb = Join-Path $dueRestore 'paperroom-data\notes.sqlite3'
$dueBackup = Join-Path $root 'due-backup.sqlite3'
$dueCsv = Join-Path $root 'dates.csv'
foreach ($folder in $dueHome, $dueRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
$today = (Get-Date).Date
$monday = $today.AddDays(-(([int]$today.DayOfWeek + 6) % 7))
$sunday = $monday.AddDays(6)
$dates = [ordered]@{
    PriorWeek = $monday.AddDays(-1)
    Monday = $monday
    Today = $today
    Sunday = $sunday
    Tomorrow = $today.AddDays(1)
    Seven = $today.AddDays(7)
    Eight = $today.AddDays(8)
}
$lines = @('Name,截止日')
foreach ($name in $dates.Keys) { $lines += "$name,$(Iso-DueDate $dates[$name])" }
$lines += @('Blank,','Invalid,2028-02-30')
[IO.File]::WriteAllLines($dueCsv,$lines,[Text.UTF8Encoding]::new($false))
$csvStamp = (Get-FileHash -LiteralPath $dueCsv).Hash

$process = Start-Notebook $dueHome
$window = $process.MainWindowHandle
Native-File 9 '匯入 CSV 表格…' $dueCsv '已從 CSV 匯入 9 列'
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Collection-Prompt '設定工作區屬性型別' '截止日'
Native-Menu 109 3 '日期' -MenuOpen
Native-Menu 109 10 '資料集合與看板…'
Native-Menu 109 2 '新增集合…' -MenuOpen
Collection-Prompt '新增集合' 'DueAudition'
$null = Wait-For 'new date-filter collection' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
foreach ($title in (@($dates.Keys) + @('Blank','Invalid'))) {
    Native-Menu 120 1 '加入筆記…'
    Choose-CollectionRow "$title  ("
    Collection-Prompt '加入集合'
}
Native-Menu 120 4 '新增公式…'
Collection-Prompt '新增公式' '公式截止'
Collection-Prompt '新增公式' 'typed("截止日")'

$weekRows = @(foreach ($name in $dates.Keys) { if ($dates[$name] -ge $monday -and $dates[$name] -le $sunday) { $name } })
$nextRows = @(foreach ($name in $dates.Keys) { if ($dates[$name] -gt $today -and $dates[$name] -le $today.AddDays(7)) { $name } })
$lateRows = @(foreach ($name in $dates.Keys) { if ($dates[$name] -lt $today) { $name } })

New-DueView 'A-Week' 18 '本週'
Assert-DueRows $weekRows
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 3 '顯示全部資料列（看板）' -MenuOpen
New-DueView 'B-Next' 24 '未來七天（不含今天）'
Assert-DueRows $nextRows
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 3 '顯示全部資料列（看板）' -MenuOpen
New-DueView 'C-Late' 25 '逾期（早於今天）' -Formula
Assert-DueRows $lateRows

$initial = Iso-DueDate $dates['Eight']
$editDay = $today.AddDays(-1)
$edited = Iso-DueDate $editDay
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 3 '顯示全部資料列（看板）' -MenuOpen
Native-Menu 120 18 '編輯資料列的型別欄位…'
Choose-CollectionRow '待處理　Eight  ('
Native-Menu 120 1 "截止日　$initial" -MenuOpen -CaptionPrefix
Rule-Prompt '截止日' $initial '2028-02-30'
Rule-Error '欄位未儲存' '2028-02-30'
Test-DueState $dueDb $initial
Rule-Prompt '截止日' '2028-02-30' $edited
$null = Wait-For 'corrected date saved through the native editor' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新資料列欄位。'
}
Test-DueState $dueDb $edited
$lateRows += 'Eight'
if ($editDay -ge $monday) { $weekRows += 'Eight' }
Open-SortNamedView 3 'C-Late'
Assert-DueRows $lateRows

Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $dueHome
$window = $process.MainWindowHandle
Open-SortCollection 'DueAudition'
Open-SortNamedView 1 'A-Week'
Assert-DueRows $weekRows
Open-SortNamedView 2 'B-Next'
Assert-DueRows $nextRows
Open-SortNamedView 3 'C-Late'
Assert-DueRows $lateRows
Native-File 4 '備份工作區…' $dueBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$sourceStamp = (Get-FileHash -LiteralPath $dueDb).Hash
$backupStamp = (Get-FileHash -LiteralPath $dueBackup).Hash
Copy-Item -LiteralPath $dueBackup -Destination $dueRestoreDb
if ((Get-FileHash -LiteralPath $dueRestoreDb).Hash -cne $backupStamp) { throw 'The isolated copy differs from the GUI backup.' }

$process = Start-Notebook $dueRestore
$window = $process.MainWindowHandle
Test-DueState $dueRestoreDb $edited
Open-SortCollection 'DueAudition'
Open-SortNamedView 1 'A-Week'
Assert-DueRows $weekRows
Open-SortNamedView 2 'B-Next'
Assert-DueRows $nextRows
Open-SortNamedView 3 'C-Late'
Assert-DueRows $lateRows
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-Date).Date -ne $today -or (Get-FileHash -LiteralPath $dueDb).Hash -cne $sourceStamp -or
    (Get-FileHash -LiteralPath $dueBackup).Hash -cne $backupStamp -or
    (Get-FileHash -LiteralPath $dueCsv).Hash -cne $csvStamp) { throw 'Local day, source workspace, backup, or imported CSV changed during verification.' }
Write-Host "PASS: native this-week, next-seven-days and overdue filters on a real CSV collection; blanks and invalid dates excluded, invalid edit rejected, corrected date recomputed, reopen and isolated backup restored. EXE SHA-256 $ExeSha256."
