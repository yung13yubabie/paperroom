# Run against the exact downloaded EXE in the same native acceptance job as the isolated Vault baseline.
function New-NativeReviewView([string]$Name, [int]$Operator, [string]$Label, [string[]]$Fields, [string]$Fold) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 7 '儲存分組與排序檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' $Name
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '分組' 'Bucket'
    Collection-Prompt '排序'
    Set-DisplayColumns '' ($Fields -join [char]10)
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
    Native-Menu 120 1 '新增篩選…' -MenuOpen
    Choose-CollectionRow 'Due　'
    Native-Menu 120 $Operator $Label -MenuOpen
    Native-Menu 120 5 '儲存變更' -MenuOpen
    Native-Menu 120 25 '群組收合…'
    Choose-CollectionRow "收合 「$Fold」"
}

function Open-JointShared([string]$Status) {
    Native-Menu 120 3 '開啟資料列…'
    Choose-CollectionRow "$Status　Shared  ("
    $null = Wait-For 'original Shared collection row opened' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'Shared' -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟集合資料列'
    }
}

function Export-JointClip([string]$Path, [string]$ExpectedHash) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Native-Menu 113 1 'Clip.txt  (4 B)' -MenuOpen
    Native-Menu 113 1 '儲存附件副本…' -MenuOpen
    Native-File 1 '儲存附件副本…' $Path '附件副本已儲存。' -AlreadyOpened -SaveName 'Clip.txt'
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -cne $ExpectedHash) {
        throw 'The attachment exported by the native UI has the wrong bytes.'
    }
}

function Read-JointReviewState([string]$Db, [string]$Phase, [string]$ExpectedDue) {
    $json = @'
from contextlib import closing
from hashlib import sha256
from pathlib import Path
from zipfile import ZipFile
import json, sqlite3, sys

db, archive, phase, due, expected_hash = sys.argv[1:]
project = 'Project 11111111111111111111111111111111'
catalog = 'Dates 33333333333333333333333333333333_all.csv'
with closing(sqlite3.connect(Path(db).as_uri()+'?mode=ro',uri=True)) as conn, ZipFile(archive) as z:
    assert conn.execute('PRAGMA user_version').fetchone() == (44,)
    assert conn.execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert not conn.execute('PRAGMA foreign_key_check').fetchall()
    source_assets = dict(conn.execute('SELECT name,content FROM attachments WHERE name IN (?,?)',(catalog,'source.txt')))
    assert source_assets == {name:z.read(project+'/'+name) for name in (catalog,'source.txt')}
    notes = conn.execute("SELECT id,version,trash_root FROM notes WHERE title='Shared'").fetchall()
    assert len(notes)==1 and notes[0][1:] == (1,None),notes
    shared = notes[0][0]
    fields = dict(conn.execute('SELECT key,value FROM note_properties WHERE note_id=?',(shared,)))
    assert fields == {'Name':'Shared','Bucket':'Hidden','Due':due,'Memo':'Old text'},fields
    contents = conn.execute('SELECT id,name,content FROM attachments WHERE note_id=?',(shared,)).fetchall()
    assert len(contents)==1 and contents[0][1]=='Clip.txt',contents
    current_id, _, raw = contents[0]
    assert sha256(raw).hexdigest().upper()==expected_hash
    members = conn.execute("""
        SELECT c.name,m.note_id,n.title,m.status FROM collection_notes m JOIN collections c ON c.id=m.collection_id
        JOIN notes n ON n.id=m.note_id ORDER BY c.name,n.title""").fetchall()
    assert len(members)==8 and [(name,id) for name,id,title,status in members if title=='Shared']==[
        ('A-Review',shared),('B-Review',shared)],members
    assert all(status==('未分類' if name=='A-Review' else '待處理') for name,_,_,status in members),members
    configs = conn.execute("""
        SELECT c.name,v.id,v.name,v.group_key,v.rules_json,d.columns_json
        FROM collections c JOIN collection_views v ON v.collection_id=c.id
        JOIN collection_display_columns d ON d.collection_id=c.id ORDER BY c.name""").fetchall()
    assert [(name,view,key,json.loads(rules),json.loads(cols)) for name,_,view,key,rules,cols in configs]==[
        ('A-Review','A-Timing','Bucket',[False,[['Due','date',23,'']],[]],['Due','Memo']),
        ('B-Review','B-Timing','Bucket',[False,[['Due','date',14,'']],[]],['Memo','Due']),
    ],configs
    folds = conn.execute("""
        SELECT c.name,v.id,f.grouping_key,f.grouping_kind,f.group_json FROM collection_group_folds f
        JOIN collection_views v ON f.view_id=v.id JOIN collections c ON v.collection_id=c.id ORDER BY c.name
    """).fetchall()
    assert [(name,key,kind,json.loads(value)) for name,_,key,kind,value in folds]==[
        ('A-Review','Bucket','text','Hidden'),('B-Review','Bucket','text','Side')
    ],folds
    if phase in ('restored','reopened','isolated'):
        saved = conn.execute("""SELECT r.id,a.original_id,a.content FROM note_revisions r
            JOIN note_revision_attachments a ON a.revision_id=r.id WHERE r.note_id=? ORDER BY r.id DESC LIMIT 1""",
            (shared,)).fetchone()
        assert saved and saved[1]!=current_id and saved[2]==b'SWAP',saved
    print(json.dumps(dict(phase=phase, note_id=shared, due=due, current_attachment_id=current_id,
        live_attachment_sha256=sha256(raw).hexdigest(),
        imported_content_sha256={name:sha256(content).hexdigest() for name,content in source_assets.items()},
        membership=members, views=configs, folded_groups=folds,
        view_state_sha256=sha256(repr((members,configs,folds)).encode('utf-8')).hexdigest())))
'@ | python - $Db $jointReviewZip $Phase $ExpectedDue $(if ($Phase -eq 'edited') { $replacementDigest } else { $originalDigest })
    if ($LASTEXITCODE -ne 0) { throw "Integrated SQLite readback failed at $Phase." }
    $state = $json | ConvertFrom-Json
    Write-Host "Verified integrated workspace $Phase, view state $($state.view_state_sha256), attachment $($state.live_attachment_sha256)."
    $state
}

$jointReviewHome = Join-Path $root 'joint-review'
$jointReviewCopy = Join-Path $root 'joint-review-copy'
$jointReviewDb = Join-Path $jointReviewHome 'paperroom-data\notes.sqlite3'
$jointReviewCopyDb = Join-Path $jointReviewCopy 'paperroom-data\notes.sqlite3'
$jointReviewBackup = Join-Path $root 'joint-review-backup.sqlite3'
$jointReviewZip = Join-Path $root 'joint-review.zip'
$catalogName = 'Dates 33333333333333333333333333333333_all.csv'
$today = (Get-Date).Date
$due = Iso-DueDate $today.AddDays(1)
$dayTwo = Iso-DueDate $today.AddDays(2)
$later = Iso-DueDate $today.AddDays(8)
@'
from zipfile import ZipFile
import sys

archive,catalog,due,second=sys.argv[1:]
project='Project 11111111111111111111111111111111'
child='Child 22222222222222222222222222222222'
csv = f'Name,Bucket,Due,Memo\nShared,Hidden,{due},Old text\nCover,Open,{due},Control A\n'
csv += f'Side,Side,{due},Control B\nWitness,Open,{second},Control C\n'
with ZipFile(archive,'x') as z:
    z.writestr(project+'.md','# Project\n\n[Open](Project%2011111111111111111111111111111111/Child%2022222222222222222222222222222222.md)\n')
    z.writestr(f'{project}/{child}.md','# Native asset page\n\n[Evidence](source.txt)\n')
    z.writestr(project+'/source.txt','Original Notion attachment.\n')
    z.writestr(f'{project}/{catalog}',csv)
'@ | python - $jointReviewZip $catalogName $due $dayTwo
if ($LASTEXITCODE -ne 0) { throw 'Could not create the real local ZIP fixture.' }
$zipDigest = (Get-FileHash -LiteralPath $jointReviewZip -Algorithm SHA256).Hash
foreach ($folder in $jointReviewHome,$jointReviewCopy) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
$clips = @(foreach ($name in 'ORIG','SWAP') {
    $folder = Join-Path $root ("joint-clip-"+$name)
    [void][IO.Directory]::CreateDirectory($folder)
    $file = Join-Path $folder 'Clip.txt'
    [IO.File]::WriteAllBytes($file,[Text.Encoding]::ASCII.GetBytes($name))
    $file
})
$originalDigest = (Get-FileHash -LiteralPath $clips[0] -Algorithm SHA256).Hash
$replacementDigest = (Get-FileHash -LiteralPath $clips[1] -Algorithm SHA256).Hash
$records = [ordered]@{}

$process = Start-Notebook $jointReviewHome
$window = $process.MainWindowHandle
$report = Native-File 2 '匯入 Notion Markdown ZIP…' $jointReviewZip '' -NextDialog '匯入報告'
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('Notion CSV 原檔 1 份')) {
    throw 'The import did not expose the original CSV in the asset-bearing archive.'
}
Finish-Revision $report 1
Select-Note "1｜$catalogName"
$prompt = Open-NotionCsvCollectionPrompt $catalogName
Type-SearchFilter $prompt 101 'A-Review'
Finish-Revision $prompt 1
$null = Wait-For 'native CSV conversion produced four editable rows' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已建立集合，匯入 4 列。原始 CSV 已保留。'
}
Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Collection-Prompt '設定工作區屬性型別' 'Due'
Native-Menu 109 3 '日期' -MenuOpen
Open-ColumnsCollection 1 'A-Review'
Open-JointShared '未分類'
Add-HistoryFile $clips[0]

Open-ColumnsCollection 1 'A-Review'
New-NativeReviewView 'A-Timing' 24 '未來七天（不含今天）' @('Due','Memo') 'Hidden'
Assert-GroupRows @('Cover','Witness','Side') -Status '未分類'
Set-UpColumnCollection 'B-Review' @('Shared','Cover','Side','Witness')
New-NativeReviewView 'B-Timing' 15 '明天' @('Memo','Due') 'Side'
Assert-GroupRows @('Shared','Cover')
$records.baseline = Read-JointReviewState $jointReviewDb 'baseline' $due

Open-JointShared '待處理'
Native-Menu 113 4 '設定文字屬性…'
Rule-Prompt '文字屬性' '' 'Due'
Rule-Prompt '設定文字屬性' $due $later
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 2 '管理附件…' -MenuOpen
Native-Menu 113 1 'Clip.txt  (4 B)' -MenuOpen
Native-Menu 113 2 '刪除附件…' -MenuOpen
$delete = Wait-For 'native attachment removal confirmation' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'刪除附件')
}
Finish-Revision $delete 1
Add-HistoryFile $clips[1]
Export-JointClip (Join-Path $root 'joint-export-edited.txt') $replacementDigest
$records.edited = Read-JointReviewState $jointReviewDb 'edited' $later
Open-ColumnsCollection 2 'B-Review'
Open-SortNamedView 1 'B-Timing'
Assert-GroupRows @('Cover')

Open-ColumnsCollection 2 'B-Review'
Open-JointShared '待處理'
$revision = Open-Revision 3
$preview = [PaperroomHighlightGui]::Read((Control $revision 101))
if (!$preview.Contains("Due = $due") -or !$preview.Contains("Clip.txt　4 位元組　#$($records.baseline.current_attachment_id)")) {
    throw 'The actual revision preview lacked the original date and attachment identity.'
}
Finish-Revision $revision 1
$null = Wait-For 'native shared row metadata was restored' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已還原選取的筆記版本。'
}
Export-JointClip (Join-Path $root 'joint-export-restored.txt') $originalDigest
$records.restored = Read-JointReviewState $jointReviewDb 'restored' $due
Open-ColumnsCollection 1 'A-Review'
Open-SortNamedView 1 'A-Timing'
Assert-GroupRows @('Cover','Witness','Side') -Status '未分類'
Open-ColumnsCollection 2 'B-Review'
Open-SortNamedView 1 'B-Timing'
Assert-GroupRows @('Shared','Cover')

Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $jointReviewHome
$window = $process.MainWindowHandle
$records.reopened = Read-JointReviewState $jointReviewDb 'reopened' $due
Open-ColumnsCollection 1 'A-Review'
Open-SortNamedView 1 'A-Timing'
Assert-GroupRows @('Cover','Witness','Side') -Status '未分類'
Open-ColumnsCollection 2 'B-Review'
Open-SortNamedView 1 'B-Timing'
Assert-GroupRows @('Shared','Cover')
Native-File 4 '備份工作區…' $jointReviewBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$sourceStamp = (Get-FileHash -LiteralPath $jointReviewDb -Algorithm SHA256).Hash
$backupStamp = (Get-FileHash -LiteralPath $jointReviewBackup -Algorithm SHA256).Hash
Copy-Item -LiteralPath $jointReviewBackup -Destination $jointReviewCopyDb
if ((Get-FileHash -LiteralPath $jointReviewCopyDb -Algorithm SHA256).Hash -cne $backupStamp) { throw 'Backup copy bytes differ.' }

$process = Start-Notebook $jointReviewCopy
$window = $process.MainWindowHandle
$records.isolated = Read-JointReviewState $jointReviewCopyDb 'isolated' $due
Open-ColumnsCollection 1 'A-Review'
Open-SortNamedView 1 'A-Timing'
Assert-GroupRows @('Cover','Witness','Side') -Status '未分類'
Native-Menu 120 25 '群組收合…'
Choose-CollectionRow '展開 「Hidden」'
Assert-GroupRows @('Shared','Cover','Witness','Side') -Status '未分類'
Native-Menu 120 24 '表格與清單欄位…'
Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
Choose-CollectionRow 'Hidden　Shared  ('
$report = Wait-For 'expanded row actual field display' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
}
$text = [PaperroomHighlightGui]::Read((Control $report 101))
$datePosition = $text.IndexOf("Due　$due",[StringComparison]::Ordinal)
if ($datePosition -lt 0 -or $text.IndexOf('Memo　Old text',[StringComparison]::Ordinal) -le $datePosition) {
    throw 'The expanded row did not show the original editable values in the saved display order.'
}
Finish-Revision $report 1
Native-Menu 120 25 '群組收合…'
Choose-CollectionRow '收合 「Hidden」'
Open-ColumnsCollection 2 'B-Review'
Open-SortNamedView 1 'B-Timing'
Assert-GroupRows @('Shared','Cover')
Open-JointShared '待處理'
Export-JointClip (Join-Path $root 'joint-export-isolated.txt') $originalDigest
$records.refolded = Read-JointReviewState $jointReviewCopyDb 'isolated' $due
Close-Notebook $process
$process.Dispose()
$process = $null

foreach ($entry in $records.Values) {
    if ($entry.view_state_sha256 -cne $records.baseline.view_state_sha256) { throw 'A view or membership changed unexpectedly.' }
}
if ((Get-Date).Date -ne $today -or (Get-FileHash -LiteralPath $jointReviewDb).Hash -cne $sourceStamp -or
    (Get-FileHash -LiteralPath $jointReviewBackup).Hash -cne $backupStamp -or
    (Get-FileHash -LiteralPath $jointReviewZip).Hash -cne $zipDigest) {
    throw 'Calendar day, source workspace, backup or original ZIP changed unexpectedly.'
}
if ($env:PAPERROOM_GROUP_HISTORY_RECEIPT) {
    [ordered]@{
        platform='GitHub-hosted Windows Server native UI'
        windows_10_11_offline='not run'
        commit=$env:GITHUB_SHA
        run_id=$env:GITHUB_RUN_ID
        exe_sha256=$ExeSha256
        source_zip_sha256=$zipDigest
        native_workspace_backup_sha256=$backupStamp
        stages=$records
        expanded_group_readback=@('Shared','Cover','Witness','Side')
    } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_GROUP_HISTORY_RECEIPT -Encoding utf8
}
Write-Host "PASS: same Notion workspace and CSV note retain both saved views, relative dates and independent folded groups through exact attachment history restore, restart and isolated backup. Expanded rows and native exported attachment bytes verified. EXE SHA-256 $ExeSha256."
