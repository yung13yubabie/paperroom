# Sourced by the real downloaded-EXE GUI suite after its other apps close.
function Select-NotionHeading {
    Select-Note 'Notion 入口'
    Native-Menu 120 2 '前往　同層標題 #唯一%20取景　｜ 甲 11111111111111111111111111111111/鏡頭 A.md' -CaptionPrefix
    $preview = Wait-For 'exact Notion heading preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','#唯一%20取景',1)
    }
    if (![PaperroomHighlightGui]::Read((Control $preview 101)).Contains('甲資料夾特有內容')) {
        throw 'The exact-title fragment opened the wrong Notion page.'
    }
    Finish-Revision $preview 1
    $null = Wait-For 'real editor selected the unique Notion heading' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名鏡頭' -and
        [PaperroomHighlightGui]::Read((Control $window 104)).Contains('甲資料夾特有內容') -and
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('標題連結：第 1 行，唯一 取景')
    }
}

$notionHome = Join-Path $root 'NotionLinkWorkspace'
$notionRestore = Join-Path $root 'NotionLinkRestore'
$notionArchive = Join-Path $root 'NotionLinks.zip'
$notionBackup = Join-Path $root 'NotionBackup.sqlite3'
foreach ($folder in @($notionHome,$notionRestore)) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}

@'
from zipfile import ZipFile
import sys

a = '甲 11111111111111111111111111111111'
b = '乙 22222222222222222222222222222222'
body = (
    '[同層標題](./%E9%8F%A1%E9%A0%AD%20A.md#唯一%20取景)\n\n'
    '[父層](../乙%2022222222222222222222222222222222/鏡頭%20A.md)\n\n'
    '[未知片段](鏡頭%20A.md#unknown-block-id)\n\n'
    '[字面百分號](進度%2520.md)\n\n'
    '[缺失](../無此資料夾/鏡頭%20A.md) [外部](https://example.invalid/鏡頭%20A.md) [附件](本地.txt)\n'
)
with ZipFile(sys.argv[1], 'x') as out:
    out.writestr(a + '/入口.md', '# Notion 入口\n\n' + body)
    out.writestr(a + '/鏡頭 A.md', '# 同名鏡頭\n\n## 唯一 取景\n甲資料夾特有內容\n')
    out.writestr(b + '/鏡頭 A.md', '# 同名鏡頭\n\n乙資料夾特有內容\n')
    out.writestr(a + '/進度%20.md', '# 百分號頁\n\n一次解碼後的精確路徑\n')
    out.writestr(a + '/本地.txt', '不變更的本地附件')
'@ | python - $notionArchive
if ($LASTEXITCODE -ne 0) { throw 'Could not construct the synthetic Notion ZIP.' }
$notionArchiveHash = (Get-FileHash -LiteralPath $notionArchive).Hash

$process = Start-Notebook $notionHome
$window = $process.MainWindowHandle
Native-File 2 '匯入 Notion Markdown ZIP…' $notionArchive '已匯入 4 篇 Markdown'
$report = Wait-For 'native Notion link import report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('片段只認唯一的字面標題')) {
    throw 'The native import report did not explain the heading scope.'
}
Finish-Revision $report 1
Select-NotionHeading

Select-Note 'Notion 入口'
Native-Menu 120 3 '前往　父層　｜ 乙 22222222222222222222222222222222/鏡頭 A.md' -CaptionPrefix
$null = Wait-For 'parent-relative path opened the other same-name source' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名鏡頭' -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('乙資料夾特有內容')
}

Select-Note 'Notion 入口'
Native-Menu 120 4 '前往　未知片段 #unknown-block-id' -CaptionPrefix
$null = Wait-For 'unknown fragment reached its note and gave an explicit status' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名鏡頭' -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲資料夾特有內容') -and
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟來源筆記。找不到這個標題。'
}

Select-Note 'Notion 入口'
Native-Menu 120 6 '開啟筆記關聯圖…'
$null = Wait-For 'the link menu exposes only the four resolved relative links' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '關聯圖操作'
}
Select-Note 'Notion 入口'
Native-File 4 '備份工作區…' $notionBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $notionHome
$window = $process.MainWindowHandle
Select-NotionHeading
Close-Notebook $process
$process.Dispose()
$process = $null

$originDb = Join-Path $notionHome 'paperroom-data/notes.sqlite3'
$restoreDb = Join-Path $notionRestore 'paperroom-data/notes.sqlite3'
$originHash = (Get-FileHash -LiteralPath $originDb).Hash
$backupHash = (Get-FileHash -LiteralPath $notionBackup).Hash
Copy-Item -LiteralPath $notionBackup -Destination $restoreDb
if ((Get-FileHash -LiteralPath $restoreDb).Hash -cne $backupHash) { throw 'The isolated restore copy differs.' }

$process = Start-Notebook $notionRestore
$window = $process.MainWindowHandle
Select-NotionHeading
Select-Note 'Notion 入口'
Native-Menu 120 5 '前往　字面百分號　｜ 甲 11111111111111111111111111111111/進度%20.md' -CaptionPrefix
$null = Wait-For 'literal percent in the same restored exact path' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '百分號頁' -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('一次解碼後的精確路徑')
}
Close-Notebook $process
$process.Dispose()
$process = $null

@'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import sqlite3, sys

a = '甲 11111111111111111111111111111111'
with ZipFile(sys.argv[1]) as source, closing(sqlite3.connect(Path(sys.argv[2]).as_uri() + '?mode=ro', uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone() == ('ok',)
    rows = db.execute("""SELECT p.note_id, p.import_root, p.archive_part, p.relative_path, n.title, n.body
        FROM notion_page_paths p JOIN notes n ON n.id=p.note_id""").fetchall()
    assert len(rows) == 4 and len({r[1] for r in rows}) == 1 and all(r[2] == '' for r in rows)
    for note_id, _, _, relative, title, body in rows:
        assert source.read(relative).decode() == f'# {title}\n\n{body}', relative
        if relative == a + '/入口.md':
            assert db.execute("""SELECT o.relative_path,a.content FROM attachments a
                JOIN attachment_origins o ON a.id=o.attachment_id WHERE a.note_id=?""",(note_id,)).fetchone() == (
                a + '/本地.txt', source.read(a + '/本地.txt'))
'@ | python - $notionArchive $restoreDb
if ($LASTEXITCODE -ne 0) { throw 'Restored Notion source identity or preserved bytes differed.' }
if ((Get-FileHash -LiteralPath $notionArchive).Hash -cne $notionArchiveHash -or
    (Get-FileHash -LiteralPath $notionBackup).Hash -cne $backupHash -or
    (Get-FileHash -LiteralPath $originDb).Hash -cne $originHash) { throw 'Original ZIP, workspace or backup changed.' }
Write-Host 'PASS: native Notion ZIP relative page links, same-name scopes, exact and unsupported headings, percent decoding, unchanged sources, reopening, UI backup and real isolated restore.'
