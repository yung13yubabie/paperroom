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
function Show-IntegratedVaultPictures {
    foreach ($colors in $jointVaultColors.Values) {
        $rgb = $colors.root
        $null = Wait-For 'native Vault-root pixels in the correct Canvas' { Read-CanvasInk $rgb[0] $rgb[1] $rgb[2] }
    }
}
function Assert-IntegratedWorkspace([string]$Database, [string]$VaultEvidenceFolder) {
    $snapshot = @'
from contextlib import closing
from hashlib import sha256
from pathlib import Path
from zipfile import ZipFile
import json, sqlite3, sys

database, notion, canvas, portable, vault = map(Path, sys.argv[1:])
source = canvas.read_bytes()
png = (canvas.parent / "附件" / "畫面.png").read_bytes()
vault_scene = vault / "Boards" / "VaultRestoreScene.canvas"
vault_raw = vault_scene.read_bytes()
vault_doc = json.loads(vault_raw)
vault_assets = {
    "group_background": next(n["background"] for n in vault_doc["nodes"] if n["type"] == "group"),
    "file_card": next(n["file"] for n in vault_doc["nodes"] if n["type"] == "file"),
}
with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db, \
     ZipFile(notion) as raw, ZipFile(portable) as moved:
    assert db.execute("PRAGMA user_version").fetchone() == (41,)
    assert db.execute("PRAGMA integrity_check").fetchall() == [("ok",)]
    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    assert set(moved.namelist()) == {canvas.name, "附件/畫面.png"}
    assert moved.read(canvas.name) == source and moved.read("附件/畫面.png") == png
    assert db.execute("SELECT title,source FROM saved_canvases ORDER BY id").fetchall() == [
        ("共同畫布", source.decode()), ("共同畫布", source.decode()),
        ("VaultRestoreScene", vault_raw.decode()),
    ]
    assert db.execute("""SELECT path,bytes FROM canvas_bundle_assets
        JOIN saved_canvases ON id=canvas_id WHERE title='共同畫布' ORDER BY canvas_id""").fetchall() == [
        ("附件/畫面.png", png), ("附件/畫面.png", png)
    ]
    bundled = dict(db.execute("""SELECT path,bytes FROM canvas_bundle_assets
        JOIN saved_canvases ON id=canvas_id WHERE title='VaultRestoreScene'"""))
    assert set(bundled) == set(vault_assets.values())
    assert db.execute("""SELECT canvas_path FROM canvas_bundles
        JOIN saved_canvases ON id=canvas_id WHERE title='VaultRestoreScene'""").fetchone() == ("VaultRestoreScene.canvas",)
    hashes = {}
    for role, path in vault_assets.items():
        root = (vault / path).read_bytes()
        decoy = (vault / "Boards" / path).read_bytes()
        assert bundled[path] == root and bundled[path] != decoy, (role, path)
        hashes[role] = {"member": path, "workspace_sha256": sha256(bundled[path]).hexdigest(),
                        "vault_root_sha256": sha256(root).hexdigest(),
                        "same_folder_decoy_sha256": sha256(decoy).hexdigest()}
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
    row = dict(db.execute("SELECT key,value FROM note_properties JOIN notes ON id=note_id WHERE title='追蹤列甲'"))
    assert row == {"名稱":"追蹤列甲", "編號":"00042", "備註":"修改後欄值"}
    assert db.execute("SELECT count(*) FROM notes WHERE title IN ('不可寫入列','禁止建立')").fetchone() == (0,)
    print(json.dumps({"root_assets":hashes,"edited_csv_row":row,"canvas_sha256":sha256(vault_raw).hexdigest()}, sort_keys=True))
'@ | python - $Database $jointNotion $jointCanvas $jointPortable $VaultEvidenceFolder
    if ($LASTEXITCODE -ne 0) { throw 'The actual joint workspace and ZIP payloads did not match.' }
    return $snapshot
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
$jointVault = Join-Path $jointInputFolder 'Vault'
$jointVaultScene = Join-Path $jointVault 'Boards/VaultRestoreScene.canvas'
$jointDetachedVault = Join-Path $jointInputFolder 'DetachedVault'
$jointVaultColors = (Get-Content -LiteralPath $jointFixture -Raw | ConvertFrom-Json -AsHashtable).vault_asset_colors
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
def write_png(path, rgb):
    path.parent.mkdir(parents=True, exist_ok=True)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress((b"\0" + bytes(rgb)*16)*16)) + chunk(b"IEND", b"")
    path.write_bytes(png)
write_png(root / "附件" / "畫面.png", [37,146,179])
vault = root / "Vault"
for path, colors in source["vault_asset_colors"].items():
    write_png(vault / path, colors["root"])
    write_png(vault / "Boards" / path, colors["decoy"])
(vault / "Boards/VaultRestoreScene.canvas").write_text(
    json.dumps(source["vault_canvas"], ensure_ascii=False), encoding="utf-8")
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
Native-MenuFromKeyboard 109 39 '從 Obsidian Vault 匯入 Canvas…'
Pick-NativeVaultFolder $jointVault
$vaultPreview = Native-File 39 '從 Obsidian Vault 匯入 Canvas…' $jointVaultScene '' -AlreadyOpened -NextDialog '預覽 Obsidian Vault Canvas'
$previewText = [PaperroomHighlightGui]::Read((Control $vaultPreview 101))
foreach ($part in 'Boards/VaultRestoreScene.canvas','將保存：Media/RestoreFrame.png','將保存：Media/RestoreBackground.png') {
    if (!$previewText.Contains($part)) { throw "The explicit Vault preview lost $part." }
}
Finish-Revision $vaultPreview 1
$null = Wait-For 'explicit Vault-root import saved' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已從 Obsidian Vault 匯入 Canvas 與附件。'
}
Show-IntegratedVaultPictures
$liveAssetProof = Assert-IntegratedWorkspace $jointDb $jointVault
Native-File 4 '備份工作區…' $jointBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process = $null

$jointLiveHash = (Get-FileHash -LiteralPath $jointDb).Hash
$jointBackupHash = (Get-FileHash -LiteralPath $jointBackup).Hash
Copy-Item -LiteralPath $jointBackup -Destination $jointCloneDb
$jointCloneHash = (Get-FileHash -LiteralPath $jointCloneDb).Hash
if ($jointCloneHash -cne $jointBackupHash) { throw 'The isolated restore clone is not the actual UI backup.' }
# Temporarily detach only the generated test Vault. The EXE opens the clone
# without access to the original Vault paths.
Move-Item -LiteralPath $jointVault -Destination $jointDetachedVault
if (Test-Path -LiteralPath $jointVaultScene) { throw 'The original Vault path is still available.' }
$process = Start-Notebook $jointClone
$window = $process.MainWindowHandle
Show-IntegratedLink 2 '甲'
Show-IntegratedLink 3 '乙'
Show-IntegratedEditedRow
Open-IntegratedCanvas
Native-Menu 109 12 '開啟已匯入的 Canvas…'
Native-Menu 109 3 'VaultRestoreScene' -MenuOpen
Show-IntegratedVaultPictures
$restoredAssetProof = Assert-IntegratedWorkspace $jointCloneDb $jointDetachedVault
if ($restoredAssetProof -cne $liveAssetProof -or (Test-Path -LiteralPath $jointVaultScene)) {
    throw 'Vault-root assets or the edited row differ after restoring with the original Vault detached.'
}
Close-Notebook $process
$process.Dispose()
$process = $null
Move-Item -LiteralPath $jointDetachedVault -Destination $jointVault
foreach ($source in $jointSourceHashes) {
    if ((Get-FileHash -LiteralPath $source.Path).Hash -cne $source.Hash) { throw 'A source fixture was edited.' }
}
if ((Get-FileHash -LiteralPath $jointDb).Hash -cne $jointLiveHash -or
    (Get-FileHash -LiteralPath $jointBackup).Hash -cne $jointBackupHash) { throw 'The isolated restore changed its original or backup.' }
if ($env:PAPERROOM_VAULT_RESTORE_RECEIPT) {
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    @{
        source_commit = $env:GITHUB_SHA
        run_id = $env:GITHUB_RUN_ID
        package_sha256 = $env:PACKAGE_HASH
        exe_sha256 = $ExeSha256
        os_caption = $os.Caption
        os_build = $os.BuildNumber
        backup_sha256 = $jointBackupHash.ToLowerInvariant()
        restored_copy_before_open_sha256 = $jointCloneHash.ToLowerInvariant()
        original_workspace_sha256 = $jointLiveHash.ToLowerInvariant()
        original_vault_unavailable_during_restore = $true
        restored_workspace = ($restoredAssetProof | ConvertFrom-Json -AsHashtable)
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $env:PAPERROOM_VAULT_RESTORE_RECEIPT -Encoding utf8
}
Write-Host "PASS: one Notion ZIP across CSV conversion and edit, exact same-name page links, atomic typed-column refusal, Canvas PNG ZIP round trip and native backup restored in isolation. The detached Vault's root file card and group background survived with the CSV edit. EXE SHA-256 $ExeSha256."
