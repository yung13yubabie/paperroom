# Executed by the native GUI suite against the downloaded EXE.
function Test-HistoryMetadata([string]$Database, [string]$Stage) {
    @'
from contextlib import closing
from pathlib import Path
import sqlite3, sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri() + '?mode=ro', uri=True)) as db:
    (note, version, body), = db.execute("SELECT id, version, body FROM notes WHERE title='MetadataHistory'")
    assert body == 'History fixture\n', repr(body)
    assert version == 1, version
    files = db.execute('SELECT id,name,content FROM attachments WHERE note_id=? ORDER BY id', (note,)).fetchall()
    properties = db.execute('SELECT key,value FROM note_properties WHERE note_id=? ORDER BY key', (note,)).fetchall()
    changed = sys.argv[2] == 'changed'
    assert files == ([(2,'Same.txt',b'BBB'),(3,'Same.txt',b'CCC')] if changed else
                     [(1,'Same.txt',b'AAA'),(2,'Same.txt',b'BBB')]), files
    expected = '\u73fe\u503c' if changed else '\u820a\u503c'
    assert properties == [('\u6b04\u4f4d',expected)], properties
    revisions = db.execute('''
        SELECT h.id, a.original_id,a.content FROM note_revisions h
        JOIN note_revision_attachments a ON a.revision_id=h.id
        WHERE h.note_id=? ORDER BY h.id DESC, a.original_id
    ''', (note,)).fetchall()
    assert any(a == 1 and raw == b'AAA' for _,a,raw in revisions), revisions
    if not changed:
        newest = revisions[0][0]
        assert [(a,raw) for r,a,raw in revisions if r == newest] == [(2,b'BBB'),(3,b'CCC')], revisions
    print(f"Verified {sys.argv[2]} note metadata, exact attachment IDs/bytes, displaced version and unchanged text.")
'@ | python - $Database $Stage
    if ($LASTEXITCODE -ne 0) { throw "Exact SQLite metadata or bytes differ at $Stage." }
}

function Set-HistoryProperty([string]$Previous, [string]$Value) {
    Native-Menu 113 4 '設定文字屬性…'
    Rule-Prompt '文字屬性' '' '欄位'
    Rule-Prompt '設定文字屬性' $Previous $Value
}

function Add-HistoryFile([string]$Path) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 1 '加入本機附件…' -MenuOpen
    Native-File 1 '加入本機附件…' $Path '附件已保存到本機工作區。' -AlreadyOpened
}

$historyHome = Join-Path $root 'metadata-history'
$historyRestore = Join-Path $root 'metadata-restored'
$historyDb = Join-Path $historyHome 'paperroom-data\notes.sqlite3'
$historyRestoreDb = Join-Path $historyRestore 'paperroom-data\notes.sqlite3'
$historyBackup = Join-Path $root 'metadata-backup.sqlite3'
$historySource = Join-Path $root 'MetadataHistory.md'
[IO.File]::WriteAllText($historySource, "History fixture`n")
foreach ($folder in $historyHome, $historyRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
$historyFiles = foreach ($name in 'AAA','BBB','CCC') {
    $dir = Join-Path $root $name
    [void][IO.Directory]::CreateDirectory($dir)
    $file = Join-Path $dir 'Same.txt'
    [IO.File]::WriteAllBytes($file, [Text.Encoding]::ASCII.GetBytes($name))
    $file
}

$process = Start-Notebook $historyHome
$window = $process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $historySource '已匯入 1 篇 Markdown'
Select-Note 'MetadataHistory'
Set-HistoryProperty '' '舊值'
Add-HistoryFile $historyFiles[0]
Add-HistoryFile $historyFiles[1]
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 2 '管理附件…' -MenuOpen
Native-Menu 113 1 'Same.txt  (3 B)' -MenuOpen
Native-Menu 113 2 '刪除附件…' -MenuOpen
$delete = Wait-For 'native confirmation of first same-name attachment removal' {
    [PaperroomHighlightGui]::ForegroundAlert($window, $process.Id, '刪除附件')
}
Finish-Revision $delete 1
Add-HistoryFile $historyFiles[2]
Set-HistoryProperty '舊值' '現值'
Close-Notebook $process
$process.Dispose()
$process = $null
Test-HistoryMetadata $historyDb changed

$process = Start-Notebook $historyHome
$window = $process.MainWindowHandle
Select-Note 'MetadataHistory'
$preview = Open-Revision 3
$report = [PaperroomHighlightGui]::Read((Control $preview 101))
foreach ($line in '欄位 = 舊值','附件（2）','Same.txt　3 位元組　#1','Same.txt　3 位元組　#2') {
    if (!$report.Contains($line)) { throw "Missing historical metadata in the native preview: $line" }
}
Finish-Revision $preview 2
Test-HistoryMetadata $historyDb changed
$preview = Open-Revision 3
Finish-Revision $preview 1
$null = Wait-For 'metadata-only revision restored' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已還原選取的筆記版本。'
}
Test-HistoryMetadata $historyDb restored
Native-File 4 '備份工作區…' $historyBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$sourceStamp = (Get-FileHash -LiteralPath $historyDb).Hash
$backupStamp = (Get-FileHash -LiteralPath $historyBackup).Hash
Copy-Item -LiteralPath $historyBackup -Destination $historyRestoreDb
if ((Get-FileHash -LiteralPath $historyRestoreDb).Hash -cne $backupStamp) { throw 'Backup copy bytes differ.' }

$process = Start-Notebook $historyRestore
$window = $process.MainWindowHandle
Select-Note 'MetadataHistory'
Test-HistoryMetadata $historyRestoreDb backup
$preview = Open-Revision 1
$report = [PaperroomHighlightGui]::Read((Control $preview 101))
if (!$report.Contains('欄位 = 現值') -or !$report.Contains('Same.txt　3 位元組　#3')) {
    throw 'The displaced attachment and property did not survive full backup.'
}
Finish-Revision $preview 2
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-FileHash -LiteralPath $historyDb).Hash -cne $sourceStamp -or
    (Get-FileHash -LiteralPath $historyBackup).Hash -cne $backupStamp) { throw 'Isolated reopen changed source or backup.' }
Write-Host "PASS: native metadata revision preview, cancel, exact same-name attachment restore, close/reopen, isolated full backup, and displaced version. EXE SHA-256 $ExeSha256."
