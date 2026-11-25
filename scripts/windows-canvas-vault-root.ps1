# Run both source scopes on the same synthetic nested Obsidian Canvas.
function Verify-CanvasScopePixels([int[]]$Frame, [int[]]$Background) {
    $null = Wait-For 'real native pixels for the scoped file node' {
        Read-CanvasInk $Frame[0] $Frame[1] $Frame[2]
    }
    $null = Wait-For 'real native pixels for the scoped group background' {
        Read-CanvasInk $Background[0] $Background[1] $Background[2]
    }
}

function Read-CanvasScopeDb([string]$Database, [string]$ScopeFolder, [string]$Export = '-') {
    @'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import sqlite3, sys

db, source, scope, export = sys.argv[1:]
db,source,scope = map(Path, (db,source,scope))
with closing(sqlite3.connect(db.as_uri() + "?mode=ro", uri=True)) as connection:
    rows = connection.execute("SELECT id,title,source FROM saved_canvases").fetchall()
    packages = connection.execute("SELECT canvas_path FROM canvas_bundles").fetchall()
    assets = dict(connection.execute("SELECT path,bytes FROM canvas_bundle_assets"))
    if scope == Path("-"):
        assert rows == [] and packages == [] and assets == {}, (rows, packages, assets)
    else:
        raw = source.read_bytes()
        expected = {
            name:(scope / name).read_bytes()
            for name in ("Media/Frame.png", "Media/Background.png", "Media/OnlyInBoards.txt")
            if (scope / name).is_file()
        }
        assert len(rows) == 1 and rows[0][1:] == ("Scene", raw.decode("utf-8"))
        assert packages == [("Scene.canvas",)] and assets == expected
        if export != "-":
            with ZipFile(export) as bundle:
                assert set(bundle.namelist()) == set(expected) | {"Scene.canvas"}
                assert bundle.read("Scene.canvas") == raw
                assert all(bundle.read(name) == data for name,data in expected.items())
'@ | python - $Database $scopeScene $ScopeFolder $Export
    if ($LASTEXITCODE -ne 0) { throw 'The scoped Canvas storage or ZIP contents differ.' }
}

function Open-CanvasScopePreview([string]$Mode) {
    if ($Mode -ceq 'vault') {
        Native-MenuFromKeyboard 109 39 '從 Obsidian Vault 匯入 Canvas…'
        Pick-NativeVaultFolder $scopeVault
        Native-File 39 '從 Obsidian Vault 匯入 Canvas…' $scopeScene '' -AlreadyOpened -NextDialog '預覽 Obsidian Vault Canvas'
    } else {
        Native-MenuFromKeyboard 109 38 '匯入 JSON Canvas 與同資料夾附件…'
        Native-File 38 '匯入 JSON Canvas 與同資料夾附件…' $scopeScene '' -AlreadyOpened -NextDialog '預覽 Canvas 與同資料夾附件'
    }
}

function Open-SavedScopeCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'Scene' -MenuOpen
}

$scopeVault = Join-Path $root 'ScopeVault'
$scopeBoards = Join-Path $scopeVault 'Boards'
$scopeScene = Join-Path $scopeBoards 'Scene.canvas'
$scopeOutside = Join-Path $root 'ScopeOutside/Scene.canvas'
@'
from pathlib import Path
import json, struct, sys, zlib

vault = Path(sys.argv[1])
scene = Path(sys.argv[2])
outside = Path(sys.argv[3])
def png(path, rgb):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    raw = (b"\0" + bytes(rgb)*12)*12
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 12, 12, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
(vault / ".obsidian").mkdir(parents=True)
for folder, frame, bg in [(vault,(21,116,183),(23,145,106)),(scene.parent,(176,67,130),(188,101,36))]:
    media = folder / "Media"
    media.mkdir(parents=True)
    png(media / "Frame.png", frame)
    png(media / "Background.png", bg)
(scene.parent / "Media/OnlyInBoards.txt").write_text("Canvas-folder only", encoding="utf-8")
body = {
    "nodes": [
        {"id":"g","type":"group","x":-200,"y":-150,"width":400,"height":300,"label":"場景",
         "background":"Media/Background.png","backgroundStyle":"cover"},
        {"id":"f","type":"file","x":-100,"y":-60,"width":200,"height":120,"file":"Media/Frame.png"},
        {"id":"m","type":"file","x":220,"y":0,"width":110,"height":90,"file":"Media/OnlyInBoards.txt"},
        {"id":"a","type":"file","x":220,"y":100,"width":110,"height":90,"file":"MEDIA/Frame.png"},
        {"id":"l","type":"link","x":220,"y":205,"width":110,"height":90,"url":"https://example.invalid"}
    ],
    "edges":[{"id":"e","fromNode":"f","toNode":"m","toEnd":"arrow"}]
}
raw = json.dumps(body, ensure_ascii=False).encode("utf-8")
scene.write_bytes(raw)
outside.parent.mkdir()
outside.write_bytes(raw)
'@ | python - $scopeVault $scopeScene $scopeOutside
if ($LASTEXITCODE -ne 0) { throw 'Could not create the sample Vault and competing PNGs.' }
$scopeOriginal = Get-ChildItem -LiteralPath $scopeVault -File -Recurse | Get-FileHash

foreach ($case in @(
    @{ Mode='vault'; Folder=$scopeVault; Frame=@(21,116,183); Bg=@(23,145,106);
       Status='已從 Obsidian Vault 匯入 Canvas 與附件。'; Extra='缺少：Media/OnlyInBoards.txt' },
    @{ Mode='folder'; Folder=$scopeBoards; Frame=@(176,67,130); Bg=@(188,101,36);
       Status='已匯入 Canvas 與同資料夾附件。'; Extra='將保存：Media/OnlyInBoards.txt' }
)) {
    $liveFolder = Join-Path $root ("ScopeLive-" + $case.Mode)
    $cloneFolder = Join-Path $root ("ScopeClone-" + $case.Mode)
    $liveDb = Join-Path $liveFolder 'paperroom-data/notes.sqlite3'
    $cloneDb = Join-Path $cloneFolder 'paperroom-data/notes.sqlite3'
    $scopeZip = Join-Path $root ("Scope-" + $case.Mode + ".zip")
    foreach ($dir in $liveFolder,$cloneFolder) {
        [void][IO.Directory]::CreateDirectory((Join-Path $dir 'paperroom-data'))
        Copy-Item -LiteralPath $Exe -Destination (Join-Path $dir 'Paperroom.exe')
    }
    $process = Start-Notebook $liveFolder
    $window = $process.MainWindowHandle
    if ($case.Mode -ceq 'vault') {
        Native-MenuFromKeyboard 109 39 '從 Obsidian Vault 匯入 Canvas…'
        Pick-NativeVaultFolder $scopeVault
        Native-File 39 '從 Obsidian Vault 匯入 Canvas…' $scopeOutside '拒絕 Vault 之外' -AlreadyOpened -ExpectedError '請選擇位於已選 Vault 內的 .canvas 檔。'
        Read-CanvasScopeDb $liveDb '-'
        $preview = Open-CanvasScopePreview 'vault'
        Finish-Revision $preview 2
        Read-CanvasScopeDb $liveDb '-'
    }
    $preview = Open-CanvasScopePreview $case.Mode
    $report = [PaperroomHighlightGui]::Read((Control $preview 101))
    foreach ($line in '將保存：Media/Frame.png','將保存：Media/Background.png','缺少：MEDIA/Frame.png',$case.Extra) {
        if (!$report.Contains($line)) { throw "Scoped preview lacks '$line'. Preview: $report" }
    }
    if ($case.Mode -ceq 'vault' -and !$report.Contains('Boards/Scene.canvas')) {
        throw 'The Vault-root preview lost the original nested path.'
    }
    Finish-Revision $preview 1
    $null = Wait-For 'actual scoped Canvas import completed' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq $case.Status
    }
    Verify-CanvasScopePixels $case.Frame $case.Bg
    Read-CanvasScopeDb $liveDb $case.Folder
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $liveFolder
    $window = $process.MainWindowHandle
    Open-SavedScopeCanvas
    Verify-CanvasScopePixels $case.Frame $case.Bg
    Native-Menu 120 9 '匯出 JSON Canvas ZIP…'
    Native-File 9 '匯出 JSON Canvas ZIP…' $scopeZip '已匯出 JSON Canvas ZIP 及保存的附件。' -AlreadyOpened -SaveName 'canvas.zip'
    Read-CanvasScopeDb $liveDb $case.Folder $scopeZip
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $cloneFolder
    $window = $process.MainWindowHandle
    Native-MenuFromKeyboard 109 31 '匯入 JSON Canvas ZIP…'
    $result = Native-File 31 '匯入 JSON Canvas ZIP…' $scopeZip '' -AlreadyOpened -NextDialog 'Canvas ZIP 匯入報告'
    if ($case.Mode -ceq 'vault' -and ![PaperroomHighlightGui]::Read((Control $result 101)).Contains('缺少：Media/OnlyInBoards.txt')) {
        throw 'The ZIP round trip unexpectedly rebound the missing Vault-root reference.'
    }
    Finish-Revision $result 1
    Verify-CanvasScopePixels $case.Frame $case.Bg
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $cloneFolder
    $window = $process.MainWindowHandle
    Open-SavedScopeCanvas
    Verify-CanvasScopePixels $case.Frame $case.Bg
    Read-CanvasScopeDb $cloneDb $case.Folder $scopeZip
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host "Verified scoped preview, both sets of PNG bytes and pixels after ZIP reimport and restart: $($case.Mode)"
}
foreach ($source in $scopeOriginal) {
    if ((Get-FileHash -LiteralPath $source.Path).Hash -cne $source.Hash) { throw 'The source Vault has been changed.' }
}
Write-Host "PASS: explicit Obsidian Vault-root and legacy Canvas-folder resolution used distinct file/background PNGs, refused the outside Canvas, kept the source case and missing paths and survived both ZIP reimports and restarts. EXE SHA-256 $ExeSha256."
