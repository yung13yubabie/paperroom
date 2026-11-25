# Sourced after the individual Notion and Canvas cases. All three features share these workspaces.
function Show-IntegratedLink([int]$Item, [string]$Side) {
    Select-Note '驗收入口'
    Native-Menu 120 $Item "前往　到${Side}　｜ 專案 11111111111111111111111111111111/${Side}/鏡頭.md" -CaptionPrefix
    $null = Wait-For "Notion relative link opened $Side by exact source path" {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名鏡頭' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains("$Side" + '子頁專屬正文。')
    }
}
function Show-IntegratedEditedRow {
    Select-Note '追蹤列甲'
    Native-Menu 113 6 '檢視文字屬性…'
    $dialog = Wait-For 'edited CSV row fields' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記的文字屬性',1)
    }
    $fields = [PaperroomHighlightGui]::Read((Control $dialog 101))
    if (!$fields.Contains('編號：00042') -or !$fields.Contains('備註：修改後欄值')) {
        throw "The CSV row properties did not survive: $fields"
    }
    Finish-Revision $dialog 1
}
function Get-IntegratedWorkspaceStamp([string]$Database) {
    $stamp = @'
from contextlib import closing
from hashlib import sha256
from pathlib import Path
import sqlite3, sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri() + "?mode=ro", uri=True)) as db:
    print(sha256("\n".join(db.iterdump()).encode("utf-8")).hexdigest())
'@ | python - $Database
    if ($LASTEXITCODE -ne 0 -or $stamp -cnotmatch '^[a-f0-9]{64}$') { throw 'Could not verify the refusal snapshot.' }
    return $stamp
}
function Open-IntegratedCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 '共同畫布' -MenuOpen
    $null = Wait-For 'real local PNG on the shared-workspace Canvas' { Read-CanvasInk 37 146 179 }
}
function Assert-IntegratedWorkspace([string]$Database) {
    @'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import sqlite3, sys

database, notion, canvas, portable = map(Path, sys.argv[1:])
source = canvas.read_bytes()
png = (canvas.parent / "附件" / "畫面.png").read_bytes()
with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db, \
     ZipFile(notion) as raw, ZipFile(portable) as moved:
    assert db.execute("PRAGMA user_version").fetchone() == (31,)
    assert db.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    assert set(moved.namelist()) == {canvas.name, "附件/畫面.png"}
    assert moved.read(canvas.name) == source and moved.read("附件/畫面.png") == png
    assert db.execute("SELECT title,source FROM saved_canvases ORDER BY id").fetchall() == [
        ("共同畫布", source.decode()), ("共同畫布", source.decode())
    ]
    assert db.execute("SELECT path,bytes FROM canvas_bundle_assets ORDER BY canvas_id").fetchall() == [
        ("附件/畫面.png", png), ("附件/畫面.png", png)
    ]
    paths = dict(db.execute("SELECT relative_path, body FROM notion_page_paths JOIN notes ON id=note_id"))
    assert len(paths) == 4 and all(raw.read(path).decode().split("\n\n", 1)[1] == body for path, body in paths.items())
    for name, data in db.execute("SELECT name,content FROM attachments WHERE name LIKE '%.csv'"):
        member, = (path for path in raw.namelist() if path.endswith("/" + name))
        assert raw.read(member) == data
    assert db.execute("SELECT count(*) FROM attachments WHERE name LIKE '%.csv'").fetchone() == (2,)
    assert db.execute("SELECT key,kind FROM property_definitions").fetchall() == [("工時", "number")]
    assert db.execute("SELECT name FROM collections").fetchall() == [("聯合驗收集合",)]
    assert db.execute("SELECT title FROM notes JOIN collection_notes ON id=note_id ORDER BY title").fetchall() == [
        ("追蹤列乙",), ("追蹤列甲",)
    ]
    assert dict(db.execute("SELECT key,value FROM note_properties JOIN notes ON id=note_id WHERE title='追蹤列甲'")) == {
        "名稱":"追蹤列甲", "編號":"00042", "備註":"修改後欄值"
    }
    assert db.execute("SELECT count(*) FROM notes WHERE title IN ('不可寫入列','禁止建立')").fetchone() == (0,)
'@ | python - $Database $jointNotion $jointCanvas $jointPortable
    if ($LASTEXITCODE -ne 0) { throw 'The actual joint workspace and ZIP payloads did not match.' }
}

$jointHome = Join-Path $root 'NotionCanvasWorkspace'
$jointClone = Join-Path $root 'NotionCanvasClone'
$jointInputFolder = Join-Path $root 'NotionCanvasInputs'
$jointNotion = Join-Path $jointInputFolder 'NotionJoint.zip'
$jointCanvas = Join-Path $jointInputFolder '共同畫布.canvas'
$jointPortable = Join-Path $root 'JointCanvas.zip'
$jointDb = Join-Path $jointHome 'paperroom-data/notes.sqlite3'
$jointCloneDb = Join-Path $jointClone 'paperroom-data/notes.sqlite3'
$jointBackup = Join-Path $root 'NotionCanvasBackup.sqlite3'
$jointFixture = Join-Path $PSScriptRoot '../tests/fixtures/notion-canvas-integration.json'
foreach ($folder in $jointHome,$jointClone) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
@'
from pathlib import Path
from zipfile import ZipFile
import json, struct, sys, zlib

source = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
root = Path(sys.argv[2])
(root / "附件").mkdir(parents=True)
with ZipFile(root / "NotionJoint.zip", "x") as archive:
    for path, body in source["notion"].items():
        archive.writestr(path, body.encode("utf-8"))
(root / "共同畫布.canvas").write_text(json.dumps(source["canvas"], ensure_ascii=False), encoding="utf-8")
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress((b"\0" + bytes([37,146,179])*16)*16)) + chunk(b"IEND", b"")
(root / "附件" / "畫面.png").write_bytes(png)
'@ | python - $jointFixture $jointInputFolder
if ($LASTEXITCODE -ne 0) { throw 'Could not create the combined real Notion and Canvas fixture.' }
$jointSourceHashes = Get-ChildItem -LiteralPath $jointInputFolder -File -Recurse | Get-FileHash

$process = Start-Notebook $jointHome
$window = $process.MainWindowHandle
$report = Native-File 2 '匯入 Notion Markdown ZIP…' $jointNotion '' -NextDialog '匯入報告'
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('Notion CSV 原檔 2 份')) {
    throw 'The single archive did not expose both CSV sources.'
}
Finish-Revision $report 1
Show-IntegratedLink 2 '甲'
Show-IntegratedLink 3 '乙'

Select-Note '2｜BB-valid.csv'
$prompt = Open-NotionCsvCollectionPrompt 'BB-valid.csv'
Type-SearchFilter $prompt 101 '聯合驗收集合'
Finish-Revision $prompt 1
$null = Wait-For 'joint CSV rows created' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已建立集合，匯入 2 列。原始 CSV 已保留。'
}
Select-Note '追蹤列甲'
Native-Menu 113 4 '設定文字屬性…'
Rule-Prompt '文字屬性' '' '備註'
Rule-Prompt '設定文字屬性' 'CSV 原始值' '修改後欄值'
Show-IntegratedEditedRow
Show-IntegratedLink 2 '甲'
Show-IntegratedLink 3 '乙'

Native-Menu 109 14 '屬性型別…'
Native-Menu 109 1 '設定屬性型別…' -MenuOpen
Rule-Prompt '設定工作區屬性型別' '' '工時'
Native-Menu 109 1 '數字' -MenuOpen
$null = Wait-For 'workspace number field in effect' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新工作區屬性型別，原有筆記值已保留。'
}
Select-Note '1｜AA-conflict.csv'
$before = Get-IntegratedWorkspaceStamp $jointDb
$prompt = Open-NotionCsvCollectionPrompt 'AA-conflict.csv'
Type-SearchFilter $prompt 101 '禁止建立'
Finish-Revision $prompt 1
$refusal = Wait-For 'visible non-text field CSV refusal' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
}
if (![PaperroomHighlightGui]::DialogContains($refusal,$process.Id,'「工時」已在工作區宣告為非文字屬性。原始 CSV 已保留。')) {
    throw 'The actual error did not identify the conflicting typed field.'
}
[PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
$null = Wait-For 'typed CSV refusal dismissed' { ![PaperroomHighlightGui]::IsWindowVisible($refusal) }
if ((Get-IntegratedWorkspaceStamp $jointDb) -cne $before) { throw 'The rejected conversion wrote workspace data.' }
Write-Host 'Verified same-archive post-edit path navigation and a typed-column refusal with unchanged database rows.'

Native-MenuFromKeyboard 109 38 '匯入 JSON Canvas 與同資料夾附件…'
$preview = Native-File 38 '匯入 JSON Canvas 與同資料夾附件…' $jointCanvas '' -AlreadyOpened -NextDialog '預覽 Canvas 與同資料夾附件'
if (![PaperroomHighlightGui]::Read((Control $preview 101)).Contains('將保存：附件/畫面.png')) {
    throw 'The same workspace did not recognize the local Canvas attachment.'
}
Finish-Revision $preview 1
$null = Wait-For 'imported Canvas PNG appears' { Read-CanvasInk 37 146 179 }
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $jointHome
$window = $process.MainWindowHandle
Show-IntegratedLink 2 '甲'
Show-IntegratedEditedRow
Open-IntegratedCanvas
Native-Menu 120 9 '匯出 JSON Canvas ZIP…'
Native-File 9 '匯出 JSON Canvas ZIP…' $jointPortable '已匯出 JSON Canvas ZIP 及保存的附件。' -AlreadyOpened -SaveName 'canvas.zip'
Native-MenuFromKeyboard 109 31 '匯入 JSON Canvas ZIP…'
$zipReport = Native-File 31 '匯入 JSON Canvas ZIP…' $jointPortable '' -AlreadyOpened -NextDialog 'Canvas ZIP 匯入報告'
Finish-Revision $zipReport 1
$null = Wait-For 'reimported Canvas ZIP renders the same actual PNG' { Read-CanvasInk 37 146 179 }
Assert-IntegratedWorkspace $jointDb
Native-File 4 '備份工作區…' $jointBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null

$jointLiveHash = (Get-FileHash -LiteralPath $jointDb).Hash
$jointBackupHash = (Get-FileHash -LiteralPath $jointBackup).Hash
Copy-Item -LiteralPath $jointBackup -Destination $jointCloneDb
if ((Get-FileHash -LiteralPath $jointCloneDb).Hash -cne $jointBackupHash) { throw 'The isolated restore clone is not the actual UI backup.' }
$process = Start-Notebook $jointClone
$window = $process.MainWindowHandle
Show-IntegratedLink 2 '甲'
Show-IntegratedLink 3 '乙'
Show-IntegratedEditedRow
Open-IntegratedCanvas
Assert-IntegratedWorkspace $jointCloneDb
Close-Notebook $process
$process.Dispose()
$process = $null
foreach ($source in $jointSourceHashes) {
    if ((Get-FileHash -LiteralPath $source.Path).Hash -cne $source.Hash) { throw 'A source fixture was edited.' }
}
if ((Get-FileHash -LiteralPath $jointDb).Hash -cne $jointLiveHash -or
    (Get-FileHash -LiteralPath $jointBackup).Hash -cne $jointBackupHash) { throw 'The isolated restore changed its original or backup.' }
Write-Host "PASS: one Notion ZIP across CSV conversion and edit, exact same-name page links, atomic typed-column refusal, Canvas PNG ZIP round trip and native backup restored in isolation. EXE SHA-256 $ExeSha256."
