# Verify the explicit companion placement command through the downloaded EXE.
$placementHome=Join-Path $root 'companion-placement-live'
$placementDb=Join-Path $placementHome 'paperroom-data/notes.sqlite3'
$placementZip=Join-Path $root 'Placement.zip'
$placementBackup=Join-Path $root 'companion-placement-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $placementDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $placementHome 'Paperroom.exe')
@'
import json,sys,zipfile
with zipfile.ZipFile(sys.argv[1],"w",zipfile.ZIP_DEFLATED) as out:
    out.writestr("All-Data.json","Synthetic marker.")
    out.writestr("甲/鏡頭.md","# 鏡頭\n\n甲正文")
    out.writestr("乙/鏡頭.md","# 鏡頭\n\n乙正文")
    doc={"nodes":[{"id":"old-group","type":"group","x":-50,"y":-50,"width":550,"height":500,"label":"原有群組"},
      {"id":"old-file","type":"file","x":10,"y":15,"width":280,"height":120,"file":"甲/鏡頭.md"}],"edges":[]}
    out.writestr("Boards/Placement.canvas",json.dumps(doc))
'@ | python - $placementZip
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the ordinary standard Canvas and two same-title notes.' }
$placementZipHash=(Get-FileHash -LiteralPath $placementZip).Hash

function Open-PlacementCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'Boards/Placement.canvas' -MenuOpen
    $null=Wait-For 'the saved companion Canvas' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
    }
}
function Add-Placement([string]$Coordinates,[switch]$Cancel) {
    Native-Menu 120 14 '新增同包筆記節點…'
    Choose-CollectionRow '甲/鏡頭.md  (#'
    $dialog=Wait-For 'native geometry prompt' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','畫布位置與尺寸',1)
    }
    if ($Cancel) { Finish-Revision $dialog 2; return }
    Type-SearchFilter $dialog 101 $Coordinates
    Finish-Revision $dialog 1
    $null=Wait-For 'saved added node' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已新增同包筆記節點。'
    }
}
function Check-PlacementDatabase([string]$Db,[int]$Added) {
    @'
from contextlib import closing
from pathlib import Path
import json,sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    c,data=db.execute("select id,source from saved_canvases").fetchone()
    notes=json.loads(data)["nodes"]
    n=[x for x in notes if x["type"]=="file"]
    assert len(n)==1+int(sys.argv[2]) and all(x["file"]=="甲/鏡頭.md" for x in n)
    assert [x["x"] for x in n]==[10,70,185][:len(n)]
    assert notes[0]["label"]=="原有群組"
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
'@ | python - $Db $Added
    if ($LASTEXITCODE -ne 0) { throw 'Canvas geometry or original content differed.' }
}
function Open-PlacementSource([int]$Index) {
    Native-Menu 120 5 '開啟同包來源筆記…'
    $items=@(Collection-MenuLabels)
    if ($Index -gt $items.Count) { throw 'No native source link for the added placement.' }
    Native-Menu 120 $Index '甲/鏡頭.md' -MenuOpen -CaptionPrefix
    $null=Wait-For 'actual opened source' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲正文')
    }
}
$process=Start-Notebook $placementHome
$window=$process.MainWindowHandle
$confirm=Native-File 20 '匯入 Markdown ZIP（相對附件）…' $placementZip '' -NextDialog 'Markdown ZIP 匯入範圍' -NextDialogButton 6
Finish-Revision $confirm 6
$report=Wait-For 'original companion imported' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','匯入報告',1)
}
if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('保存 1 份可編輯 Canvas')) { throw 'The actual companion was not retained.' }
Finish-Revision $report 1
Open-PlacementCanvas
Add-Placement '70 200 240 114' -Cancel
Check-PlacementDatabase $placementDb 0
Add-Placement '70 200 240 114'
Add-Placement '185 360 240 114'
Check-PlacementDatabase $placementDb 2
Open-PlacementSource 3
Native-Menu 120 60002 '查看此筆記的 Canvas 位置…'
Native-Menu 120 1 'Boards/Placement.canvas  (#' -MenuOpen -CaptionPrefix
$locations=@(Collection-MenuLabels)
if ($locations.Count -ne 3) { throw 'The note does not list all three original and new positions.' }
Native-Menu 120 3 '3　甲/鏡頭.md' -MenuOpen -CaptionPrefix
$null=Wait-For 'native Canvas location from linked note' {
    [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('已定位 Canvas 節點')
}
Native-File 4 '備份工作區…' $placementBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restore=Join-Path $root 'companion-placement-restore'
$restoredDb=Join-Path $restore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
$backupHash=(Get-FileHash -LiteralPath $placementBackup).Hash
Copy-Item -LiteralPath $placementBackup -Destination $restoredDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restore 'Paperroom.exe')
$process=Start-Notebook $restore
$window=$process.MainWindowHandle
Check-PlacementDatabase $restoredDb 2
Open-PlacementCanvas
Open-PlacementSource 3
Open-PlacementCanvas
Native-Menu 120 12 '復原上一次 Canvas 修改'
$null=Wait-For 'Undo completed' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已復原上一次 Canvas 修改。'
}
Check-PlacementDatabase $restoredDb 1
Open-PlacementSource 2
Close-Notebook $process
$process.Dispose()
$process=$null
if ((Get-FileHash -LiteralPath $placementBackup).Hash -cne $backupHash -or
    (Get-FileHash -LiteralPath $placementZip).Hash -cne $placementZipHash) { throw 'An original file changed.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('close-geometry-dialog-no-write','same-batch-same-title-exact-identity','two-real-placements','forward-and-reverse-ui','isolated-restore','undo-linked-add','source-archives-unchanged')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: exact EXE adds two explicit same-note Canvas placements and can reopen and undo one after isolated restore.'

