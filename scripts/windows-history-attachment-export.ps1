# Verify saved historical bytes for the exact downloaded EXE. Only real application UI writes SQLite.
$archiveHome = Join-Path $root 'history-attachment-live'
$archiveRestore = Join-Path $root 'history-attachment-restored'
$archiveDb = Join-Path $archiveHome 'paperroom-data/notes.sqlite3'
$archiveRestoreDb = Join-Path $archiveRestore 'paperroom-data/notes.sqlite3'
$archiveBackup = Join-Path $root 'history-attachment-backup.sqlite3'
$archiveOutput = Join-Path $root 'history-attachment-exports'
$archiveSource = Join-Path $root 'HistoryAttachmentFixture.md'
$archiveTitle = 'HistoryAttachmentFixture'
foreach ($folder in $archiveHome,$archiveRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
[void][IO.Directory]::CreateDirectory($archiveOutput)
[IO.File]::WriteAllText($archiveSource, 'Original history attachment body.' + [char]10)
$archiveInputs = foreach ($item in 'archive-A','archive-B','live-only') {
    $folder = Join-Path $root $item
    [void][IO.Directory]::CreateDirectory($folder)
    $file = Join-Path $folder 'Same.txt'
    [IO.File]::WriteAllText($file, $item + [char]10, [Text.Encoding]::ASCII)
    $file
}

function Set-ArchiveHistoryProperty([string]$Previous,[string]$Value) {
    Native-Menu 113 4 '設定文字屬性…'
    Rule-Prompt '文字屬性' '' 'HistoryState'
    Rule-Prompt '設定文字屬性' $Previous $Value
}
function Add-ArchiveHistoryFile([string]$Path) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 1 '加入本機附件…' -MenuOpen
    Native-File 1 '加入本機附件…' $Path '附件已保存到本機工作區。' -AlreadyOpened
}
function Remove-FirstLiveHistoryFile([int]$ExpectedCount) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    $labels = @(Collection-MenuLabels)
    if ($labels.Count -ne $ExpectedCount -or @($labels | Where-Object {$_ -cne 'Same.txt  (10 B)'}).Count) {
        throw "Unexpected current attachment choices: $($labels -join ' | ')"
    }
    # Both old files are intentionally removed. The existing live list orders by attachment ID.
    Native-Menu 113 1 'Same.txt  (10 B)' -MenuOpen
    Native-Menu 113 2 '刪除附件…' -MenuOpen
    $confirm = Wait-For 'native deletion confirmation for an old file' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'刪除附件')
    }
    Finish-Revision $confirm 1
}
function Read-ArchiveHistoryWitness([string]$Database) {
    $json = @'
from contextlib import closing
from pathlib import Path
import hashlib, json, sqlite3, sys

db_path, source_a, source_b, source_live = sys.argv[1:]
a, b, live = [Path(p).read_bytes() for p in (source_a, source_b, source_live)]
assert a != b and live not in (a,b) and len(a)==len(b)==len(live)==10
digest = lambda raw: hashlib.sha256(raw).hexdigest()
with closing(sqlite3.connect(Path(db_path).resolve(strict=True).as_uri()+"?mode=ro", uri=True)) as db:
    db.execute("PRAGMA query_only=ON")
    db.execute("BEGIN")
    assert db.execute("PRAGMA integrity_check").fetchall()==[("ok",)]
    assert not db.execute("PRAGMA foreign_key_check").fetchall()
    note, = db.execute("""SELECT n.id, i.token, n.title, n.body, n.version, n.pinned
        FROM notes n JOIN note_link_identities i ON i.note_id=n.id
        WHERE n.title='HistoryAttachmentFixture' AND n.trash_root IS NULL""").fetchall()
    identity, token, _, body, _, _ = note
    assert token and body=="Current body retained.", note
    prop = db.execute("SELECT key,value FROM note_properties WHERE note_id=?", (identity,)).fetchall()
    assert prop==[("HistoryState","current")], prop
    current, = db.execute("SELECT id,name,kind,content FROM attachments WHERE note_id=?", (identity,)).fetchall()
    assert current[1:]==("Same.txt","text",live), current[:3]
    revisions = db.execute("""SELECT r.id,r.prior_version,r.title,r.body,s.properties_json
        FROM note_revisions r JOIN note_revision_state s ON s.revision_id=r.id
        WHERE r.note_id=? ORDER BY r.id""",(identity,)).fetchall()
    history = {}
    for rev in revisions:
        rows = db.execute("""SELECT original_id,name,kind,content,origin FROM note_revision_attachments
            WHERE revision_id=? ORDER BY original_id""", (rev[0],)).fetchall()
        history[rev[0]] = rows
    old = [r[0] for r in revisions if r[3]=="Original history attachment body.\n"
           and json.loads(r[4])=={"HistoryState":"older"}
           and [asset[3] for asset in history[r[0]]]==[a,b]]
    assert len(old)==1, old
    revision = old[0]
    selected = history[revision]
    assert all(row[1:3]==("Same.txt","text") and row[4] is None for row in selected)
    ids = [row[0] for row in selected]
    assert len(set(ids+[current[0]]))==3
    empty = [r[0] for r in revisions if r[3]=="Original history attachment body.\n" and not history[r[0]]]
    assert empty
    snapshots=[(rev,[(item[0],item[1],item[2],digest(item[3]),item[4])
                  for item in rows]) for rev,rows in history.items()]
    print(json.dumps(dict(note=note, properties=prop, live=[current[0],digest(live)],
        archive_revision=revision, no_attachment_revision=empty[0],
        archived=[[row[0],digest(row[3])] for row in selected], revisions=revisions, snapshots=snapshots),
        sort_keys=True,ensure_ascii=True))
'@ | python -B - $Database $archiveInputs[0] $archiveInputs[1] $archiveInputs[2]
    if ($LASTEXITCODE -ne 0 -or !$json) { throw 'Read-only history attachment verification failed.' }
    return [string]$json
}


function Choose-HistoryExportRevision([long]$Revision) {
    Native-Menu 113 28 '另存歷史版本附件…'
    $suffix = "#$Revision"
    $labels = @(Collection-MenuLabels)
    $matches = @(for ($i=0; $i -lt $labels.Count; $i++) {
        if ($labels[$i].StartsWith('版本 ',[StringComparison]::Ordinal) -and
            $labels[$i].EndsWith($suffix,[StringComparison]::Ordinal)) { $i }
    })
    if ($matches.Count -ne 1) { throw "Did not find exactly one visible history version $suffix." }
    Native-Menu 113 ($matches[0]+1) $labels[$matches[0]] -MenuOpen
}
function Open-ArchiveFileSaveAs([long]$Revision,[long]$Asset) {
    Choose-HistoryExportRevision $Revision
    $labels = @(Collection-MenuLabels)
    $expected = @($archiveIdentity.archived | ForEach-Object { "[#$($_[0])]　Same.txt　10 B" })
    if (($labels -join '|') -cne ($expected -join '|')) {
        throw "Archive selector labels do not distinguish the two original IDs: $($labels -join ' | ')"
    }
    $label = "[#$Asset]　Same.txt　10 B"
    Choose-CollectionRow $label
}
function Save-HistoryArchiveFile([long]$Revision,$Asset,[string]$OutputName) {
    Open-ArchiveFileSaveAs $Revision $Asset[0]
    $output = Join-Path $archiveOutput $OutputName
    Native-File 1 '另存歷史版本附件…' $output '已另存歷史版本附件。' -AlreadyOpened -SaveName 'Same.txt'
    $savedSha = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash
    if ($savedSha -ine $Asset[1] -or $savedSha -ieq $archiveIdentity.live[1]) {
        throw "Saved bytes for archived attachment #$($Asset[0]) do not match the chosen history entry."
    }
    Write-Host "Verified saved bytes for historical revision #$Revision, original attachment #$($Asset[0]): $savedSha."
}

$process = Start-Notebook $archiveHome
$window = $process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $archiveSource '已匯入 1 篇 Markdown'
Select-Note $archiveTitle
Set-ArchiveHistoryProperty '' 'older'
Add-ArchiveHistoryFile $archiveInputs[0]
Add-ArchiveHistoryFile $archiveInputs[1]
Remove-FirstLiveHistoryFile 2
Remove-FirstLiveHistoryFile 1
Add-ArchiveHistoryFile $archiveInputs[2]
Set-ArchiveHistoryProperty 'older' 'current'
$field = Open-Editor
Replace-EditorText $field 'Current body retained.'
Command $window 110
$null = Wait-For 'saved distinct current note body' {
    [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('Current body retained.')
}
$archiveBefore = Read-ArchiveHistoryWitness $archiveDb
$archiveIdentity = $archiveBefore | ConvertFrom-Json

Choose-HistoryExportRevision $archiveIdentity.no_attachment_revision
$null = Wait-For 'historical revision with zero archived attachments refused' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這個版本沒有保存附件。'
}
Open-ArchiveFileSaveAs $archiveIdentity.archive_revision $archiveIdentity.archived[0][0]
$cancel = Wait-For 'native Save As with original historical filename' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$null,2)
}
$null = Wait-For 'historical filename displayed before cancellation' {
    [PaperroomHighlightGui]::FilenameField($cancel,$process.Id,'Same.txt')
}
Finish-Revision $cancel 2
if (@(Get-ChildItem -LiteralPath $archiveOutput -Force).Count -or
    (Read-ArchiveHistoryWitness $archiveDb) -cne $archiveBefore) {
    throw 'Cancel or a revision with no attachment wrote output or changed the workspace.'
}
Save-HistoryArchiveFile $archiveIdentity.archive_revision $archiveIdentity.archived[0] 'original-A.txt'
Save-HistoryArchiveFile $archiveIdentity.archive_revision $archiveIdentity.archived[1] 'original-B.txt'
if ((Read-ArchiveHistoryWitness $archiveDb) -cne $archiveBefore) { throw 'Archive export changed the live workspace.' }

Native-File 4 '備份工作區…' $archiveBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$archiveSourceHash = (Get-FileHash -LiteralPath $archiveDb -Algorithm SHA256).Hash
$archiveBackupHash = (Get-FileHash -LiteralPath $archiveBackup -Algorithm SHA256).Hash
Copy-Item -LiteralPath $archiveBackup -Destination $archiveRestoreDb
if ((Get-FileHash -LiteralPath $archiveRestoreDb -Algorithm SHA256).Hash -cne $archiveBackupHash) {
    throw 'The isolated copy differs from the backup.'
}
$cases = @(
    @{ Home=$archiveHome; Db=$archiveDb; Asset=$archiveIdentity.archived[0]; Name='reopened-A.txt' },
    @{ Home=$archiveRestore; Db=$archiveRestoreDb; Asset=$archiveIdentity.archived[1]; Name='isolated-B.txt' }
)
foreach ($case in $cases) {
    $process = Start-Notebook $case.Home
    $window = $process.MainWindowHandle
    Select-Note $archiveTitle
    $null = Wait-For 'original current body visible after reopening' {
        [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('Current body retained.')
    }
    Save-HistoryArchiveFile $archiveIdentity.archive_revision $case.Asset $case.Name
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    if ((Read-ArchiveHistoryWitness $case.Db) -cne $archiveBefore) {
        throw 'Reopened or isolated export changed note, live attachment, or saved history.'
    }
}
if ((Get-FileHash -LiteralPath $archiveDb -Algorithm SHA256).Hash -cne $archiveSourceHash -or
    (Get-FileHash -LiteralPath $archiveBackup -Algorithm SHA256).Hash -cne $archiveBackupHash) {
    throw 'Reopening changed the source workspace or backup file.'
}
$exportHashes = [ordered]@{}
Get-ChildItem -LiteralPath $archiveOutput -File | Sort-Object Name | ForEach-Object {
    $exportHashes[$_.Name] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
if ($exportHashes.Count -ne 4) { throw 'Expected four checked Save As outputs.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    backup_sha256=$archiveBackupHash
    revision=$archiveIdentity.archive_revision
    empty_revision=$archiveIdentity.no_attachment_revision
    archived_attachments=$archiveIdentity.archived
    current_attachment=$archiveIdentity.live
    note_id=$archiveIdentity.note[0]
    saved_sha256=$exportHashes
    scenarios=@('exact revision and original identity for same-name files of the same size',
        'zero archived attachments and Save As cancellation without output',
        'exact historical bytes from both original IDs with current note and live attachment untouched',
        'same saved history after reopening original and isolated workspace backup')
} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: exact historical attachment Save As, cancellation, same names, archived bytes after reopen and isolated backup. EXE SHA-256 $ExeSha256."
