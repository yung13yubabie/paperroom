# Sourced by windows-highlight-workflow.ps1 after its other isolated app instances are closed.
function Open-NotionCsvCollectionPrompt([string]$AttachmentName) {
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Native-Menu 113 1 "$AttachmentName  (" -MenuOpen -CaptionPrefix
    Native-Menu 113 10 '從 Notion CSV 建立新集合…' -MenuOpen
    Wait-For 'new Notion CSV collection prompt' {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '從 Notion CSV 建立新集合', 1)
    }
}

function Assert-NotionCsvWorkspace([string]$Phase) {
    @'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import sqlite3, sys

path, archive, phase = sys.argv[1:]
parent = "Project 11111111111111111111111111111111"
child = "Child 22222222222222222222222222222222"
table = "Catalog 33333333333333333333333333333333_all.csv"
with ZipFile(archive) as source, closing(sqlite3.connect(Path(path).as_uri() + '?mode=ro', uri=True)) as db:
    page = db.execute("SELECT id, body FROM notes WHERE title='資料手冊'").fetchone()
    nested = db.execute("SELECT id, parent FROM notes WHERE title='巢狀子頁'").fetchone()
    assert page and nested and nested[1] == page[0]
    assert '[到子頁](Project%2011111111111111111111111111111111/Child%2022222222222222222222222222222222.md)' in page[1]
    media = db.execute("""SELECT content, relative_path FROM attachments
        JOIN attachment_origins ON attachment_id=attachments.id WHERE note_id=?""", (nested[0],)).fetchone()
    assert media == (source.read(parent + '/local.txt'), parent + '/local.txt'), media
    samples = db.execute("SELECT note_id,name,content FROM attachments WHERE name LIKE '%.csv' ORDER BY name").fetchall()
    assert [(name, content) for _, name, content in samples] == [
        ('Broken.csv', source.read('Broken.csv')),
        (table, source.read(parent + '/' + table)),
    ]
    collections = db.execute("SELECT id,name FROM collections").fetchall()
    if phase in ('canceled', 'invalid'):
        assert not collections
        assert db.execute("SELECT count(*) FROM note_properties").fetchone() == (0,)
        assert db.execute("SELECT count(*) FROM notes").fetchone() == (6,)
    else:
        assert len(collections) == 1 and collections[0][1] == '書目集合'
        members = db.execute("""SELECT n.id,n.title,f.parent FROM collection_notes c JOIN notes n ON n.id=c.note_id
            JOIN notes f ON f.id=n.parent WHERE collection_id=? ORDER BY n.title""", (collections[0][0],)).fetchall()
        by_title = {title:(note, parent_id) for note,title,parent_id in members}
        assert set(by_title) == {'書目甲','書目乙'}
        row_id, source_id = by_title['書目甲']
        assert source_id == samples[1][0]
        fields = dict(db.execute("SELECT key,value FROM note_properties WHERE note_id=?", (row_id,)))
        assert fields['編號'] == '00042'
        assert fields['關聯文字'] == 'https://example.invalid/notion/item'
        assert fields['備註'] == ('第一行\r\n第二行,引文' if phase == 'created' else '編輯後仍保留原始 CSV')
        assert db.execute("SELECT count(*) FROM note_relations").fetchone() == (0,)
    print(f'Notion CSV real-workspace verification: {phase}')
'@ | python - $notionDb $notionArchive $Phase
    if ($LASTEXITCODE -ne 0) { throw "The Notion ZIP workspace check failed during $Phase." }
}

$notionFolder = Join-Path $root 'NotionCsvReview'
$notionDb = Join-Path $notionFolder 'paperroom-data\notes.sqlite3'
$notionArchive = Join-Path $root 'NotionSource.zip'
[void][IO.Directory]::CreateDirectory((Split-Path $notionDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $notionFolder 'Paperroom.exe')

@'
from zipfile import ZipFile
import sys

parent = 'Project 11111111111111111111111111111111'
child = 'Child 22222222222222222222222222222222'
csv = '\ufeff名稱,編號,關聯文字,備註\r\n書目甲,00042,https://example.invalid/notion/item,"第一行\r\n第二行,引文"\r\n書目乙,00000,,\r\n'
with ZipFile(sys.argv[1], 'x') as z:
    z.writestr(parent + '.md', '# 資料手冊\n\n[到子頁](Project%2011111111111111111111111111111111/Child%2022222222222222222222222222222222.md)\n')
    z.writestr(parent + '/' + child + '.md', '# 巢狀子頁\n\n[本地附件](local.txt)\n')
    z.writestr(parent + '/local.txt', '紙間原始本地附件。')
    z.writestr(parent + '/Catalog 33333333333333333333333333333333_all.csv', csv)
    z.writestr('Broken.csv', '名稱,備註\r\n欄位不足\r\n')
'@ | python - $notionArchive
if ($LASTEXITCODE -ne 0) { throw 'Could not create the synthetic Notion export.' }
$sourceHash = (Get-FileHash -LiteralPath $notionArchive).Hash
$process = Start-Notebook $notionFolder
$window = $process.MainWindowHandle
Native-File 2 '匯入 Notion Markdown ZIP…' $notionArchive '已匯入 2 篇 Markdown'
$dialog = Wait-For 'Notion import report' {
    [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '匯入報告', 1)
}
$report = [PaperroomHighlightGui]::Read((Control $dialog 101))
if (!$report.Contains('公式與關聯不會重建') -or !$report.Contains('不代表可在紙間跳轉')) {
    throw "The import report did not explain the migration boundaries: $report"
}
Finish-Revision $dialog 1

Select-Note '巢狀子頁'
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 2 '管理附件…' -MenuOpen
Native-Menu 113 1 'local.txt  (' -MenuOpen -CaptionPrefix
Native-Menu 113 9 '查看匯入來源路徑' -MenuOpen
$dialog = Wait-For 'original local attachment path' {
    [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '匯入時的相對附件路徑', 1)
}
if ([PaperroomHighlightGui]::Read((Control $dialog 101)) -cne 'Project 11111111111111111111111111111111/local.txt') {
    throw 'The native attachment path differs from the source path.'
}
Finish-Revision $dialog 1

$catalog = 'Catalog 33333333333333333333333333333333_all.csv'
Select-Note "2｜$catalog"
$dialog = Open-NotionCsvCollectionPrompt $catalog
Finish-Revision $dialog 2
Assert-NotionCsvWorkspace 'canceled'

Select-Note '1｜Broken.csv'
$dialog = Open-NotionCsvCollectionPrompt 'Broken.csv'
Type-SearchFilter $dialog 101 '不該建立的集合'
Finish-Revision $dialog 1
$errorDialog = Wait-For 'malformed CSV refusal' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
}
if (![PaperroomHighlightGui]::DialogContains($errorDialog,$process.Id,'CSV 每列欄位數必須一致')) {
    throw 'The refusal did not explain the malformed CSV.'
}
Finish-Revision $errorDialog 1
Assert-NotionCsvWorkspace 'invalid'

Select-Note "2｜$catalog"
$dialog = Open-NotionCsvCollectionPrompt $catalog
Type-SearchFilter $dialog 101 '書目集合'
Finish-Revision $dialog 1
$null = Wait-For 'new collection and two CSV notes saved' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已建立集合，匯入 2 列。原始 CSV 已保留。'
}
Assert-NotionCsvWorkspace 'created'
Select-Note '書目甲'
Native-Menu 113 4 '設定文字屬性…'
$dialog = Wait-For 'existing property selector' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','文字屬性',1)
}
Type-SearchFilter $dialog 101 '備註'
Finish-Revision $dialog 1
$dialog = Wait-For 'existing CSV field editor' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','設定文字屬性',1)
}
$field = Control $dialog 101
if (![PaperroomHighlightGui]::Read($field).Contains('第二行,引文')) { throw 'The editable field lost its CSV value.' }
Type-SearchFilter $dialog 101 '編輯後仍保留原始 CSV'
Finish-Revision $dialog 1

Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $notionFolder
$window = $process.MainWindowHandle
Select-Note '書目甲'
Native-Menu 113 6 '檢視文字屬性…'
$dialog = Wait-For 'properties visible after reopening' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記的文字屬性',1)
}
$props = [PaperroomHighlightGui]::Read((Control $dialog 101))
if (!$props.Contains('編號：00042') -or !$props.Contains('備註：編輯後仍保留原始 CSV')) {
    throw 'The raw identifier or edited property was lost after reopen.'
}
Finish-Revision $dialog 1
Assert-NotionCsvWorkspace 'reopened'
if ((Get-FileHash -LiteralPath $notionArchive).Hash -cne $sourceHash) { throw 'Source ZIP bytes changed.' }
Close-Notebook $process
$process.Dispose()
$process = $null
Write-Host 'PASS: native Notion ZIP hierarchy, original Markdown link and attachment, cancel and malformed CSV refusal, editable collection, reopen and unchanged source ZIP and CSV bytes.'
