# Physical right click over each of two exact same-basename bundled assets.
$assetApp=Join-Path $root 'canvas-node-assets'
$assetDb=Join-Path $assetApp 'paperroom-data/notes.sqlite3'
$assetZip=Join-Path $root 'SelectedAssets.zip'
$backup=Join-Path $root 'node-assets-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $assetDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $assetApp 'Paperroom.exe')
@'
import json,struct,sys,zipfile,zlib
def chunk(tag,value):
    return struct.pack(">I",len(value))+tag+value+struct.pack(">I",zlib.crc32(tag+value))
def png(rgb):
    return b"\x89PNG\r\n\x1a\n"+chunk(b"IHDR",struct.pack(">IIBBBBB",16,16,8,2,0,0,0))+chunk(b"IDAT",zlib.compress((b"\0"+bytes(rgb)*16)*16))+chunk(b"IEND",b"")
doc={"nodes":[
    {"id":"file","type":"file","file":"甲/同名.png","x":20,"y":20,"width":220,"height":180},
    {"id":"group","type":"group","label":"群組背景","background":"乙/同名.png","backgroundStyle":"cover","x":300,"y":20,"width":240,"height":220}],
     "edges":[]}
with zipfile.ZipFile(sys.argv[1],"x") as z:
    z.writestr("All-Data.json","Synthetic marker.")
    z.writestr("Boards/Selected.canvas",json.dumps(doc,ensure_ascii=False))
    z.writestr("獨立筆記.md","# 一般筆記")
    z.writestr("甲/同名.png",png([30,139,211]))
    z.writestr("乙/同名.png",png([117,83,204]))
    z.writestr("Boards/甲/同名.png",png([213,92,37]))
'@ | python - $assetZip
if ($LASTEXITCODE -ne 0) { throw 'Could not create the real companion ZIP with distinct source images.' }
$inputSha=(Get-FileHash -LiteralPath $assetZip).Hash

function Open-NodeAssetCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'Boards/Selected.canvas' -MenuOpen
    $null=Wait-For 'editable companion Canvas ready' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Boards/Selected.canvas' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
    }
    Native-Menu 120 8 '將畫布調整至視窗'
}
function Save-ExactNodeAsset([bool]$Group,[string]$Destination) {
    $ink=if ($Group) {
        Wait-For 'visible bounded group image' { Read-CanvasInk 117 83 204 }
    } else {
        Wait-For 'visible file card image' { Read-CanvasInk 30 139 211 }
    }
    $x=$ink.ScreenX+$ink.L+[int]($ink.W/2)
    $offset=if ($Group) { 2 } else { [int]($ink.H/2) }
    $y=$ink.ScreenY+$ink.T+$offset
    [PaperroomHighlightGui]::RightCanvasClick($window,$process.Id,$x,$y)
    $command=if ($Group) { '儲存此群組的背景圖片…' } else { '儲存此節點的附件…' }
    $status=if ($Group) { '已另存群組背景的原始圖片。' } else { '已另存節點的原始附件。' }
    Native-Menu 120 1 $command -MenuOpen
    Native-File 1 $command $Destination $status -AlreadyOpened -SaveName '同名.png'
}
function Verify-NodeAssets([string]$Database,[string]$File,[string]$Group) {
    @'
import sys,zipfile,sqlite3
from pathlib import Path
db,source,card,group=map(Path,sys.argv[1:])
with zipfile.ZipFile(source) as z, sqlite3.connect(db.as_uri()+"?mode=ro",uri=True) as c:
    expected={p:z.read(p) for p in ["甲/同名.png","乙/同名.png"]}
    assert card.read_bytes()==expected["甲/同名.png"]!=z.read("Boards/甲/同名.png")
    assert group.read_bytes()==expected["乙/同名.png"]!=card.read_bytes()
    assert dict(c.execute("select path,bytes from canvas_bundle_assets"))==expected
    assert c.execute("select source from saved_canvases").fetchone()[0].encode()==z.read("Boards/Selected.canvas")
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
'@ | python - $Database $assetZip $File $Group
    if ($LASTEXITCODE -ne 0) { throw 'Physical UI export did not select the exact path for each node.' }
}
$process=Start-Notebook $assetApp
$window=$process.MainWindowHandle
$notice=Native-File 20 '匯入 Markdown ZIP（相對附件）…' $assetZip '' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $notice 6
$report=Wait-For 'native companion import report' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
Finish-Revision $report 1
Open-NodeAssetCanvas
$fileBefore=Join-Path $root 'file-before.png'
$groupBefore=Join-Path $root 'group-before.png'
Save-ExactNodeAsset $false $fileBefore
Save-ExactNodeAsset $true $groupBefore
Verify-NodeAssets $assetDb $fileBefore $groupBefore
Native-File 4 '備份工作區…' $backup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$assetRestore=Join-Path $root 'canvas-assets-restored'
$restoreDb=Join-Path $assetRestore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoreDb -Parent))
Copy-Item -LiteralPath $backup -Destination $restoreDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $assetRestore 'Paperroom.exe')
$process=Start-Notebook $assetRestore
$window=$process.MainWindowHandle
Open-NodeAssetCanvas
$fileAfter=Join-Path $root 'file-after.png'
$groupAfter=Join-Path $root 'group-after.png'
Save-ExactNodeAsset $false $fileAfter
Save-ExactNodeAsset $true $groupAfter
Verify-NodeAssets $restoreDb $fileAfter $groupAfter
Close-Notebook $process
$process.Dispose()
$process=$null
if ((Get-FileHash -LiteralPath $assetZip).Hash -cne $inputSha) { throw 'The original archive was touched.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('physical-file-and-group-context-menu','same-basename-different-bytes','decoy-excluded','exact-canvas-bytes','isolated-restore')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: per-node right-click saves exact same-basename but distinct bytes before and after isolated restore.'
