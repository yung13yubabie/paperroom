# Real Windows restore of this candidate's own backup into a previously nonexistent portable workspace.
$restoreSourceHome = Join-Path $root 'original-workspace'
$restoreSourceDb = Join-Path $restoreSourceHome 'paperroom-data/notes.sqlite3'
$restoreParent = Join-Path $root 'new-workspaces'
$restoreChildName = 'Restored portable'
$restoreHome = Join-Path $restoreParent $restoreChildName
$restoreExe = Join-Path $restoreHome 'Paperroom.exe'
$restoreDb = Join-Path $restoreHome 'paperroom-data/notes.sqlite3'
$restoreBackup = Join-Path $root 'original-backup.sqlite3'
$restoreBadBackup = Join-Path $root 'invalid-backup.sqlite3'
$restoreOccupied = Join-Path $restoreParent 'Occupied'
$restoreSentinel = Join-Path $restoreOccupied 'keep.txt'
$restoreNoteTitle = 'Restore source'
$restorePeerTitle = 'Restore peer'
$restoreCollection = 'Restore collection'
$restoreMd = Join-Path $root 'Restore source.md'
$restorePeerMd = Join-Path $root 'Restore peer.md'
$restoreAsset = Join-Path $root 'RestoreAsset.txt'
[void][IO.Directory]::CreateDirectory((Join-Path $restoreSourceHome 'paperroom-data'))
[void][IO.Directory]::CreateDirectory($restoreOccupied)
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restoreSourceHome 'Paperroom.exe')
[IO.File]::WriteAllText($restoreMd, 'Original saved source before revision.' + [char]10)
[IO.File]::WriteAllText($restorePeerMd, 'Exact peer reached after portable restore.' + [char]10)
[IO.File]::WriteAllText($restoreAsset, 'Synthetic restore attachment bytes.' + [char]10)
[IO.File]::WriteAllText($restoreSentinel, 'Existing destination content must survive.')
[IO.File]::WriteAllText($restoreBadBackup, 'Synthetic non-database file.')
$restoreAssetSha = (Get-FileHash -LiteralPath $restoreAsset -Algorithm SHA256).Hash
$restoreSentinelSha = (Get-FileHash -LiteralPath $restoreSentinel -Algorithm SHA256).Hash
$restoreBadBackupSha = (Get-FileHash -LiteralPath $restoreBadBackup -Algorithm SHA256).Hash

function Read-WorkspaceRestoreWitness([string]$Database,[string]$Expected) {
    $state = @'
from contextlib import closing
from pathlib import Path
import hashlib, json, sqlite3, sys

p, attachment_path, expected = sys.argv[1:]
initial_body = "Original saved source before revision.\n"
before_copy = "Original saved source at backup."
after_copy = "New text in restored copy only."
db_path = Path(p).resolve(strict=True)
standalone = db_path.name == "original-backup.sqlite3"
uri = db_path.as_uri()+"?mode=ro"+("&immutable=1" if standalone else "")
with closing(sqlite3.connect(uri, uri=True)) as db:
    db.execute("PRAGMA query_only=ON")
    db.execute("BEGIN")
    assert db.execute("PRAGMA integrity_check").fetchall()==[("ok",)]
    assert not db.execute("PRAGMA foreign_key_check").fetchall()
    def row(title):
        found, = db.execute("""SELECT n.id,i.token,n.title,n.body,n.version FROM notes n
            JOIN note_link_identities i ON i.note_id=n.id WHERE n.title=? AND n.trash_root IS NULL""",
            (title,)).fetchall()
        return found
    source, peer = row("Restore source"), row("Restore peer")
    assert source[1] and peer[1] and source[:2]!=peer[:2]
    assert source[3]==(after_copy if expected=="edited" else before_copy), source
    assert peer[3]=="Exact peer reached after portable restore.\n", peer
    files = db.execute("SELECT id,name,kind,content FROM attachments WHERE note_id=?",(source[0],)).fetchall()
    asset, = files
    assert asset[1:3]==("RestoreAsset.txt","text") and asset[3]==Path(attachment_path).read_bytes()
    properties = db.execute("SELECT key,value FROM note_properties WHERE note_id=?",(source[0],)).fetchall()
    assert properties==[("Stage","review")], properties
    links = db.execute("SELECT source_id,name,target_id FROM note_relations WHERE source_id=?",(source[0],)).fetchall()
    assert links==[(source[0],"Supporting",peer[0])], links
    collection, = db.execute("SELECT id,name FROM collections WHERE name='Restore collection'").fetchall()
    memberships = db.execute("SELECT note_id,status FROM collection_notes WHERE collection_id=? ORDER BY note_id",
        (collection[0],)).fetchall()
    assert [who for who,_ in memberships]==sorted([source[0],peer[0]]) and all(status for _,status in memberships)
    history = db.execute("""SELECT r.id,r.prior_version,r.body,s.properties_json FROM note_revisions r
        JOIN note_revision_state s ON s.revision_id=r.id WHERE r.note_id=? ORDER BY r.id""",(source[0],)).fetchall()
    history_files = [(h[0],db.execute("""SELECT original_id,name,kind,hex(content) FROM note_revision_attachments
        WHERE revision_id=? ORDER BY original_id""",(h[0],)).fetchall()) for h in history]
    assert any(h[2]==initial_body for h in history)
    if expected=="edited":
        assert history[-1][2]==before_copy and json.loads(history[-1][3])=={"Stage":"review"}
        assert history_files[-1][1]==[(asset[0],asset[1],asset[2],asset[3].hex().upper())]
    static = dict(source=source[:3],peer=peer,properties=properties,relations=links,
        membership=[collection,memberships],attachment=[*asset[:3],hashlib.sha256(asset[3]).hexdigest()])
    print(json.dumps(dict(static=static,source=source,history=history,history_files=history_files),
        sort_keys=True,ensure_ascii=True))
'@ | python -B - $Database $restoreAsset $Expected
    if ($LASTEXITCODE -ne 0 -or !$state) { throw "Read-only portable workspace witness failed ($Expected)." }
    return [string]$state
}
function Enter-RestoreBackupFile([string]$Path) {
    Native-Menu 109 43 '從備份另開可攜工作區…'
    $picker = Wait-For 'native backup input chooser' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$null,1)
    }
    $field = Wait-For 'backup input filename field' {
        $edit = [PaperroomHighlightGui]::FilenameField($picker,$process.Id,'')
        if ($edit -eq [IntPtr]::Zero) { return $null }
        try {
            $point = [PaperroomHighlightGui]::FilenamePoint($picker,$process.Id,$edit)
            [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
        } catch {
            if ($_.Exception.Message -match 'owned, visible file name edit|edit is not exposed|point is no longer|pointer target changed') { return $null }
            throw
        }
        if ([PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$picker,$edit)) { $edit }
    }
    [PaperroomHighlightGui]::TypeFilename($window,$process.Id,$picker,$field,$Path)
    $null = Wait-For 'actual backup file path entered' { Test-NativeEditValue $field $Path }
    Finish-Revision $picker 1
}
function Open-RestoreConfirmation([string]$Backup,[string]$ChildName) {
    Enter-RestoreBackupFile $Backup
    Pick-NativeVaultFolder $restoreParent
    Rule-Prompt '新可攜工作區' 'Paperroom-還原' $ChildName
    Wait-For 'native restore confirmation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認另開備份',7)
    }
}
function Wait-ForRestoreCollectionView {
    $null = Wait-For 'native restore fixture collection switched to the intended board' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($restoreCollection) -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' -and
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '返回筆記'
    }
}
function Confirm-RestoreRefusal([string]$Text) {
    $errorWindow = Wait-For 'native workspace restore refusal' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
    }
    if (![PaperroomHighlightGui]::DialogContains($errorWindow,$process.Id,$Text)) {
        throw "Missing expected portable restore error: $Text"
    }
    # A native MessageBox does not always expose its default OK as child 1.
    # Match the exact owned foreground alert before sending its real Enter key.
    if ([PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間') -ne $errorWindow) {
        throw 'The validated restore refusal is no longer in the foreground.'
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null = Wait-For 'native restore refusal dismissed' { ![PaperroomHighlightGui]::IsWindowVisible($errorWindow) }
    Write-Host "Verified and dismissed the native restore refusal: $Text."
}
function Get-LiveWorkspaceFileHash([string]$Path) {
    # SQLite's active Windows handle requires sharing reads and writes. This
    # stream has read-only access and comparisons happen between UI actions.
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $stream = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,$share)
    try {
        $digest = [Security.Cryptography.SHA256]::Create()
        try { [Convert]::ToHexString($digest.ComputeHash($stream)) }
        finally { $digest.Dispose() }
    } finally { $stream.Dispose() }
}
function Confirm-RestoreSource([string]$Witness,[string]$DbHash,[string]$BackupHash) {
    if ((Read-WorkspaceRestoreWitness $restoreSourceDb 'baseline') -cne $Witness -or
        (Get-LiveWorkspaceFileHash $restoreSourceDb) -cne $DbHash -or
        (Get-FileHash -LiteralPath $restoreBackup -Algorithm SHA256).Hash -cne $BackupHash) {
        throw 'The live original workspace or its backup changed during portable restore.'
    }
}
function Find-LaunchedPortableWorkspace {
    $running = @(Get-Process -Name Paperroom -ErrorAction SilentlyContinue | Where-Object {
        $_.Id -ne $sourceProcess.Id -and ($_.Path -ieq $restoreExe -or $_.Path -ieq "\\?\$restoreExe")
    })
    if ($running.Count -gt 1) { throw 'More than one restored portable EXE started.' }
    if ($running.Count) {
        $candidate = $running[0]
        $handle = $candidate.MainWindowHandle
        [uint32]$owner = 0
        if ([PaperroomHighlightGui]::GetWindowThreadProcessId($handle,[ref]$owner) -and
            $owner -eq $candidate.Id -and [PaperroomHighlightGui]::GetDlgItem($handle,103) -ne [IntPtr]::Zero) {
            $candidate
        }
    }
}

$process = Start-Notebook $restoreSourceHome
$window = $process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $restoreMd '已匯入 1 篇 Markdown'
Native-File 1 '匯入 Markdown…' $restorePeerMd '已匯入 1 篇 Markdown'
Select-Note $restoreNoteTitle
Native-Menu 113 4 '設定文字屬性…'
Rule-Prompt '文字屬性' '' 'Stage'
Rule-Prompt '設定文字屬性' '' 'review'
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 1 '加入本機附件…' -MenuOpen
Native-File 1 '加入本機附件…' $restoreAsset '附件已保存到本機工作區。' -AlreadyOpened
Native-Menu 113 7 '筆記關聯…'
Native-Menu 113 1 '新增關聯…' -MenuOpen
Choose-CollectionRow "$restorePeerTitle  ("
Collection-Prompt '新增關聯' 'Supporting'
$field = Open-Editor
Replace-EditorText $field 'Original saved source at backup.'
Command $window 110
$null = Wait-For 'restorable source version saved' {
    [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('Original saved source at backup.')
}
Native-Menu 109 10 '資料集合與看板…'
Native-Menu 109 2 '新增集合…' -MenuOpen
Collection-Prompt '新增集合' $restoreCollection
Wait-ForRestoreCollectionView
foreach ($title in $restoreNoteTitle,$restorePeerTitle) {
    Native-Menu 120 1 '加入筆記…'
    Choose-CollectionRow "$title  ("
    Collection-Prompt '加入集合'
    Wait-ForRestoreCollectionView
}
Command $window 119
$null = Wait-For 'source note view after creating the collection' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
}
Native-File 4 '備份工作區…' $restoreBackup '工作區備份已完成'
$sourceWitness = Read-WorkspaceRestoreWitness $restoreSourceDb 'baseline'
if ((Read-WorkspaceRestoreWitness $restoreBackup 'baseline') -cne $sourceWitness) { throw 'The source backup differs logically from the live workspace.' }
$sourceHash = (Get-LiveWorkspaceFileHash $restoreSourceDb)
$backupHash = (Get-FileHash -LiteralPath $restoreBackup -Algorithm SHA256).Hash

$sourceProcess = $process
$sourceWindow = $window
$autoChild = $null
try {
    $confirm = Open-RestoreConfirmation $restoreBackup 'Cancelled restore'
    Finish-Revision $confirm 7
    if (Test-Path -LiteralPath (Join-Path $restoreParent 'Cancelled restore')) {
        throw 'Cancelling created a portable workspace.'
    }
    Write-Host 'Verified cancelling the native confirmation leaves the child path absent.'
    $confirm = Open-RestoreConfirmation $restoreBackup 'Occupied'
    Finish-Revision $confirm 6
    Confirm-RestoreRefusal '目的地已存在'
    if (@(Get-ChildItem -LiteralPath $restoreOccupied -Force).Count -ne 1 -or
        (Get-FileHash -LiteralPath $restoreSentinel -Algorithm SHA256).Hash -cne $restoreSentinelSha) {
        throw 'Refusing an occupied destination changed its existing content.'
    }
    $confirm = Open-RestoreConfirmation $restoreBadBackup 'Invalid restore'
    Finish-Revision $confirm 6
    Confirm-RestoreRefusal '紙間備份'
    if (Test-Path -LiteralPath (Join-Path $restoreParent 'Invalid restore')) {
        throw 'The invalid backup left a partial workspace.'
    }
    if ((Get-FileHash -LiteralPath $restoreBadBackup -Algorithm SHA256).Hash -cne $restoreBadBackupSha -or
        @(Get-ChildItem -LiteralPath $restoreParent -Force).Count -ne 1) {
        throw 'The invalid backup or restore cancellation left unexpected file changes.'
    }
    Confirm-RestoreSource $sourceWitness $sourceHash $backupHash
    Write-Host 'Verified native cancellation, occupied target and corrupt backup refusal without altering source bytes.'

    $confirm = Open-RestoreConfirmation $restoreBackup $restoreChildName
    Finish-Revision $confirm 6
    $null = Wait-For 'parent confirms new portable workspace' {
        [PaperroomHighlightGui]::Read((Control $sourceWindow 116)) -ceq '已從備份另開可攜工作區。'
    }
    if (!(Test-Path -LiteralPath $restoreExe -PathType Leaf) -or !(Test-Path -LiteralPath $restoreDb -PathType Leaf)) {
        throw 'Portable workspace is missing the generated executable or database.'
    }
    Assert-Hash $restoreExe
    $autoChild = Wait-For 'copy of exact EXE actually launched in its generated workspace' { Find-LaunchedPortableWorkspace }
    $childPid = $autoChild.Id
    Write-Host "Verified the application's automatic portable launch (PID $childPid)."
    $process = $autoChild
    $window = $process.MainWindowHandle
    $null = Wait-For 'restored child has presented its portable workspace indicator' {
        [PaperroomHighlightGui]::Read((Control $window 118)).Contains('可攜工作區')
    }
    if ((Read-WorkspaceRestoreWitness $restoreDb 'baseline') -cne $sourceWitness) {
        throw 'Newly created portable workspace does not match the backed-up notes, assets and identities.'
    }
    Confirm-RestoreSource $sourceWitness $sourceHash $backupHash

    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Choose-CollectionRow $restoreCollection
    Wait-ForRestoreCollectionView
    Native-Menu 120 3 '開啟資料列…'
    Choose-CollectionRow "待處理　$restoreNoteTitle  ("
    $null = Wait-For 'restored collection opens the backed-up source note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $restoreNoteTitle -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('Original saved source at backup.')
    }
    Native-Menu 113 7 '筆記關聯…'
    Native-Menu 113 2 '開啟關聯筆記…' -MenuOpen
    Choose-CollectionRow "Supporting　$restorePeerTitle  ("
    $null = Wait-For 'restored note opens its exact related peer' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $restorePeerTitle -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('Exact peer reached after portable restore.')
    }
    Select-Note $restoreNoteTitle
    $field = Open-Editor
    Replace-EditorText $field 'New text in restored copy only.'
    Command $window 110
    $null = Wait-For 'isolated restored copy accepted a real note edit' {
        [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('New text in restored copy only.')
    }
    $copyWitness = Read-WorkspaceRestoreWitness $restoreDb 'edited'
    $baseIdentity = ($sourceWitness | ConvertFrom-Json).static | ConvertTo-Json -Depth 10 -Compress
    $copyIdentity = ($copyWitness | ConvertFrom-Json).static | ConvertTo-Json -Depth 10 -Compress
    if ($baseIdentity -cne $copyIdentity) { throw 'Editing the restored copy changed attachment, relation or collection identity.' }
    Confirm-RestoreSource $sourceWitness $sourceHash $backupHash
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $autoChild = $null

    $process = Start-Notebook $restoreHome
    $window = $process.MainWindowHandle
    Select-Note $restoreNoteTitle
    $null = Wait-For 'real reopened portable copy retains its own edit' {
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('New text in restored copy only.')
    }
    if ((Read-WorkspaceRestoreWitness $restoreDb 'edited') -cne $copyWitness) {
        throw 'The isolated edit or its revision did not persist after reopening.'
    }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Confirm-RestoreSource $sourceWitness $sourceHash $backupHash
    if ((Get-FileHash -LiteralPath $restoreAsset -Algorithm SHA256).Hash -cne $restoreAssetSha -or
        @(Get-ChildItem -LiteralPath $restoreParent -Force).Count -ne 2) {
        throw 'The restore changed its source file or left unintended workspaces.'
    }
    $process = $sourceProcess
    $window = $sourceWindow
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $sourceProcess = $null
    if ((Read-WorkspaceRestoreWitness $restoreSourceDb 'baseline') -cne $sourceWitness) {
        throw 'Original workspace changed after the restored copy was edited.'
    }
    @{
        exe_sha256=$ExeSha256
        os=(Get-CimInstance Win32_OperatingSystem).Caption
        backup_sha256=$backupHash
        restored_process=$childPid
        restored_exe_sha256=(Get-FileHash -LiteralPath $restoreExe -Algorithm SHA256).Hash.ToLowerInvariant()
        original_db_sha256_while_open=$sourceHash
        identity=($copyWitness | ConvertFrom-Json).static
        scenarios=@('native cancellation and occupied or malformed backup refusal without writes',
            'actual File menu creates and launches verified portable EXE and standalone database',
            'real collection, note relationship, attachment bytes and history preserved',
            'restored copy edit persists after reopening while original and backup stay unchanged')
    } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
    Write-Host "PASS: native portable restore, occupied and corrupt rejection, actual copied EXE launch, independent persisted edit. EXE SHA-256 $ExeSha256."
} finally {
    # Driver cleanup is restricted to processes launched from these isolated fixture paths.
    foreach ($candidate in $autoChild,$sourceProcess) {
        if ($null -ne $candidate -and !$candidate.HasExited) {
            [void]$candidate.CloseMainWindow()
            [void]$candidate.WaitForExit(4000)
        }
    }
}
