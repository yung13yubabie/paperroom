# Real GUI smoke for the three source-detail imports in an isolated portable workspace.
$detailHome = Join-Path $root 'source-detail-live'
$detailDb = Join-Path $detailHome 'paperroom-data/notes.sqlite3'
$detailBackup = Join-Path $root 'source-detail.sqlite3'
$detailNotion = Join-Path $root 'NotionDetails.zip'
$detailCanvas = Join-Path $root 'Boards.zip'
$detailVault = Join-Path $root 'SceneVault'
$detailBase = Join-Path $root 'SceneDetails.base'
[void][IO.Directory]::CreateDirectory((Split-Path $detailDb -Parent))
[void][IO.Directory]::CreateDirectory((Join-Path $detailVault '甲'))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $detailHome 'Paperroom.exe')
$encoded = [Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $detailVault '甲/鏡頭.md'), @'
---
status: 準備
checked: false
number: 0
tags: [studio, notes]
---
原始 Markdown 場次
'@, $encoded)
[IO.File]::WriteAllText($detailBase, @'
views:
  - type: list
    name: SourceDetails
    filters: 'file.folder == "甲"'
    order: [file.path, status, note.checked, number]
'@, $encoded)
'{"schema":"paperroom-notion-mcp-transfer/2","confirmed_page_ids":["11111111-1111-1111-1111-111111111111","22222222-2222-2222-2222-222222222222"],"pages":[{"id":"11111111-1111-1111-1111-111111111111","title":"同名頁","markdown":"可驗甲正文","truncated":false,"unknown_block_count":0,"unknown_block_ids":[]},{"id":"22222222-2222-2222-2222-222222222222","title":"同名頁","markdown":"可驗乙正文","truncated":false,"unknown_block_count":0,"unknown_block_ids":[]}],"confirmed_page_references":[{"source_page_id":"11111111-1111-1111-1111-111111111111","block_parent_page_id":"11111111-1111-1111-1111-111111111111","block_id":"33333333-3333-3333-3333-333333333333","block_type":"paragraph","rich_text_type":"mention","mention_type":"page","target_page_id":"22222222-2222-2222-2222-222222222222","code":false}]}' |
    python -B (Join-Path $PSScriptRoot 'notion_mcp_transfer.py') $detailNotion
if ($LASTEXITCODE -ne 0) { throw 'Could not create the test Notion transfer.' }
@'
import json,sys,zipfile
doc={"nodes":[
  {"id":"g","type":"group","x":-40,"y":-40,"width":700,"height":450,"label":"繁中群組"},
  {"id":"a","type":"file","file":"甲/鏡頭.md","x":-10,"y":10,"width":300,"height":100},
  {"id":"b","type":"file","file":"甲/鏡頭.md","x":340,"y":30,"width":250,"height":140}],
  "edges":[{"id":"ab","fromNode":"a","toNode":"b","label":"方向"}]}
with zipfile.ZipFile(sys.argv[1],"w",zipfile.ZIP_DEFLATED) as z:
    z.writestr("All-Data.json","Synthetic marker. Not vendor JSON.")
    z.writestr("甲/鏡頭.md","# 畫布筆記\n\nCanvas 來源")
    z.writestr("Boards/複合.canvas",json.dumps(doc,ensure_ascii=False))
'@ | python - $detailCanvas
if ($LASTEXITCODE -ne 0) { throw 'Could not create the standard JSON Canvas fixture.' }
$detailHashes = @{}
foreach ($file in @($detailNotion,$detailCanvas,$detailBase,(Join-Path $detailVault '甲/鏡頭.md'))) {
    $detailHashes[$file] = (Get-FileHash -LiteralPath $file).Hash
}

function Close-DetailReport([IntPtr]$Dialog, [string]$Expected) {
    if (![PaperroomHighlightGui]::Read((Control $Dialog 101)).Contains($Expected)) {
        throw "Missing imported work report: $Expected"
    }
    Finish-Revision $Dialog 1
}

$process = Start-Notebook $detailHome
$window = $process.MainWindowHandle
$detailReport = Native-File 2 '匯入 Notion Markdown ZIP…' $detailNotion '已匯入 3 篇 Markdown' -NextDialog '匯入報告'
Close-DetailReport $detailReport '紙間 MCP 文字交接標記'
$notice = Native-File 20 '匯入 Markdown ZIP（相對附件）…' $detailCanvas '保存 1 份可編輯 Canvas' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $notice 6
$detailReport = Wait-For 'Canvas preservation report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
Close-DetailReport $detailReport '保存 1 份可編輯 Canvas'
Native-VaultFolder $detailVault '匯入 1 篇筆記'
Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
$prompt = Native-File 40 '匯入 Obsidian Bases 檢視…' $detailBase '已匯入 1 個 Bases 檢視' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $prompt 6
$null = Wait-For 'saved Bases source note' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'Bases 來源：SceneDetails.base'
}
Native-File 4 '備份工作區…' $detailBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$detailBackupHash = (Get-FileHash -LiteralPath $detailBackup).Hash
$detailRestore = Join-Path $root 'source-detail-restore'
$restoredDb = Join-Path $detailRestore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $detailRestore 'Paperroom.exe')
Copy-Item -LiteralPath $detailBackup -Destination $restoredDb
$process = Start-Notebook $detailRestore
$window = $process.MainWindowHandle

Select-Note '本機頁面引用'
Native-Menu 120 2 '前往　同名頁' -CaptionPrefix
$null = Wait-For 'explicit link found the other exact same-title note after restore' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名頁' -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('可驗乙正文')
}
Native-Menu 109 12 '開啟已匯入的 Canvas…'
Native-Menu 109 1 'Boards/複合.canvas' -MenuOpen
$null = Wait-For 'saved complex Canvas opened with native controls' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
}
Open-SortCollection 'SceneDetails／SourceDetails'
Native-Menu 120 11 '集合檢視…'
Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
Native-Menu 120 1 'SourceDetails（清單）' -MenuOpen
Native-Menu 120 24 '表格與清單欄位…'
Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
Choose-CollectionRow '已匯入　鏡頭  ('
$detailReport = Wait-For 'restored Bases displayed fields' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
}
$text = [PaperroomHighlightGui]::Read((Control $detailReport 101))
foreach ($field in @('file.path　甲/鏡頭.md','status　準備','checked　false','number　0')) {
    if (!$text.Contains($field)) { throw "Saved Bases view did not display $field" }
}
Finish-Revision $detailReport 1
Close-Notebook $process
$process.Dispose()
$process = $null

@'
import sys,zipfile,sqlite3
from pathlib import Path
with sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as db, zipfile.ZipFile(sys.argv[2]) as z:
    saved=db.execute("select id,source from saved_canvases where title='Boards/複合.canvas'").fetchone()
    assert saved[1].encode()==z.read("Boards/複合.canvas")
    assert not db.execute("select 1 from canvas_vault_bindings where canvas_id=?",(saved[0],)).fetchall()
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
'@ | python - $restoredDb $detailCanvas
if ($LASTEXITCODE -ne 0) { throw 'Restored rich Canvas or isolated SQLite integrity differs.' }
foreach ($file in $detailHashes.Keys) {
    if ((Get-FileHash -LiteralPath $file).Hash -cne $detailHashes[$file]) { throw 'An original source fixture changed.' }
}
if ((Get-FileHash -LiteralPath $detailBackup).Hash -cne $detailBackupHash) { throw 'The source workspace backup changed.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('notion-same-title-explicit-reference','editable-standard-canvas','bases-scalars','isolated-backup-reopen')
    sources_unchanged=$true
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: exact EXE detail workflows after isolated backup."
