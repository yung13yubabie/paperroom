# Native viewport and DB proof using the previously saved official Obsidian Canvas.
$dynamic=$env:FLOW -eq 'dynamic-bases'
$scenario=if ($dynamic) { 'dynamic' } else { 'official' }
$vendor=$env:PAPERROOM_OBSIDIAN_PINS
$assetRoot=Join-Path $root 'VendorVault'
$appRoot=Join-Path $root 'pinned-bases-live'
$dbPath=Join-Path $appRoot 'paperroom-data/notes.sqlite3'
$backupPath=Join-Path $root 'pinned-bases-backup.sqlite3'
$pinA='Pin_A_601'
$pinB='Pin_B_904'
$idA='dfc973b7cdfab317'
$idB='1b282d54e50ee300'
$expectedOriginal=@{
    'PinNative.canvas'='8a3047f3ca4411f74cec374b24b20d7227f91af1286bb91b3b96bf2f90347a92'
    'Shipping.base'='6e6c94be10902b32d5aa385766e80b7602a3d53d98070e88dd9d6f56c3e1e4f1'
    'Alpha-616.md'='fc6d86e4357578548c78c851a09f2993ba886f09ebed6e919ffdb3a470fe69ae'
    'Bravo-926.md'='a0262171915a649e6b90eb2fd3fc797f625cb67be082e19973c89252109fc6b8'
    'Default-307.md'='d302aeaf4dda2327d62c439c635a29941082abda367937d1f0728ac82922da4d'
}
foreach ($part in $expectedOriginal.Keys) {
    $p=Join-Path $vendor $part
    if ((Get-FileHash -LiteralPath $p).Hash -ine $expectedOriginal[$part]) { throw "Original Obsidian source failed SHA-256: $part" }
}
[void][IO.Directory]::CreateDirectory($assetRoot)
[void][IO.Directory]::CreateDirectory((Split-Path $dbPath -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $appRoot 'Paperroom.exe')
Copy-Item -LiteralPath (Join-Path $vendor 'PinNative.canvas') -Destination $assetRoot
if (!$dynamic) {
    foreach ($part in 'Shipping.base','Alpha-616.md','Bravo-926.md','Default-307.md') {
        Copy-Item -LiteralPath (Join-Path $vendor $part) -Destination $assetRoot
    }
} else {
    # The vendor Canvas stays byte-identical. This separate Base and note set is Paperroom test data.
    @'
from pathlib import Path
import sys
root=Path(sys.argv[1])
rows=[("A/TieA.md","true","2","Tie A canonical note."),
      ("B/Beta.md","false","2.00","Beta changes memberships."),
      ("C/TieC.md","true","2.0","Tie C canonical note."),
      ("D/High.md","true","10","High changes memberships."),
      ("E/Empty.md","null","0","Null is distinct from false.")]
for path,flag,cost,body in rows:
    p=root/path
    p.parent.mkdir(exist_ok=True)
    p.write_bytes(f"---\napproved: {flag}\ncost: {cost}\n---\n{body}".encode())
base="""formulas:
  ApprovedCheck: 'note.approved == true'
  CostCheck: 'note.cost >= 0'
properties:
  note.approved: {displayName: 核准}
  note.cost: {displayName: 工時}
views:
  - type: table
    name: Pin_A_601
    filters: 'note.approved == true'
    order: [file.path, note.approved, note.cost, formula.ApprovedCheck]
    sort: [{property: note.cost, direction: ASC}]
  - type: table
    name: Pin_B_904
    filters: 'note.approved == false'
    order: [file.path, note.approved, note.cost, formula.ApprovedCheck]
    sort: [{property: note.cost, direction: ASC}]
"""
(root/"Shipping.base").write_bytes(base.encode())
'@ | python - $assetRoot
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the separate typed Base fixture.' }
}
$canvasPath=Join-Path $assetRoot 'PinNative.canvas'
$basePath=Join-Path $assetRoot 'Shipping.base'
$sourceHashes=@{}
foreach ($source in Get-ChildItem -LiteralPath $assetRoot -File -Recurse) {
    $sourceHashes[$source.FullName]=(Get-FileHash -LiteralPath $source.FullName).Hash
}
function Open-PinCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'PinNative' -MenuOpen
    $null=Wait-For 'pinned Canvas fully open' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'PinNative' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
    }
}
function Read-PinThroughCanvas([string]$Name,[string]$ID,[string[]]$Rows,[switch]$Stay) {
    Open-PinCanvas
    Native-Menu 120 16 '預覽或開啟 Bases 具名檢視…'
    Choose-CollectionRow "Shipping.base#$Name 〔$ID〕"
    $preview=Wait-For "native preview of exact pin $Name" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Bases 卡片預覽',1)
    }
    $body=[PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$body.Contains($Name)) { throw "Wrong named preview: $body" }
    foreach ($row in $Rows) {
        if (!$body.Contains($row)) { throw "Pinned preview $Name omitted $row." }
    }
    Finish-Revision $preview 1
    if ($Stay) { Native-Menu 120 2 '留在畫布' -MenuOpen; return }
    Native-Menu 120 1 '開啟此具名檢視' -MenuOpen
    $null=Wait-For "named view $Name fully open" {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq $Name -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作'
    }
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    $options=@(Collection-MenuLabels)
    if ($options.Count -ne $Rows.Count) { throw "Wrong visible membership in $Name : $($options -join ' | ')." }
    for ($i=0;$i -lt $Rows.Count;$i++) {
        if (!$options[$i].Contains("　$($Rows[$i])  (")) {
            throw "Wrong stable sorted order in $Name : $($options -join ' | ')."
        }
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null=Wait-For 'native grouped row picker closed' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0) -eq [IntPtr]::Zero
    }
    Write-Host "Pin $Name ($ID) verified with exactly: $($Rows -join ', ')."
}
function Change-PinApproval([string]$Title,[string]$Expected,[string]$Next) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 6 '編輯連結的布林或數值…' -MenuOpen
    Choose-CollectionRow '核准'
    Choose-CollectionRow "$Title (#"
    Rule-Prompt '編輯工作區型別欄位' $Expected $Next
}
function Export-PinCanvas([string]$Destination) {
    Open-PinCanvas
    Native-Menu 120 1 '匯出原始 JSON Canvas…'
    Native-File 1 '匯出原始 JSON Canvas…' $Destination '已匯出原始 JSON Canvas。' -AlreadyOpened -CanvasSave
    if ((Get-FileHash -LiteralPath $Destination).Hash -ine $expectedOriginal['PinNative.canvas']) {
        throw 'The raw official Canvas was changed by binding or using native views.'
    }
}
function Read-PersistedPins([string]$Db,[string]$State,[string]$ExpectedId='') {
    $ids=@'
from pathlib import Path
from contextlib import closing
import json,sqlite3,sys
db, src, scenario, state, prior=sys.argv[1:]
with closing(sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True)) as c:
    cid,raw=c.execute("select id,source from saved_canvases where title='PinNative'").fetchone()
    assert raw.encode()==Path(src).read_bytes()
    bindings=c.execute("select import_root,file,receipt from canvas_base_paths where canvas_id=?",(cid,)).fetchall()
    if state=="unbound":
        assert bindings==[],bindings
        source=""
    else:
        root,file,receipt=bindings[0]
        assert len(bindings)==1 and file=="Shipping.base",bindings
        assert c.execute("select import_root from base_card_receipts where receipt=?",(receipt,)).fetchone()==(root,)
        named=dict(c.execute("select name,view_id from base_card_views where receipt=?",(receipt,)))
        assert named["Pin_A_601"]!=named["Pin_B_904"]
        source=f"/{root}:{receipt}:{named['Pin_A_601']}:{named['Pin_B_904']}"
    if scenario=="dynamic":
        note_ids=[]
        for path in ("A/TieA.md","B/Beta.md","C/TieC.md","D/High.md","E/Empty.md"):
            row=c.execute("select n.id,n.body,i.token from notes n join vault_paths p on p.note_id=n.id join note_link_identities i on i.note_id=n.id where p.relative_path=?",(path,)).fetchone()
            assert row
            active={ "B/Beta.md":"true" if state=="changed" else "false", "D/High.md":"false" if state=="changed" else "true"}
            if path in active: assert f"approved: {active[path]}"==row[1].splitlines()[1],row[1]
            note_ids.append(f"{row[0]}:{row[2]}")
        identity=(f"{cid}{source}/"+"/".join(note_ids))
    else: identity=f"{cid}{source}"
    assert not prior or identity==prior
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    print(identity)
'@ | python - $Db $canvasPath $scenario $State $ExpectedId
    if ($LASTEXITCODE -ne 0 -or !$ids) { throw 'Stored Canvas bytes or canonical row identities failed readback.' }
    return [string]$ids
}
function Select-BaseBindingConfirmation {
    Native-Menu 120 15 '手動配對 Bases 檔案來源…'
    Choose-CollectionRow "Shipping.base#$pinA 〔$idA〕"
    Choose-CollectionRow '來源收據 #'
    Wait-For 'explicit Base path binding' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認 Bases 來源路徑',7)
    }
}
$process=Start-Notebook $appRoot
$window=$process.MainWindowHandle
$expectedImport=if ($dynamic) { '匯入 5 篇筆記' } else { '匯入 3 篇筆記' }
Native-VaultFolder $assetRoot $expectedImport
if ($dynamic) { Open-LinkedFixture 'TieA' 'A' } else { Select-Note 'Alpha-616' }
Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
$confirm=Native-File 40 '匯入 Obsidian Bases 檢視…' $basePath '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $confirm 6
if ($dynamic) { Native-Menu 109 3 '動態篩選與排序（連結來源）' -MenuOpen }
$expectedViews=if ($dynamic) { 2 } else { 3 }
$null=Wait-For 'scoped Bases import completed' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq "已匯入 $expectedViews 個 Bases 檢視。"
}
Native-File 11 '匯入 JSON Canvas…' $canvasPath '中鍵拖曳平移'
Native-Menu 120 4 '綁定匯入的 Vault…'
Choose-CollectionRow '匯入：VendorVault  ('
$null=Wait-For 'correct Canvas Vault selected' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'
}
$confirmation=Select-BaseBindingConfirmation
Finish-Revision $confirmation 7
$null=Read-PersistedPins $dbPath 'unbound'
$confirmation=Select-BaseBindingConfirmation
if (![PaperroomHighlightGui]::DialogContains($confirmation,$process.Id,'Shipping.base')) { throw 'Pairing omitted the actual relative path.' }
Finish-Revision $confirmation 6
$null=Wait-For 'persisted exact pairing' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已保存這份畫布的 Bases 來源配對。'
}

$stable=Read-PersistedPins $dbPath 'original'
if ($dynamic) {
    Read-PinThroughCanvas $pinA $idA @('TieA','TieC','High')
    Read-PinThroughCanvas $pinB $idB @('Beta')
    Change-PinApproval 'Beta' 'false' 'true'
    Read-PinThroughCanvas $pinA $idA @('TieA','Beta','TieC','High')
    Change-PinApproval 'High' 'true' 'false'
    $setA=@('TieA','Beta','TieC')
    $setB=@('High')
} else { 
    $setA=@('Alpha-616')
    $setB=@('Bravo-926')
}
Read-PinThroughCanvas $pinA $idA $setA
Read-PinThroughCanvas $pinB $idB $setB
$null=Read-PersistedPins $dbPath 'changed' $stable
if (!$dynamic) {
    Open-PinCanvas
    Native-Menu 120 17 '撤銷 Bases 路徑配對…'
    Choose-CollectionRow "Shipping.base#$pinB 〔$idB〕"
    $revoke=Wait-For 'reversible source unbinding' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','撤銷 Bases 路徑配對',6)
    }
    Finish-Revision $revoke 6
    $null=Wait-For 'binding actually revoked' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已撤銷 Bases 路徑配對。'
    }
    $null=Read-PersistedPins $dbPath 'unbound'
    $confirmation=Select-BaseBindingConfirmation
    Finish-Revision $confirmation 6
    $null=Wait-For 'original exact source rebound' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已保存這份畫布的 Bases 來源配對。'
    }
    $null=Read-PersistedPins $dbPath 'changed' $stable
}
Export-PinCanvas (Join-Path $root 'native-export.canvas')
Native-File 4 '備份工作區…' $backupPath '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restored=Join-Path $root 'pinned-bases-restore'
$restoredDb=Join-Path $restored 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $backupPath -Destination $restoredDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restored 'Paperroom.exe')
foreach ($workspaceCase in @(@($appRoot,$dbPath),@($restored,$restoredDb))) {
    $process=Start-Notebook $workspaceCase[0]
    $window=$process.MainWindowHandle
    Read-PinThroughCanvas $pinA $idA $setA
    Read-PinThroughCanvas $pinB $idB $setB
    $null=Read-PersistedPins $workspaceCase[1] 'changed' $stable
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
foreach ($path in $sourceHashes.Keys) {
    if ((Get-FileHash -LiteralPath $path).Hash -cne $sourceHashes[$path]) { throw "External source was mutated: $path" }
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    variant=$scenario
    source_canvas_sha256=$expectedOriginal['PinNative.canvas']
    exact_expected_views=@{Pin_A_601=$setA; Pin_B_904=$setB}
    tested=@('official-node-ids-and-subpaths','exact-manual-receipt-mapping','two-independent-native-views','untouched-canvas-bytes','reopen','isolated-restore')
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: native $scenario named Bases pins, exact memberships, original Canvas preservation, reopening and backup restore."
