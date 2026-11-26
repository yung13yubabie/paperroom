# Exercise a real search-to-editor transition through an immutable #21 Windows EXE.
$focusHome = Join-Path $root 'focus-live'
$focusDb = Join-Path $focusHome 'paperroom-data/notes.sqlite3'
$focusZip = Join-Path $root 'FocusBoards.zip'
$focusBase = Join-Path $root 'Focus.base'
$focusBackup = Join-Path $root 'focus-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $focusDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $focusHome 'Paperroom.exe')
@'
import json,sys,zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_DEFLATED) as out:
    out.writestr("All-Data.json", "Synthetic marker only.")
    out.writestr("甲/鏡頭.md", "# 鏡頭\n\n甲正文\n[乙卡](../乙/鏡頭.md)")
    out.writestr("乙/鏡頭.md", "# 鏡頭\n\n乙正文")
    for name,x in (("分鏡",20),("參考",30)):
        doc = {"nodes":[{"id":"a","type":"file","file":"甲/鏡頭.md","x":x,"y":20,"width":232,"height":114}]}
        out.writestr(f"Boards/{name}.canvas",json.dumps(doc))
'@ | python - $focusZip
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the actual import ZIP.' }
[IO.File]::WriteAllText($focusBase, @'
views:
  - type: list
    name: 甲場
    filters: 'file.folder == "甲"'
    order: [file.path, file.name]
'@, [Text.UTF8Encoding]::new($false))
$zipHash = (Get-FileHash -LiteralPath $focusZip).Hash
$baseHash = (Get-FileHash -LiteralPath $focusBase).Hash

function Search-ExactFocusNote([string]$Query) {
    $dialog = Open-WorkspaceSearch
    Type-SearchFilter $dialog 101 $Query
    Type-SearchFilter $dialog 103 '甲'
    Search-Workspace $dialog '找到 1 筆筆記。' 1
    Finish-Revision $dialog 111
    $null = Wait-For 'exact same-title note from folder 甲' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭'
    }
}
function Assert-FocusPreview([string]$Expected) {
    $preview = Control $window 121
    $null = Wait-For 'exact saved note preview' {
        [PaperroomHighlightGui]::IsWindowVisible($preview) -and
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
        [PaperroomHighlightGui]::Read($preview).Contains($Expected) -and
        ![PaperroomHighlightGui]::Read($preview).Contains('乙正文')
    }
}
function Open-FocusCollection {
    Open-SortCollection 'Focus／甲場'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
    Native-Menu 120 1 '甲場（清單）' -MenuOpen
    Native-Menu 120 3 '開啟資料列…'
    Choose-CollectionRow '已匯入　鏡頭  ('
}
function Open-FocusBoardCard([string]$Name) {
    Native-Menu 120 60003 '查看此筆記的原生白板位置…'
    $rows = @(Collection-MenuLabels)
    if ($rows.Count -ne 2 -or $Name -cnotin $rows) { throw 'The original note must occur on both boards.' }
    $index = [Array]::IndexOf([string[]]$rows,$Name) + 1
    Native-Menu 120 $index $Name -MenuOpen
    $null = Wait-For 'the app located the original board card' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位白板上的筆記卡片。'
    }
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
}
function Read-FocusIdentity([string]$Path, [string]$Expected) {
    $identity = @'
from contextlib import closing
from pathlib import Path
import sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    ident=db.execute("SELECT n.id,i.token,n.body,n.version FROM notes n JOIN vault_paths p ON p.note_id=n.id JOIN note_link_identities i ON i.note_id=n.id WHERE p.relative_path='甲/鏡頭.md'").fetchone()
    note,token,body,version=ident
    assert body == sys.argv[2]+"\n[乙卡](../乙/鏡頭.md)",repr(body)
    assert db.execute("SELECT n.body FROM notes n JOIN vault_paths p ON p.note_id=n.id WHERE p.relative_path='乙/鏡頭.md'").fetchall()==[("乙正文",)]
    assert db.execute("SELECT b.title,c.x,c.y FROM board_cards c JOIN boards b ON b.id=c.board_id WHERE c.note_id=? ORDER BY c.x",(note,)).fetchall()==[("分鏡",20,20),("參考",30,20)]
    assert db.execute("SELECT cn.note_id FROM collection_notes cn JOIN collections c ON c.id=cn.collection_id WHERE c.name='Focus／甲場'").fetchall()==[(note,)]
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    print(f"{note}:{token}:{version}")
'@ | python - $Path $Expected
    if ($LASTEXITCODE -ne 0 -or !$identity) { throw 'The saved note, identity or projection did not match.' }
    return [string]$identity
}

$process = Start-Notebook $focusHome
$window = $process.MainWindowHandle
$notice = Native-File 20 '匯入 Markdown ZIP（相對附件）…' $focusZip '已建立 2 張原生白板' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $notice 6
$report = Wait-For 'two original board imports' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('已建立 2 張原生白板')) { throw 'Missing both boards.' }
Finish-Revision $report 1
Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
$prompt = Native-File 40 '匯入 Obsidian Bases 檢視…' $focusBase '已匯入 1 個 Bases 檢視' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $prompt 6
$null = Wait-For 'the collection is ready' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 1 個 Bases 檢視。'
}
$original = Read-FocusIdentity $focusDb '甲正文'
# A body hit already opens the editor. Clicking Edit here would switch to reading.
Search-ExactFocusNote '甲正文'
Assert-SearchSelection 104 '甲正文' '全域搜尋：正文第 1 行'
$editor = Control $window 104
[PaperroomHighlightGui]::TypeEditorText($window,$process.Id,$editor,'甲正文已修改')
$null = Wait-For 'first physical edit saved' {
    [PaperroomHighlightGui]::Read($editor).StartsWith('甲正文已修改') -and
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
}
$first = Read-FocusIdentity $focusDb '甲正文已修改'
$before = $original.Split(':')
$after = $first.Split(':')
if ($before[0] -cne $after[0] -or $before[1] -cne $after[1] -or [int]$after[2] -le [int]$before[2]) {
    throw 'The first edit must advance the very same note.'
}
Open-FocusCollection
Assert-FocusPreview '甲正文已修改'
Open-FocusBoardCard '分鏡'
Assert-FocusPreview '甲正文已修改'
Open-FocusBoardCard '參考'
Assert-FocusPreview '甲正文已修改'
Native-File 4 '備份工作區…' $focusBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null
$backupHash = (Get-FileHash -LiteralPath $focusBackup).Hash
$focusRestore = Join-Path $root 'focus-restore'
$restoredDb = Join-Path $focusRestore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $focusRestore 'Paperroom.exe')
Copy-Item -LiteralPath $focusBackup -Destination $restoredDb
foreach ($check in @(@($focusHome,$focusDb),@($focusRestore,$restoredDb))) {
    $process = Start-Notebook $check[0]
    $window = $process.MainWindowHandle
    Open-FocusCollection
    Assert-FocusPreview '甲正文已修改'
    Open-FocusBoardCard '參考'
    Assert-FocusPreview '甲正文已修改'
    if ((Read-FocusIdentity $check[1] '甲正文已修改') -cne $first) { throw 'The note identity changed across reopen or backup.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
}
if ((Get-FileHash -LiteralPath $focusZip).Hash -cne $zipHash -or
    (Get-FileHash -LiteralPath $focusBase).Hash -cne $baseHash -or
    (Get-FileHash -LiteralPath $focusBackup).Hash -cne $backupHash) {
    throw 'An input or the original backup changed.'
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('body-search-enters-editing','focused-utf16-selection-and-physical-input',
       'same-id-two-board-cards-and-collection','reopen','isolated-backup-restore','unchanged-decoy-and-sources')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: the exact EXE changes the focused note and shows the saved body through both boards, the collection, reopen and isolated restore."
