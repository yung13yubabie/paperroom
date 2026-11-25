param()

$attachmentHome = Join-Path $root 'loose-canvas-workspace'
$destinationHome = Join-Path $root 'loose-canvas-destination'
$sourceFolder = Join-Path $root 'LooseCanvasSource'
$sourcePath = Join-Path $sourceFolder 'LooseCanvas.canvas'
$zipPath = Join-Path $root 'LooseCanvas.zip'
$liveCanvasDb = Join-Path $attachmentHome 'paperroom-data/notes.sqlite3'
$reimportedCanvasDb = Join-Path $destinationHome 'paperroom-data/notes.sqlite3'
foreach ($folder in @($attachmentHome,$destinationHome,$sourceFolder)) {
    [void][IO.Directory]::CreateDirectory($folder)
}
foreach ($folder in @($attachmentHome,$destinationHome)) {
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
@'
import json, struct, sys, zlib
from pathlib import Path
root = Path(sys.argv[1])
media = root / "繁中 附件"
media.mkdir()
def solid_png(path, rgb):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = (b"\0" + bytes(rgb) * 16) * 16
    path.write_bytes(b"\x89PNG\r\n\x1a\n" +
                     chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0)) +
                     chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
solid_png(media / "封面.png", (172, 67, 133))
solid_png(media / "圖片卡.png", (41, 131, 196))
outside = root.parent / "outside-synthetic"
outside.mkdir()
solid_png(outside / "never-copy.png", (4, 32, 61))
body = {
    "nodes": [
        {"id":"group","type":"group","label":"合成故事板","x":-220,"y":-120,"width":400,"height":380,
         "background":"繁中 附件/封面.png","backgroundStyle":"cover"},
        {"id":"image","type":"file","x":-140,"y":-45,"width":200,"height":150,"file":"繁中 附件/圖片卡.png"},
        {"id":"text","type":"text","x":250,"y":-45,"width":140,"height":100,"text":"圖卡旁的備註"},
        {"id":"missing","type":"file","x":250,"y":75,"width":140,"height":65,"file":"繁中 附件/缺件.png"},
        {"id":"outside","type":"file","x":250,"y":155,"width":140,"height":65,"file":"../outside-synthetic/never-copy.png"},
        {"id":"junction","type":"file","x":250,"y":315,"width":140,"height":65,"file":"junction/never-copy.png"},
        {"id":"link","type":"link","x":250,"y":230,"width":140,"height":65,"url":"https://example.invalid"}
    ],
    "edges": [
        {"id":"arrow","fromNode":"image","fromSide":"right","fromEnd":"none",
         "toNode":"text","toSide":"left","toEnd":"arrow","label":"讀圖到文字","vendorExtra":{"test":True}}
    ]
}
(root / "LooseCanvas.canvas").write_text(json.dumps(body, ensure_ascii=False), encoding="utf-8")
'@ | python - $sourceFolder
if ($LASTEXITCODE -ne 0) { throw 'Failed to generate the isolated loose Canvas fixture.' }
$null = New-Item -ItemType Junction -Path (Join-Path $sourceFolder 'junction') -Target (Join-Path $root 'outside-synthetic')

function Canvas-ScopeSnapshot([string]$Database, [string]$Expected, [string]$Zip = '') {
    @'
from contextlib import closing
from pathlib import Path, PurePosixPath
import sqlite3, sys, zipfile
db_path, source, expected = map(str, sys.argv[1:4])
source = Path(source)
with closing(sqlite3.connect(Path(db_path).as_uri() + "?mode=ro", uri=True)) as db:
    rows = db.execute("SELECT title,source FROM saved_canvases").fetchall()
    assets = dict(db.execute("SELECT path,bytes FROM canvas_bundle_assets").fetchall())
    bundles = db.execute("SELECT canvas_path FROM canvas_bundles").fetchall()
if expected == "cancel":
    assert rows == [] and assets == {} and bundles == [], (rows, assets, bundles)
else:
    raw = source.read_bytes()
    expected_assets = {name: source.parent.joinpath(PurePosixPath(name)).read_bytes() for name in
                       ("繁中 附件/封面.png", "繁中 附件/圖片卡.png")}
    assert rows == [("LooseCanvas", raw.decode("utf-8"))], rows
    assert bundles == [("LooseCanvas.canvas",)] and assets == expected_assets
    if len(sys.argv) > 4 and sys.argv[4]:
        with zipfile.ZipFile(sys.argv[4]) as archive:
            assert set(archive.namelist()) == set(expected_assets) | {source.name}
            assert archive.read(source.name) == raw
            for name, data in expected_assets.items():
                assert archive.read(name) == data
'@ | python - $Database $sourcePath $Expected $Zip
    if ($LASTEXITCODE -ne 0) { throw "Canvas state or ZIP bytes were different at stage $Expected." }
}
function Canvas-LoosePreview {
    Native-File 38 '匯入 JSON Canvas 與同資料夾附件…' $sourcePath '' -NextDialog '預覽 Canvas 與同資料夾附件'
}
function Canvas-LocalPaint {
    $null = Wait-For 'rendered PNG pixels on the native group background' { Read-CanvasInk 172 67 133 }
    $null = Wait-For 'rendered PNG pixels on the native file card' { Read-CanvasInk 41 131 196 }
}

$originalHash = (Get-FileHash -LiteralPath $sourcePath).Hash
$originalMedia = Get-ChildItem -LiteralPath (Join-Path $sourceFolder '繁中 附件') -File | Get-FileHash
$process = Start-Notebook $attachmentHome
$window = $process.MainWindowHandle
$preview = Canvas-LoosePreview
$report = [PaperroomHighlightGui]::Read((Control $preview 101))
foreach ($line in @('將保存：繁中 附件/封面.png','將保存：繁中 附件/圖片卡.png','缺少：繁中 附件/缺件.png','無法安全讀取或不在匯入範圍：../outside-synthetic/never-copy.png',
                    '無法安全讀取或不在匯入範圍：junction/never-copy.png')) {
    if (!$report.Contains($line)) { throw "Preview omitted $line. Preview text: $report" }
}
Canvas-ScopeSnapshot $liveCanvasDb cancel
Finish-Revision $preview 2
Canvas-ScopeSnapshot $liveCanvasDb cancel
$preview = Canvas-LoosePreview
Finish-Revision $preview 1
$null = Wait-For 'loose JSON Canvas confirmed' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 Canvas 與同資料夾附件。'
}
Canvas-LocalPaint
Canvas-ScopeSnapshot $liveCanvasDb imported
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $attachmentHome
$window = $process.MainWindowHandle
Native-Menu 109 12 '開啟已匯入的 Canvas…'
Native-Menu 109 1 'LooseCanvas' -MenuOpen
Canvas-LocalPaint
Native-Menu 120 9 '匯出 JSON Canvas ZIP…'
Native-File 9 '匯出 JSON Canvas ZIP…' $zipPath '已匯出 JSON Canvas ZIP 及保存的附件。' -AlreadyOpened -SaveName 'canvas.zip'
Canvas-ScopeSnapshot $liveCanvasDb exported $zipPath
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $destinationHome
$window = $process.MainWindowHandle
$zipReport = Native-File 31 '匯入 JSON Canvas ZIP…' $zipPath '' -NextDialog 'Canvas ZIP 匯入報告'
Finish-Revision $zipReport 1
Canvas-LocalPaint
Canvas-ScopeSnapshot $reimportedCanvasDb reimported
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-FileHash -LiteralPath $sourcePath).Hash -cne $originalHash) { throw 'The original Canvas was edited.' }
foreach ($file in $originalMedia) {
    if ((Get-FileHash -LiteralPath $file.Path).Hash -cne $file.Hash) { throw 'An original Canvas image was edited.' }
}
Write-Host "PASS: loose Canvas preview and cancel, real native group and image pixels, raw geometry and directed edges, reopened source, portable ZIP and isolated reimport. EXE SHA-256 $ExeSha256."
