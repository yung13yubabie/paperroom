# Native Paperroom import of the exact Canvas written by official Obsidian.
$vendorVault=$env:PAPERROOM_OBSIDIAN_FIXTURE
$vendorCanvas=Join-Path $vendorVault '白板/Board.canvas'
$vendorApp=Join-Path $root 'official-obsidian-rich-canvas'
$exported=Join-Path $root 'rich-vendor-roundtrip.zip'
$vendorDb=Join-Path $vendorApp 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $vendorDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $vendorApp 'Paperroom.exe')
$expected=@{
    '白板/Board.canvas'='9ce82c87ea27672c6b4239d4a324d8d70c31978400afc6b64e66c2c9648d628f'
    '媒體/底圖-紫.png'='388de718a1a538f05167ef6170654d0be2961bad405b94a7edc174142020b037'
    '媒體/場景-青.png'='91f6a73cce6cfcc9cc4adddb65b6e451306c6c20a96af468f153d0c01fe507cd'
}
$original=@{}
foreach ($member in '白板/Board.canvas','媒體/場景-青.png','媒體/底圖-紫.png') {
    $file=Join-Path $vendorVault $member
    $original[$member]=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
    if ($original[$member] -ine $expected[$member]) { throw "Original Obsidian receipt member differs: $member." }
}

$process=Start-Notebook $vendorApp
$window=$process.MainWindowHandle
Native-MenuFromKeyboard 109 39 '從 Obsidian Vault 匯入 Canvas…'
Pick-NativeVaultFolder $vendorVault
$preview=Native-File 39 '從 Obsidian Vault 匯入 Canvas…' $vendorCanvas '' -AlreadyOpened -NextDialog '預覽 Obsidian Vault Canvas'
$text=[PaperroomHighlightGui]::Read((Control $preview 101))
foreach ($value in '白板/Board.canvas','將保存：媒體/場景-青.png','將保存：媒體/底圖-紫.png') {
    if (!$text.Contains($value)) { throw "Obsidian Vault preview omitted $value." }
}
Finish-Revision $preview 1
$null=Wait-For 'official Canvas imported with assets' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已從 Obsidian Vault 匯入 Canvas 與附件。'
}
Close-Notebook $process
$process.Dispose()
$process=$null
$process=Start-Notebook $vendorApp
$window=$process.MainWindowHandle
Native-Menu 109 12 '開啟已匯入的 Canvas…'
Native-Menu 109 1 'Board' -MenuOpen
$null=Wait-For 'same official Canvas after process restart' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
    [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Board'
}
Native-Menu 120 9 '匯出 JSON Canvas ZIP…'
Native-File 9 '匯出 JSON Canvas ZIP…' $exported '已匯出 JSON Canvas ZIP 及保存的附件。' -AlreadyOpened -SaveName 'canvas.zip'
$roundTrip=@'
from pathlib import Path
from zipfile import ZipFile
from hashlib import sha256
import json,sqlite3,sys
vault,archive,db=map(Path,sys.argv[1:])
src=(vault/"白板/Board.canvas").read_bytes()
value=json.loads(src)
assert len(value["edges"])==3
n={node["id"]:node for node in value["nodes"]}
group=n["1234567890abcde1"]
file=n["1234567890abcde2"]
assert group["type"]=="group" and group["background"]=="媒體/底圖-紫.png" and group["backgroundStyle"]=="cover"
assert file["type"]=="file" and file["file"]=="媒體/場景-青.png"
assert set(e["id"] for e in value["edges"])=={f"2234567890abcde{i}" for i in (1,2,3)}
assert all(e.get("label") and e.get("color") for e in value["edges"])
assert any(e["id"]=="2234567890abcde3" and e["label"]=="無箭頭參照｜已核對" and e["toEnd"]=="none" for e in value["edges"])
assets={name:(vault/name).read_bytes() for name in (group["background"],file["file"])}
with ZipFile(archive) as out:
    assert out.testzip() is None
    assert set(out.namelist())=={"Board.canvas",*assets}
    assert out.read("Board.canvas")==src
    assert {name:out.read(name) for name in assets}==assets
with sqlite3.connect(db.as_uri()+"?mode=ro",uri=True) as connection:
    assert dict(connection.execute("select path,bytes from canvas_bundle_assets"))==assets
    assert connection.execute("pragma integrity_check").fetchone()==("ok",)
print(json.dumps({"canvas_sha256":sha256(src).hexdigest(),"local_assets":{name:sha256(data).hexdigest() for name,data in assets.items()},
    "nodes":[dict(id=node["id"],x=node["x"],y=node["y"],w=node["width"],h=node["height"]) for node in n.values()],
    "edges":value["edges"]}))
'@ | python - $vendorVault $exported $vendorDb
if ($LASTEXITCODE -ne 0 -or !$roundTrip) { throw 'Paperroom rich Canvas round-trip did not preserve the official files.' }
foreach ($member in $original.Keys) {
    if ((Get-FileHash -LiteralPath (Join-Path $vendorVault $member) -Algorithm SHA256).Hash -cne $original[$member]) {
        throw "Paperroom modified the original $member."
    }
}
Close-Notebook $process
$process.Dispose()
$process=$null
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    vendor='Obsidian 1.15.1 Windows desktop'
    evidence=($roundTrip | ConvertFrom-Json -AsHashtable)
} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: official Obsidian rich Canvas, three complete edges, group background and exact local image bytes after Paperroom reopen/export.'
