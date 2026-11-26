# Exercise the native same-package Canvas image transport.
$compHome=Join-Path $root 'companion-assets-app'
$compDb=Join-Path $compHome 'paperroom-data/notes.sqlite3'
$compSource=Join-Path $root 'CompanionImages.zip'
$compBackup=Join-Path $root 'companion-images-backup.sqlite3'
$compExport=Join-Path $root 'companion-portable-canvas.zip'
$compRestored=Join-Path $root 'companion-restored-canvas.zip'
[void][IO.Directory]::CreateDirectory((Split-Path $compDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $compHome 'Paperroom.exe')
@'
import json,struct,sys,zipfile,zlib
def chunk(tag,body):
    return struct.pack(">I",len(body))+tag+body+struct.pack(">I",zlib.crc32(tag+body))
def png(rgb):
    return b"\x89PNG\r\n\x1a\n"+chunk(b"IHDR",struct.pack(">IIBBBBB",16,16,8,2,0,0,0))+chunk(b"IDAT",zlib.compress((b"\0"+bytes(rgb)*16)*16))+chunk(b"IEND",b"")
document={
    "nodes":[
        {"id":"g","type":"group","x":0,"y":0,"width":760,"height":320,"label":"工作區",
            "background":"媒體/背景.png","backgroundStyle":"cover"},
        {"id":"image","type":"file","x":70,"y":70,"width":215,"height":145,"file":"媒體/劇照.png"},
        {"id":"note","type":"file","x":415,"y":70,"width":215,"height":145,"file":"筆記/導演.md"}],
    "edges":[{"id":"e","fromNode":"note","fromSide":"left","fromEnd":"none","toNode":"image","toSide":"right",
        "toEnd":"arrow","color":"4","label":"確認來源"}]}
with zipfile.ZipFile(sys.argv[1],"x") as z:
    z.writestr("All-Data.json","Synthetic marker only.")
    z.writestr("筆記/導演.md","# 導演\n\n同包唯一筆記正文")
    z.writestr("Boards/Images.canvas",json.dumps(document,ensure_ascii=False))
    z.writestr("媒體/劇照.png",png([49,125,181]))
    z.writestr("媒體/背景.png",png([113,171,79]))
    z.writestr("Boards/媒體/劇照.png",png([195,33,100]))
'@ | python - $compSource
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the companion image fixture.' }
$compSourceHash=(Get-FileHash -LiteralPath $compSource).Hash

function View-CompanionImages {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'Boards/Images.canvas' -MenuOpen
    $null=Wait-For 'companion image Canvas fully open' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Boards/Images.canvas'
    }
    Native-Menu 120 10 '檢視附件與相容性'
    $report=Wait-For 'native image compatibility report' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Canvas ZIP 附件與相容性',1)
    }
    $body=[PaperroomHighlightGui]::Read((Control $report 101))
    foreach ($name in '媒體/背景.png','媒體/劇照.png') {
        if (!$body.Contains("已保存：$name")) { throw "The companion Canvas did not save $name." }
    }
    Finish-Revision $report 1
    $null=Wait-For 'true source foreground image is rendered' { Read-CanvasInk 49 125 181 }
    $null=Wait-For 'true source background image is rendered' { Read-CanvasInk 113 171 79 }
}
function Export-CompanionImages([string]$Destination) {
    Native-Menu 120 9 '匯出 JSON Canvas ZIP…'
    Native-File 9 '匯出 JSON Canvas ZIP…' $Destination '已匯出 JSON Canvas ZIP 及保存的附件。' -AlreadyOpened -SaveName 'canvas.zip'
    @'
import hashlib,sqlite3,sys,zipfile
from pathlib import Path
with zipfile.ZipFile(sys.argv[1]) as original, zipfile.ZipFile(sys.argv[2]) as result:
    assert result.testzip() is None
    members=set(result.namelist())
    assert members=={"Images.canvas","媒體/劇照.png","媒體/背景.png"},members
    assert result.read("Images.canvas")==original.read("Boards/Images.canvas")
    expected={name:original.read(name) for name in ("媒體/劇照.png","媒體/背景.png")}
    for name,asset in expected.items():
        assert result.read(name)==asset and asset!=original.read("Boards/媒體/劇照.png")
    with sqlite3.connect(Path(sys.argv[3]).as_uri()+"?mode=ro",uri=True) as db:
        actual=dict(db.execute("select path,bytes from canvas_bundle_assets"))
        assert expected==actual,(expected.keys(),actual.keys())
        assert db.execute("pragma integrity_check").fetchone()==("ok",)
    print("Original standard Canvas bytes and 2 exact image members verified.")
'@ | python - $compSource $Destination $compDb
    if ($LASTEXITCODE -ne 0) { throw 'Companion image ZIP contents did not match the selected source.' }
}

$process=Start-Notebook $compHome
$window=$process.MainWindowHandle
$notice=Native-File 20 '匯入 Markdown ZIP（相對附件）…' $compSource '' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $notice 6
$report=Wait-For 'companion archive report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('保存 1 份可編輯 Canvas')) { throw 'Editable companion Canvas not imported.' }
Finish-Revision $report 1
View-CompanionImages
Export-CompanionImages $compExport
Native-Menu 120 5 '開啟同包來源筆記…'
$items=@(Collection-MenuLabels)
$choices=@(for ($i=0;$i -lt $items.Count;$i++) { if ($items[$i].StartsWith('筆記/導演.md')) { $i+1 } })
if ($choices.Count -ne 1) { throw 'Image transport lost or duplicated the companion note binding.' }
Native-Menu 120 $choices[0] '筆記/導演.md 〔note〕' -MenuOpen -CaptionPrefix
$null=Wait-For 'companion note opened' {
    [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '導演' -and
    [PaperroomHighlightGui]::Read((Control $window 121)).Contains('同包唯一筆記正文')
}
Native-File 4 '備份工作區…' $compBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$compRestore=Join-Path $root 'companion-assets-restore'
$compDb=Join-Path $compRestore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $compDb -Parent))
Copy-Item -LiteralPath $compBackup -Destination $compDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $compRestore 'Paperroom.exe')
$process=Start-Notebook $compRestore
$window=$process.MainWindowHandle
View-CompanionImages
Export-CompanionImages $compRestored
Close-Notebook $process
$process.Dispose()
$process=$null
if ((Get-FileHash -LiteralPath $compSource).Hash -cne $compSourceHash) { throw 'The archive changed during import.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('editable-companion-canvas','exact-root-file-image','exact-root-group-background','same-folder-decoy-excluded','live-render','canonical-note-return','portable-zip','isolated-restore')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: original companion Canvas and two exact image members survive ZIP export and isolated restore.'
