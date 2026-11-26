# Native PDF history test for the exact downloaded EXE. Only Paperroom writes SQLite.
$pdfHome = Join-Path $root 'pdf-history-live'
$pdfRestore = Join-Path $root 'pdf-history-restored'
$pdfDb = Join-Path $pdfHome 'paperroom-data/notes.sqlite3'
$pdfRestoreDb = Join-Path $pdfRestore 'paperroom-data/notes.sqlite3'
$pdfBackup = Join-Path $root 'pdf-history-backup.sqlite3'
$pdfSource = Join-Path $root 'PdfHistoryOriginal.md'
$pdfInput = Join-Path $root 'Protected.pdf'
$pdfOriginal = 'PdfHistoryOriginal'
$pdfCurrent = 'PdfHistoryCurrent'
$pdfRegion = '0.08,0.05,0.50,0.12'
$pdfQuote = 'selection'
$pdfComment = 'PDF-HISTORY-ANNOTATION'
$pdfGroup = 'ProtectedSource'
$pdfExcerpt = 'PDF 摘錄 · 第 1 頁 · Protected.pdf'
foreach ($folder in $pdfHome,$pdfRestore) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
[IO.File]::WriteAllText($pdfSource, "PDF-HISTORY-BEFORE" + [char]10)
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../tests/fixtures/storyboard-zh/剪輯範圍.pdf') -Destination $pdfInput
$pdfInputSha = (Get-FileHash -LiteralPath $pdfInput -Algorithm SHA256).Hash
$pdfFileLabel = "Protected.pdf  ($((Get-Item -LiteralPath $pdfInput).Length) B)"

function Set-PdfHistoryProperty([string]$Previous,[string]$Value) {
    Native-Menu 113 4 '設定文字屬性…'
    Rule-Prompt '文字屬性' '' 'HistoryState'
    Rule-Prompt '設定文字屬性' $Previous $Value
}
function Open-PdfHistoryAttachment([int]$Item,[string]$Caption) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Choose-CollectionRow $pdfFileLabel
    Native-Menu 113 $Item $Caption -MenuOpen
}
function Set-PdfHistoryPrompt([string]$Caption,[string]$Hint,[string]$Value) {
    $prompt = Wait-For "PDF prompt $Hint" {
        $dialog = [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$Caption,1)
        if ($dialog -ne [IntPtr]::Zero -and [PaperroomHighlightGui]::DialogContains($dialog,$process.Id,$Hint)) { $dialog }
    }
    Type-SearchFilter $prompt 101 $Value
    Finish-Revision $prompt 1
}
function Wait-PdfHistoryPage {
    $null = Wait-For 'original PDF rendered on page one' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Protected.pdf · 1/2' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'PDF 操作'
    }
}
function Return-PdfHistoryNote {
    # Return during setup without creating a Win32 double-click on the sidebar button.
    Command $window 119
    $null = Wait-For 'return to note view after PDF or board' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
}
function Open-PdfHistoryBoard {
    # Call from note view. Confirm the mode before any drawing-surface interaction.
    Click-WorkspaceButton 119
    $null = Wait-For 'real whiteboard opened from note' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '返回筆記' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
    }
}
function Open-PdfHistoryPinMenu {
    Native-Menu 120 8 '分區釘選 PDF 標註…'
    Choose-CollectionRow "${pdfGroup}（1 張卡片）"
}
function Read-PdfHistoryWitness([string]$Database,[string]$Stage) {
    $state = @'
from contextlib import closing
from pathlib import Path
import hashlib, json, sqlite3, sys

database, pdf, stage = sys.argv[1:]
before, after = "PdfHistoryOriginal", "PdfHistoryCurrent"
quote, comment = "selection", "PDF-HISTORY-ANNOTATION"
old_text, new_text = "PDF-HISTORY-BEFORE\n", "PDF-HISTORY-AFTER"
with closing(sqlite3.connect(Path(database).resolve(strict=True).as_uri()+"?mode=ro", uri=True)) as db:
    db.execute("PRAGMA query_only=ON")
    db.execute("BEGIN")
    assert db.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
    assert not db.execute("PRAGMA foreign_key_check").fetchall()
    note, = db.execute("""SELECT n.id,i.token,n.title,n.body,n.version
        FROM notes n JOIN note_link_identities i ON i.note_id=n.id
        WHERE n.trash_root IS NULL AND n.title IN (?,?)""",(before,after)).fetchall()
    identity, token, title, body, _ = note
    assert token
    assert (title,body) == ((after,new_text) if stage=="current" else (before,old_text)), note
    properties = db.execute("SELECT key,value FROM note_properties WHERE note_id=? ORDER BY key",(identity,)).fetchall()
    assert properties == [("HistoryState","current")], properties
    attachment, = db.execute("SELECT id,name,kind,content FROM attachments WHERE note_id=?",(identity,)).fetchall()
    attachment_id, name, kind, data = attachment
    assert (name,kind)==("Protected.pdf","pdf") and data==Path(pdf).read_bytes()
    mark, = db.execute("""SELECT p.id,p.attachment_id,p.page,p.quote,p.comment,r.x,r.y,r.width,r.height
        FROM pdf_annotations p JOIN pdf_annotation_regions r ON r.annotation_id=p.id
        WHERE p.attachment_id=?""",(attachment_id,)).fetchall()
    assert mark[1:]==(attachment_id,1,quote,comment,0.08,0.05,0.50,0.12), mark
    excerpt, = db.execute("""SELECT c.note_id,c.origin_id,c.annotation_id,o.attachment_id,o.live_annotation,
        o.page,o.quote,o.comment,n.title,n.body FROM pdf_excerpt_cards c
        JOIN pdf_excerpt_origins o ON o.id=c.origin_id JOIN notes n ON n.id=c.note_id""").fetchall()
    assert excerpt[2:8]==(mark[0],attachment_id,mark[0],1,quote,comment), excerpt
    assert excerpt[8]=="PDF \u6458\u9304 \u00b7 \u7b2c 1 \u9801 \u00b7 Protected.pdf" and excerpt[9]==quote+"\n\n"+comment
    placement, = db.execute("""SELECT board_id,note_id,x,y FROM board_cards WHERE note_id=? ORDER BY board_id""",(excerpt[0],)).fetchall()
    excerpt_regions = db.execute("""SELECT ordinal,x,y,width,height FROM pdf_excerpt_origin_regions
        WHERE origin_id=? ORDER BY ordinal""",(excerpt[1],)).fetchall()
    assert excerpt_regions == [(-1, *mark[5:])], excerpt_regions
    pin, = db.execute("""SELECT b.id,g.id,s.annotation_id,m.note_id,p.x,p.y FROM board_group_sources s
        JOIN board_groups g ON g.id=s.group_id AND g.board_id=s.board_id
        JOIN boards b ON b.id=s.board_id JOIN board_group_cards m ON m.board_id=b.id AND m.group_id=g.id
        JOIN board_cards p ON p.board_id=b.id AND p.note_id=m.note_id
        WHERE g.title='ProtectedSource'""").fetchall()
    assert pin[2:4]==(mark[0],identity) and pin[0]==placement[0], pin
    revisions = db.execute("""SELECT r.id,r.prior_version,r.title,r.body,s.properties_json
        FROM note_revisions r JOIN note_revision_state s ON s.revision_id=r.id
        WHERE r.note_id=? ORDER BY r.id DESC""",(identity,)).fetchall()
    candidates = [r[0] for r in revisions if r[2:4]==(before,old_text) and json.loads(r[4])=={"HistoryState":"older"}
                  and not db.execute("SELECT 1 FROM note_revision_attachments WHERE revision_id=?",(r[0],)).fetchone()]
    assert candidates, revisions
    if stage=="restored":
        displaced = revisions[0]
        assert displaced[2:4]==(after,new_text) and json.loads(displaced[4])=={"HistoryState":"current"}, displaced
        assert db.execute("SELECT original_id,name,kind,content FROM note_revision_attachments WHERE revision_id=?",
            (displaced[0],)).fetchall()==[(attachment_id,name,kind,data)]
    print(json.dumps(dict(note=note,target_revision=candidates[0],revisions=revisions,
        preserved=dict(note_identity=[identity,token],attachment=[attachment_id,hashlib.sha256(data).hexdigest()],
            annotation=mark,excerpt=excerpt,excerpt_regions=excerpt_regions,placement=placement,pin=pin,properties=properties)),
        sort_keys=True,ensure_ascii=True))
'@ | python -B - $Database $pdfInput $Stage
    if ($LASTEXITCODE -ne 0 -or !$state) { throw "PDF history identity proof failed at $Stage." }
    return [string]$state
}
function Open-ProtectedPdfRevision([long]$Revision) {
    Native-Menu 113 22 '檢視與還原版本…'
    $suffix = "#$Revision"
    $labels = @(Collection-MenuLabels)
    $matches = @(for ($i=0; $i -lt $labels.Count; $i++) {
        if ($labels[$i].EndsWith($suffix,[StringComparison]::Ordinal) -and $labels[$i].StartsWith('版本 ')) { $i }
    })
    if ($matches.Count -ne 1) { throw "Could not uniquely find revision $suffix in the visible history list." }
    Native-Menu 113 ($matches[0]+1) $labels[$matches[0]] -MenuOpen
    Wait-For 'protected PDF history preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記版本差異',1)
    }
}
function Confirm-PdfHistoryPreview([IntPtr]$Preview,[long]$Attachment,[bool]$CanRestoreText) {
    $report = [PaperroomHighlightGui]::Read((Control $Preview 101))
    $identityLabel = "Protected.pdf　[#$Attachment]（PDF 標註、PDF 摘錄卡、白板分區釘選）"
    foreach ($line in $identityLabel,'整份還原會更動使用中的附件','HistoryState = older','附件（0）') {
        if (!$report.Contains($line)) { throw "Missing protected PDF preview evidence: $line." }
    }
    $accept = if ($CanRestoreText) {'只還原標題與正文'} else {'關閉'}
    if ([PaperroomHighlightGui]::Caption((Control $Preview 1)) -cne $accept) {
        throw "The protected preview must show '$accept'."
    }
    if ($CanRestoreText -and [PaperroomHighlightGui]::Caption((Control $Preview 2)) -cne '取消') {
        throw 'The protected preview lacks its explicit cancel control.'
    }
}

$process = Start-Notebook $pdfHome
$window = $process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $pdfSource '已匯入 1 篇 Markdown'
Select-Note $pdfOriginal
Set-PdfHistoryProperty '' 'older'
$field = Open-Editor
Replace-EditorText (Control $window 103) $pdfCurrent
Replace-EditorText $field 'PDF-HISTORY-AFTER'
Command $window 110
$null = Wait-For 'saved current PDF source title and prose' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $pdfCurrent -and
    [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('PDF-HISTORY-AFTER')
}
Set-PdfHistoryProperty 'older' 'current'
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 1 '加入本機附件…' -MenuOpen
Native-File 1 '加入本機附件…' $pdfInput '附件已保存到本機工作區。' -AlreadyOpened
Open-PdfHistoryAttachment 6 '新增 PDF 矩形座標摘錄…'
Set-PdfHistoryPrompt 'PDF 摘錄' '來源 PDF 頁碼' '1'
Set-PdfHistoryPrompt 'PDF 摘錄' '貼上原文' $pdfQuote
Set-PdfHistoryPrompt 'PDF 摘錄' '輸入批註' $pdfComment
Set-PdfHistoryPrompt 'PDF 矩形座標摘錄' '輸入 x,y,寬,高' $pdfRegion
Open-PdfHistoryAttachment 7 '開啟 PDF 頁面並標註'
Wait-PdfHistoryPage
Native-Menu 120 9 '從本頁標註建立摘錄卡…'
Choose-CollectionRow '第 1 頁（#'
Choose-CollectionRow '白板'
$null = Wait-For 'native excerpt opened' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $pdfExcerpt -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains($pdfComment)
}
Select-Note $pdfCurrent
Open-PdfHistoryBoard
Native-Menu 120 1 '放入筆記…'
Choose-CollectionRow $pdfCurrent
Native-Menu 120 4 '新增分區…'
Collection-Prompt '新增分區' $pdfGroup
$null = Wait-For 'board group created' {
    [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('分區已新增')
}
# The excerpt occupies slot zero. The source note was placed into slot one afterward.
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,1,$true)
Native-Menu 120 4 '設定分區…' -MenuOpen
Choose-CollectionRow $pdfGroup
Open-PdfHistoryPinMenu
Choose-CollectionRow '設定／更換釘選標註…'
Choose-CollectionRow "${pdfCurrent}（#"
Choose-CollectionRow 'Protected.pdf（#'
Choose-CollectionRow '第 1 頁（#'
$null = Wait-For 'board group PDF pin persisted' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已釘選 PDF 標註。'
}
Return-PdfHistoryNote
Write-Host 'PDF history fixture has an original annotation, excerpt, and pinned board reference.'
$baseline = Read-PdfHistoryWitness $pdfDb 'current'
$identity = $baseline | ConvertFrom-Json
$revision = $identity.target_revision
$attachment = $identity.preserved.attachment[0]
$preview = Open-ProtectedPdfRevision $revision
Confirm-PdfHistoryPreview $preview $attachment $true
Finish-Revision $preview 2
if ((Read-PdfHistoryWitness $pdfDb 'current') -cne $baseline) { throw 'Cancelled PDF history preview changed workspace state.' }

$preview = Open-ProtectedPdfRevision $revision
Confirm-PdfHistoryPreview $preview $attachment $true
Finish-Revision $preview 1
$null = Wait-For 'explicit text-only history restore' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $pdfOriginal -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('PDF-HISTORY-BEFORE') -and
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已還原標題與正文，保留目前的屬性與附件。'
}
$restoredState = Read-PdfHistoryWitness $pdfDb 'restored'
$restored = $restoredState | ConvertFrom-Json
if (($restored.preserved | ConvertTo-Json -Depth 10 -Compress) -cne
    ($identity.preserved | ConvertTo-Json -Depth 10 -Compress)) { throw 'Text restore changed protected PDF records.' }

$preview = Open-ProtectedPdfRevision $revision
Confirm-PdfHistoryPreview $preview $attachment $false
Finish-Revision $preview 1
if ((Read-PdfHistoryWitness $pdfDb 'restored') -cne $restoredState) { throw 'Reopening an equal text preview changed history.' }
Native-File 4 '備份工作區…' $pdfBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$sourceHash = (Get-FileHash -LiteralPath $pdfDb -Algorithm SHA256).Hash
$backupHash = (Get-FileHash -LiteralPath $pdfBackup -Algorithm SHA256).Hash
Copy-Item -LiteralPath $pdfBackup -Destination $pdfRestoreDb
if ((Get-FileHash -LiteralPath $pdfRestoreDb -Algorithm SHA256).Hash -cne $backupHash) { throw 'Isolated backup copy does not match.' }

foreach ($candidate in @(@{Home=$pdfHome;Db=$pdfDb},@{Home=$pdfRestore;Db=$pdfRestoreDb})) {
    $process = Start-Notebook $candidate.Home
    $window = $process.MainWindowHandle
    Select-Note $pdfOriginal
    $null = Wait-For 'restored original note in reading mode' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('PDF-HISTORY-BEFORE')
    }
    Open-PdfHistoryBoard
    Open-PdfHistoryPinMenu
    Choose-CollectionRow '開啟釘選標註'
    Wait-PdfHistoryPage
    Return-PdfHistoryNote
    Select-Note $pdfExcerpt
    Native-Menu 113 9 '開啟 PDF 摘錄來源'
    Wait-PdfHistoryPage
    Native-Menu 120 4 '檢視本頁標註'
    $report = Wait-For 'original PDF annotation visible from excerpt' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','本頁標註',1)
    }
    $text = [PaperroomHighlightGui]::Read((Control $report 101))
    if (!$text.Contains($pdfQuote) -or !$text.Contains($pdfComment)) { throw 'The reopened PDF annotation is missing.' }
    Finish-Revision $report 1
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    if ((Read-PdfHistoryWitness $candidate.Db 'restored') -cne $restoredState) {
        throw 'PDF identity, properties, or note history changed during reopening.'
    }
}
if ((Get-FileHash -LiteralPath $pdfBackup -Algorithm SHA256).Hash -cne $backupHash -or
    (Get-FileHash -LiteralPath $pdfDb -Algorithm SHA256).Hash -cne $sourceHash -or
    (Get-FileHash -LiteralPath $pdfInput -Algorithm SHA256).Hash -cne $pdfInputSha) {
    throw 'PDF history validation changed the source PDF, source workspace, or backup.'
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    backup_sha256=$backupHash
    restored_revision=$revision
    identity=$restored.preserved
    scenarios=@('protected PDF full restore unavailable with explicit text-only fallback',
        'cancelled preview wrote nothing','identical text cannot create a new revision',
        'original PDF attachment, annotation, excerpt, whiteboard pin and current property preserved',
        'original reopen and isolated backup restore opened the PDF from the pin and excerpt')
} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: PDF identity-safe text history, cancellation, missing full restore option, and original plus isolated reopening. EXE SHA-256 $ExeSha256."
