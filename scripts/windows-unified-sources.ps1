# Uses the verified downloaded executable and a single fresh synthetic workspace.
$testHome = Join-Path $root 'unified-source-fixture'
[void][IO.Directory]::CreateDirectory((Join-Path $testHome 'paperroom-data'))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $testHome 'Paperroom.exe')
$db = Join-Path $testHome 'paperroom-data/notes.sqlite3'
$transfer = Join-Path $root 'NotionTransfer.zip'
$heptaZip = Join-Path $root 'MarkdownAndCanvases.zip'
$bases = Join-Path $root '場次.base'
$workspaceBackup = Join-Path $root 'unified.sqlite3'
$enc = [Text.UTF8Encoding]::new($false)
'{"schema":"paperroom-notion-mcp-transfer/1","confirmed_page_ids":["11111111-1111-1111-1111-111111111111"],"pages":[{"id":"11111111-1111-1111-1111-111111111111","title":"MCP 交接驗收","markdown":"# 檢核\n\n- [x] 合成原文","truncated":false,"unknown_block_count":0,"unknown_block_ids":[]}]}' |
    python -B (Join-Path $PSScriptRoot 'notion_mcp_transfer.py') $transfer
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the synthetic MCP handoff.' }
@'
import json,sys,zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_DEFLATED) as out:
    out.writestr("All-Data.json","Synthetic marker only. Not vendor JSON.")
    out.writestr("甲/鏡頭.md","# 鏡頭\n\n甲正文\n[乙卡](../乙/鏡頭.md)\n[媒體](../媒體/來源.txt)")
    out.writestr("乙/鏡頭.md","# 鏡頭\n\n乙正文")
    out.writestr("媒體/來源.txt","合成附件")
    for board, pos in (("分鏡",20),("參考",50)):
        nodes = [
            dict(id="a",type="file",file="甲/鏡頭.md",x=pos,y=20,width=232,height=114),
            dict(id="b",type="file",file="乙/鏡頭.md",x=350,y=190,width=232,height=114),
        ]
        edge = dict(id="ab",fromNode="a",toNode="b",toEnd="none")
        out.writestr(f"Boards/{board}.canvas",json.dumps(dict(nodes=nodes,edges=[edge])))
'@ | python - $heptaZip
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the synthetic Markdown and JSON Canvas handoff.' }
[IO.File]::WriteAllText($bases, @'
views:
  - type: list
    name: 甲場
    filters: 'file.folder == "甲"'
    order: [file.folder, file.name]
'@, $enc)
$hashes = @{}
foreach ($file in $transfer,$heptaZip,$bases) { $hashes[$file] = (Get-FileHash -LiteralPath $file).Hash }

function Confirm-SourceReport([IntPtr]$Dialog, [string]$Expected) {
    $report = [PaperroomHighlightGui]::Read((Control $Dialog 101))
    if (!$report.Contains($Expected)) { throw "The native report did not describe the expected operation." }
    Finish-Revision $Dialog 1
    $null = Wait-For 'closed import report' { ![PaperroomHighlightGui]::IsWindowVisible($Dialog) }
}
$process = Start-Notebook $testHome
$window = $process.MainWindowHandle
$report = Native-File 2 '匯入 Notion Markdown ZIP…' $transfer '已匯入 1 篇 Markdown' -NextDialog '匯入報告'
Confirm-SourceReport $report '紙間 MCP 文字交接標記'

$notice = Native-File 20 '匯入 Markdown ZIP（相對附件）…' $heptaZip '已建立 2 張紙間白板' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $notice 6
$report = Wait-For 'Heptabase companion report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
Confirm-SourceReport $report '已建立 2 張紙間白板'
$null = Wait-For 'the imported Markdown root is selected' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '匯入：MarkdownAndCanvases'
}

Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
$prompt = Native-File 40 '匯入 Obsidian Bases 檢視…' $bases '已匯入 1 個 Bases 檢視' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $prompt 6
$null = Wait-For 'native Bases source receipt opened' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'Bases 來源：場次.base' -and
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 1 個 Bases 檢視。'
}

Open-LinkedFixture '鏡頭' '甲'
Click-WorkspaceButton 110
$null = Wait-For 'same card title is editable in the native note Open result' {
    [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) -and
    [PaperroomHighlightGui]::Read((Control $window 104)).StartsWith('甲正文')
}
$noteEditor = Control $window 104
$null = Wait-For 'Edit button placed focus in the source editor' {
    [PaperroomHighlightGui]::EditorHasFocus($window,$process.Id,$noteEditor)
}
$oldText = [PaperroomHighlightGui]::Read($noteEditor)
$caret = [IntPtr]$oldText.Length
[void][PaperroomHighlightGui]::Send($noteEditor,0x00B1,$caret,$caret)
[PaperroomHighlightGui]::TypeEditorText($window,$process.Id,$noteEditor,"`nUnified note edit")
$null = Wait-For 'native editor received the appended text' {
    [PaperroomHighlightGui]::Read($noteEditor).EndsWith('Unified note edit')
}
$null = Wait-For 'native note body saved' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
}
Native-File 4 '備份工作區…' $workspaceBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$backupHash = (Get-FileHash -LiteralPath $workspaceBackup).Hash
$restoredHome = Join-Path $root 'unified-source-restore'
$restoredDb = Join-Path $restoredHome 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restoredHome 'Paperroom.exe')
Copy-Item -LiteralPath $workspaceBackup -Destination $restoredDb
foreach ($case in @(@($testHome,$db),@($restoredHome,$restoredDb))) {
    $process = Start-Notebook $case[0]
    $window = $process.MainWindowHandle
    Open-LinkedFixture '鏡頭' '甲'
    Open-SortCollection '場次／甲場'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
    Native-Menu 120 1 '甲場（清單）' -MenuOpen
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    $rows = @(Collection-MenuLabels)
    if ($rows.Count -ne 1) { throw 'The saved Bases view displayed an unexpected number of rows.' }
    Choose-CollectionRow '已匯入　鏡頭  ('
    $display = Wait-For 'actual imported Bases values' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
    }
    $projection = [PaperroomHighlightGui]::Read((Control $display 101))
    foreach ($expected in @('筆記標題　鏡頭','file.folder　甲','file.name　鏡頭.md')) {
        if (!$projection.Contains($expected)) { throw 'The named collection view lost a file snapshot.' }
    }
    Finish-Revision $display 1
    @'
from contextlib import closing
from pathlib import Path
import sqlite3,sys
db=Path(sys.argv[1])
with closing(sqlite3.connect(db.as_uri()+"?mode=ro",uri=True)) as conn:
    assert conn.execute("pragma journal_mode").fetchone()==("wal",)
    assert conn.execute("pragma integrity_check").fetchone()==("ok",)
    assert conn.execute("select body from notes where title='MCP 交接驗收'").fetchone()==("# 檢核\n\n- [x] 合成原文",)
    aid=conn.execute("select id from notes where title='鏡頭' and instr(body,'Unified note edit')>0").fetchone()[0]
    bid=conn.execute("select id from notes where title='鏡頭' and body='乙正文'").fetchone()[0]
    assert aid != bid
    assert conn.execute("select a.content from attachments a where a.note_id=?",(aid,)).fetchone()[0]=="合成附件".encode()
    assert conn.execute("select count(*) from board_cards where note_id=?",(aid,)).fetchone()==(2,)
    assert conn.execute("select count(*) from board_edges where from_note=? and to_note=?",(min(aid,bid),max(aid,bid))).fetchone()==(2,)
    assert conn.execute("select note_id from collection_notes cn join collections c on c.id=cn.collection_id where c.name='場次／甲場'").fetchall()==[(aid,)]
    assert dict(conn.execute("select key,value from obsidian_base_file_values where note_id=?",(aid,)))=={
        "file.folder":"甲", "file.name":"鏡頭.md"}
    assert conn.execute("select count(*) from note_properties where note_id=? and key in ('file.folder','file.name')",(aid,)).fetchone()==(0,)
    assert conn.execute("select list_mode from collection_views where name='甲場'").fetchone()==(1,)
    assert conn.execute("select content from attachments where name='場次.base'").fetchone()[0]==Path(sys.argv[2]).read_bytes()
'@ | python - $case[1] $bases
    if ($LASTEXITCODE -ne 0) { throw 'Unified workspace readback disagrees with the exact source, IDs or backup.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
}
foreach ($file in $transfer,$heptaZip,$bases) {
    if ((Get-FileHash -LiteralPath $file).Hash -cne $hashes[$file]) { throw 'The source fixture changed.' }
}
if ((Get-FileHash -LiteralPath $workspaceBackup).Hash -cne $backupHash) { throw 'Opening a backup copy changed the original backup.' }
Write-Host "PASS: native Notion MCP transfer, Heptabase Markdown and two standard Canvas companions, Obsidian Bases on the same note IDs, edit, reopen and isolated backup restore. EXE SHA-256 $ExeSha256."

if ($env:PAPERROOM_UNIFIED_RECEIPT) {
    @{
        exe_sha256 = $ExeSha256
        os = (Get-CimInstance Win32_OperatingSystem).Caption
        scenarios = @('notion-mcp-synthetic-transfer','heptabase-markdown-standard-canvas',
            'obsidian-bases-displayed-after-edit','reopen','isolated-backup')
        source_fixtures_unchanged = $true
        original_backup_unchanged = $true
    } | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
}
