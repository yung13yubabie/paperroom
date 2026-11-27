# All writes to the synthetic workspace occur through the candidate's native UI.
$canvasPath=Join-Path $root 'SectionMap.canvas'
$canvasRaw=@'
{ "nodes": [
  {"id":"exact","type":"file","file":"A/鏡頭.md","label":"精確","subpath":"#舊小節","x":1.20e2,"y":40,"width":220,"height":130,"custom":{"幀":7}},
  {"id":"near","type":"file","file":"A/鏡頭.md","label":"相近","subpath":"#舊小節相似","x":470,"y":40,"width":220,"height":130},
  {"id":"decoy","type":"file","file":"B/鏡頭.md","label":"另一篇","subpath":"#舊小節","x":800,"y":40,"width":220,"height":130},
  {"id":"unresolved","type":"file","file":"鏡頭.md","label":"省略路徑","subpath":"#舊小節","x":560,"y":300,"width":220,"height":130},
  {"id":"duplicate","type":"file","file":"A/鏡頭.md","label":"重複標題","subpath":"#重名","x":40,"y":300,"width":220,"height":130}
 ], "edges":[{"id":"keep","fromNode":"near","toNode":"exact","foreign":3}], "vendor":"原始資料" }
'@
[IO.File]::WriteAllText($canvasPath,$canvasRaw,$utf8)
$sourceCanvasHash=(Get-FileHash -LiteralPath $canvasPath).Hash
function Open-HeadingCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Choose-CollectionRow 'SectionMap'
    $null=Wait-For 'fully opened exact heading Canvas' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'SectionMap' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
    }
}
function Choose-HeadingCard([string]$Label,[string]$Id) {
    Open-HeadingCanvas
    Native-Menu 120 5 '開啟畫布來源筆記…'
    Choose-CollectionRow "$Label 〔$Id〕"
}
function Confirm-HeadingExcerpt([string]$Anchor,[string]$Marker,[string[]]$Excluded) {
    $dialog=Wait-For "native excerpt for $Anchor" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',"#$Anchor",1)
    }
    $prose=[PaperroomHighlightGui]::Read((Control $dialog 101))
    if (!$prose.Contains($Marker)) {throw "Wrong destination for $Anchor : $prose"}
    foreach ($other in $Excluded) {if ($prose.Contains($other)) {throw "Wrong section boundary or note: $prose"}}
    Finish-Revision $dialog 1
}
function Test-HeadingCard([string]$Heading,[switch]$Selected) {
    if ($Selected) {
        Pick-HeadingSource
        Native-Menu 120 60002 '查看此筆記的 Canvas 位置…'
        Choose-CollectionRow 'SectionMap  (#'
        $rows=@(Collection-MenuLabels)
        $indexes=@(for ($i=0;$i -lt $rows.Count;$i++){if ($rows[$i].Contains('精確 〔exact〕')) {$i}})
        if ($indexes.Count -ne 1) {throw "Exact Canvas card is missing: $rows"}
        Native-Menu 113 ($indexes[0]+1) $rows[$indexes[0]] -MenuOpen
        $null=Wait-For 'exact card selected' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位 Canvas 節點「exact」。'}
        Native-Menu 120 13 '開啟已選節點的來源筆記'
    } else { Choose-HeadingCard '精確' 'exact' }
    Confirm-HeadingExcerpt $Heading '目標正文' @('相似段落','另一篇','歧義甲')
}
function Test-HeadingMarkdown([string]$Heading) {
    Open-LinkedFixture '入口' 'B'
    Native-Menu 120 2 "前往　原別名 #$Heading" -CaptionPrefix
    Confirm-HeadingExcerpt $Heading '目標正文' @('相似段落','另一篇','歧義甲')
}
function Test-HeadingCardBoundaries {
    Choose-HeadingCard '省略路徑' 'unresolved'
    $alert=Wait-For 'same-title note without path refused' {[PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')}
    if (![PaperroomHighlightGui]::DialogContains($alert,$process.Id,'精確路徑')) {throw 'Unexpected unresolved path refusal'}
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null=Wait-For 'unresolved file alert dismissed' {![PaperroomHighlightGui]::IsWindowVisible($alert)}
    Choose-HeadingCard '重複標題' 'duplicate'
    $null=Wait-For 'ambiguous heading is not silently resolved' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟來源筆記。這個標題連結有多個符合項目。'
    }
    Choose-HeadingCard '相近' 'near'
    Confirm-HeadingExcerpt '舊小節相似' '相似段落' @('目標正文','另一篇','歧義甲')
    Choose-HeadingCard '另一篇' 'decoy'
    Confirm-HeadingExcerpt '舊小節' '另一篇' @('目標正文','相似段落')
}
function Initialize-HeadingCanvas {
    Native-File 11 '匯入 JSON Canvas…' $canvasPath '中鍵拖曳平移'
    Native-Menu 120 4 '綁定匯入的 Vault…'
    Choose-CollectionRow '匯入：SyntheticHeadingVault  ('
    $null=Wait-For 'Canvas bound to the exact synthetic Vault' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'
    }
}

function Read-CanvasHeadingWitness([string]$Db,[string]$Heading) {
    $raw=@'
import json, sqlite3, sys
from pathlib import Path
from contextlib import closing
expected = json.loads(Path(sys.argv[2]).read_bytes())
nodes = [n for n in expected["nodes"] if n["id"] == "exact"]
assert len(nodes) == 1
nodes[0]["subpath"] = "#" + sys.argv[3]
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    rows = db.execute("""SELECT c.id,b.import_root,c.source FROM saved_canvases c
        JOIN canvas_vault_bindings b ON b.canvas_id=c.id
        JOIN notes n ON n.id=b.import_root WHERE c.title=? AND n.title=?""",
        ("SectionMap","匯入：SyntheticHeadingVault")).fetchall()
    assert len(rows)==1, "Missing unique bound working Canvas"
    canvas,root,source=rows[0]
    assert json.loads(source)==expected, "Unexpected Canvas change including nearby cards, edge or unknown metadata"
    if sys.argv[3]=="新小節":
        undo=db.execute("SELECT source FROM canvas_edit_undo WHERE canvas_id=?",(canvas,)).fetchone()
        assert undo and undo[0].encode("utf-8")==Path(sys.argv[2]).read_bytes(), "Prior raw Canvas differs"
    print(f"{canvas}/{root}")
'@ | python -B - $Db $canvasPath $Heading
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Canvas document or scoped binding differs'}
    return [string]$raw
}
