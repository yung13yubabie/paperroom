# Edit from a Canvas card and exercise sourced Vault membership in its original root.
$upstream=$env:PAPERROOM_OBSIDIAN_PINS
$sourceVault=Join-Path $root 'VaultScopeA'
$rivalVault=Join-Path $root 'VaultScopeB'
$app=Join-Path $root 'card-live-app'
$db=Join-Path $app 'paperroom-data/notes.sqlite3'
$backup=Join-Path $root 'card-live-backup.sqlite3'
$baseFile=Join-Path $sourceVault 'Shipping.base'
$canvasFile=Join-Path $sourceVault 'PinNative.canvas'
$canvasSha='8a3047f3ca4411f74cec374b24b20d7227f91af1286bb91b3b96bf2f90347a92'
foreach ($folder in (Split-Path $db -Parent),$sourceVault,$rivalVault) {
    [void][IO.Directory]::CreateDirectory($folder)
}
Copy-Item -LiteralPath $Exe -Destination (Join-Path $app 'Paperroom.exe')
Copy-Item -LiteralPath (Join-Path $upstream 'PinNative.canvas') -Destination $canvasFile
if ((Get-FileHash $canvasFile).Hash -ine $canvasSha) { throw 'Original Obsidian Canvas differs.' }
@'
from pathlib import Path
import sys
root,other=map(Path,sys.argv[1:])
def write(path,body):
    path.parent.mkdir(exist_ok=True,parents=True)
    path.write_bytes(body.encode("utf-8"))
for p,a,n in [
    ("Active/Tie-1.md","true","2"),
    ("Active/Tie-3.md","true","2.00"),
    ("Active/Flip.md","false","1"),
    ("Archive/Move.md","true","2.0"),
    ("Witness.md","false","4")]:
    write(root/p,f"---\napproved: {a}\ncost: {n}\n---\nStable source {p}.")
write(other/"Active/Tie-1.md","---\napproved: true\ncost: 0\n---\nOther imported Vault.")
base="""formulas:
  checked: 'note.approved == true'
  rank: 'note.cost >= 0'
views:
  - type: table
    name: Pin_A_601
    filters: {and: ['file.inFolder("Active")', 'note.approved == true']}
    order: [file.path, note.approved, note.cost, formula.checked]
    sort: [{property: note.cost, direction: ASC}]
  - type: table
    name: Pin_B_904
    filters: {and: ['file.inFolder("Active")', 'note.approved == false']}
    order: [file.path, note.approved, note.cost, formula.checked]
    sort: [{property: note.cost, direction: ASC}]
"""
write(root/"Shipping.base",base)
'@ | python - $sourceVault $rivalVault
if ($LASTEXITCODE -ne 0) { throw 'Could not create separate official-Canvas and editable-Base fixtures.' }
$baseSha=(Get-FileHash -LiteralPath $baseFile).Hash

function Open-ScopeCanvas {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Choose-CollectionRow 'PinNative'
    $null=Wait-For 'pinned Canvas is visibly active' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'PinNative' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作'
    }
}
function Select-ExactPin([uint32]$Id,[string]$Caption,[string]$Name) {
    $node=if ($Name -ceq 'Pin_A_601') { 'dfc973b7cdfab317' } else { '1b282d54e50ee300' }
    Native-Menu 120 $Id $Caption
    Choose-CollectionRow "Shipping.base#$Name 〔$node〕"
}
function Confirm-CanvasPinRows([string]$Name,[AllowEmptyCollection()][string[]]$Expected = @()) {
    Select-ExactPin 16 '預覽或開啟 Bases 具名檢視…' $Name
    $dialog=Wait-For "preview for $Name" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Bases 卡片預覽',1)
    }
    $text=[PaperroomHighlightGui]::Read((Control $dialog 101))
    if (!$text.Contains($Name) -or !$text.Contains("共 $(@($Expected).Count) 筆")) {
        throw "The pinned card $Name showed the wrong actual count: $text"
    }
    $rows=@([regex]::Matches($text,'(?m)^(.+?)  \(#[0-9]+\)\r?$') | ForEach-Object { $_.Groups[1].Value })
    if (($rows -join '|') -cne ($Expected -join '|')) { throw "Wrong order or membership for $Name : $text" }
    Finish-Revision $dialog 1
    Native-Menu 120 2 '留在畫布' -MenuOpen
    Write-Host "Verified native card $Name : $($rows -join ', ')."
}
function Write-CanvasFlag([string]$Name,[string]$Title,[string]$Path) {
    Native-Menu 120 8 '將畫布調整至視窗'
    $ink=Wait-For 'both physically painted native Base card frames' { Read-CanvasInk 23 98 78 }
    # The original fixture has two 400×400 cards at (-254,-218), (141,43), without edges.
    $cx=if ($Name -ceq 'Pin_A_601') { 200 } else { 595 }
    $cy=if ($Name -ceq 'Pin_A_601') { 200 } else { 461 }
    $x=$ink.ScreenX+$ink.L+[int]($ink.W*$cx/795)
    $y=$ink.ScreenY+$ink.T+[int]($ink.H*$cy/661)
    [PaperroomHighlightGui]::RightCanvasClick($window,$process.Id,$x,$y)
    Native-Menu 120 1 '編輯 Bases 卡片欄位…' -MenuOpen
    $labels=@(Collection-MenuLabels)
    $matching=@(for ($i=0;$i -lt $labels.Count;$i++) {
        if ($labels[$i].StartsWith("$Title  (#",[StringComparison]::Ordinal) -and $labels[$i].EndsWith($Path,[StringComparison]::Ordinal)) { $i+1 }
    })
    if ($matching.Count -ne 1) { throw "Could not identify exact editable Canvas row: $($labels -join ' | ')" }
    Native-Menu 120 $matching[0] "$Title  (#" -MenuOpen -CaptionPrefix
    Choose-CollectionRow 'approved  [boolean]　false'
    Rule-Prompt '編輯 Bases 卡片欄位' 'false' 'true'
    $null=Wait-For "saved inline edit on $Name, still in the Canvas" {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'PinNative' -and
        [PaperroomHighlightGui]::Read((Control $window 116)).Contains('approved = true')
    }
}
function Adopt-AddedMarkdown([switch]$Cancel) {
    Select-Note 'Witness'
    Native-MenuFromKeyboard 109 42 '從同一 Vault 加入新 Markdown…'
    Pick-NativeVaultFolder $sourceVault
    Collection-Prompt '加入 Vault 新來源' 'Active/New.md'
    $confirm=Wait-For 'explicit admission confirmation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認加入 Vault 新來源',7)
    }
    if ($Cancel) { Finish-Revision $confirm 7; return }
    Finish-Revision $confirm 6
    $null=Wait-For 'verified file admitted through actual native UI' {
        [PaperroomHighlightGui]::Read((Control $window 116)).Contains('已建立新來源筆記')
    }
}
function Reconcile-PhysicalMove([string]$New,[string]$BackupName) {
    Select-Note 'Move'
    Native-Menu 109 23 '命令面板…'
    Collection-Prompt '命令面板' '核對 Vault 外部改名或搬移'
    Native-Menu 109 1 '核對 Vault 外部改名或搬移' -MenuOpen
    Pick-NativeVaultFolder $sourceVault
    Collection-Prompt '新的 Vault 來源路徑' $New
    $dialog=Wait-For 'explicit move preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Vault 外部改名或搬移預覽',1)
    }
    Finish-Revision $dialog 1
    $confirm=Wait-For 'explicit move confirmation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認配對新的 Vault 來源',6)
    }
    Finish-Revision $confirm 6
    $destination=Join-Path $root $BackupName
    Native-File 1 'Vault 來源搬移備份' $destination "已配對 Vault 來源：$New" -AlreadyOpened -SaveName 'vault-before-relocation.sqlite'
}
function Check-ScopedWorkspace([string]$Database,[string]$Path,[int]$HasNew,[int]$Trashed,[string]$Old='') {
    $identity=@'
import sqlite3,sys
from pathlib import Path
database,path,has_new,trashed,old,canvas,base=sys.argv[1:]
with sqlite3.connect(Path(database).as_uri()+"?mode=ro",uri=True) as c:
    root=c.execute("select id from notes where title='匯入：VaultScopeA'").fetchone()[0]
    other=c.execute("select id from notes where title='匯入：VaultScopeB'").fetchone()[0]
    stmt="""select n.id,i.token,n.body,n.trash_root from notes n join note_link_identities i on i.note_id=n.id
          join vault_paths p on p.note_id=n.id where p.import_root=? and p.relative_path=?"""
    moved=c.execute(stmt,(root,path)).fetchone(); assert moved is not None
    assert c.execute("select count(*) from notes where title='Move'").fetchone()==(1,)
    flip=c.execute(stmt,(root,"Active/Flip.md")).fetchone()
    assert bool(flip[3])==bool(int(trashed))
    assert "cost: 0" in c.execute(stmt,(other,"Active/Tie-1.md")).fetchone()[2]
    new=c.execute(stmt,(root,"Active/New.md")).fetchall()
    assert len(new)==int(has_new),new
    assert c.execute("select source from saved_canvases where title='PinNative'").fetchone()[0].encode()==Path(canvas).read_bytes()
    rec=c.execute("select a.content from attachments a join base_card_receipts b on b.attachment=a.id where b.import_root=?",(root,)).fetchone()[0]
    assert rec==Path(base).read_bytes()
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    identity=f"{root}:{moved[0]}:{moved[1]}:{flip[0]}:{flip[1]}"
    assert not old or old==identity
    print(identity)
'@ | python - $Database $Path $HasNew $Trashed $Old $canvasFile $baseFile
    if ($LASTEXITCODE -ne 0 -or !$identity) { throw 'Workspace scope, unique identity or original bytes differed.' }
    return [string]$identity
}

$process=Start-Notebook $app
$window=$process.MainWindowHandle
Native-VaultFolder $rivalVault '匯入 1 篇筆記'
Native-VaultFolder $sourceVault '匯入 5 篇筆記'
Select-Note 'Witness'
Native-MenuFromKeyboard 109 41 '匯入隨 Vault 來源變動的 Bases…'
$confirm=Native-File 41 '匯入隨 Vault 來源變動的 Bases…' $baseFile '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $confirm 6
$null=Wait-For 'live-scope Base imported' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 2 個 Bases 檢視。'
}
Native-File 11 '匯入 JSON Canvas…' $canvasFile '中鍵拖曳平移'
Native-Menu 120 4 '綁定匯入的 Vault…'
Choose-CollectionRow '匯入：VaultScopeA  ('
$null=Wait-For 'correct scope selected' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'
}
Select-ExactPin 15 '手動配對 Bases 檔案來源…' 'Pin_A_601'
Choose-CollectionRow '來源收據 #'
$bind=Wait-For 'scope Base pairing' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認 Bases 來源路徑',6)
}
Finish-Revision $bind 6
$null=Wait-For 'explicit scope binding available' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已保存這份畫布的 Bases 來源配對。'
}
$id=Check-ScopedWorkspace $db 'Archive/Move.md' 0 0
Confirm-CanvasPinRows 'Pin_A_601' @('Tie-1','Tie-3')
Confirm-CanvasPinRows 'Pin_B_904' @('Flip')

$newPath=Join-Path $sourceVault 'Active/New.md'
$lines=@('---','approved: false','cost: 2','---','A newly created source.')
[IO.File]::WriteAllText($newPath,($lines -join [char]10),[Text.UTF8Encoding]::new($false))
Adopt-AddedMarkdown -Cancel
$null=Check-ScopedWorkspace $db 'Archive/Move.md' 0 0 $id
Adopt-AddedMarkdown
$newHash=(Get-FileHash $newPath).Hash
$null=Check-ScopedWorkspace $db 'Archive/Move.md' 1 0 $id
Open-ScopeCanvas
Confirm-CanvasPinRows 'Pin_B_904' @('Flip','New')

$originalMoveHash=(Get-FileHash (Join-Path $sourceVault 'Archive/Move.md')).Hash
Move-Item -LiteralPath (Join-Path $sourceVault 'Archive/Move.md') -Destination (Join-Path $sourceVault 'Active/Z-Move.md')
Reconcile-PhysicalMove 'Active/Z-Move.md' 'move-before.sqlite'
Open-ScopeCanvas
Confirm-CanvasPinRows 'Pin_A_601' @('Tie-1','Tie-3','Move')
$null=Check-ScopedWorkspace $db 'Active/Z-Move.md' 1 0 $id
Move-Item -LiteralPath (Join-Path $sourceVault 'Active/Z-Move.md') -Destination (Join-Path $sourceVault 'Active/A-Move.md')
Reconcile-PhysicalMove 'Active/A-Move.md' 'rename-before.sqlite'
Open-ScopeCanvas
Confirm-CanvasPinRows 'Pin_A_601' @('Move','Tie-1','Tie-3')
$null=Check-ScopedWorkspace $db 'Active/A-Move.md' 1 0 $id

Write-CanvasFlag 'Pin_B_904' 'Flip' 'Active/Flip.md'
Confirm-CanvasPinRows 'Pin_A_601' @('Flip','Move','Tie-1','Tie-3')
Confirm-CanvasPinRows 'Pin_B_904' @('New')
Write-CanvasFlag 'Pin_B_904' 'New' 'Active/New.md'
Confirm-CanvasPinRows 'Pin_A_601' @('Flip','Move','New','Tie-1','Tie-3')
Confirm-CanvasPinRows 'Pin_B_904' @()

Select-Note 'Flip'
Native-Menu 113 3 '移到垃圾桶'
$trash=Wait-For 'native trash confirmation' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'移到垃圾桶')
}
Finish-Revision $trash 1
Open-ScopeCanvas
Confirm-CanvasPinRows 'Pin_A_601' @('Move','New','Tie-1','Tie-3')
Confirm-CanvasPinRows 'Pin_B_904' @()
$null=Check-ScopedWorkspace $db 'Active/A-Move.md' 1 1 $id
Native-File 4 '備份工作區…' $backup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restored=Join-Path $root 'card-live-restore'
$restoredDb=Join-Path $restored 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $backup -Destination $restoredDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restored 'Paperroom.exe')
foreach ($case in @(@($app,$db),@($restored,$restoredDb))) {
    $process=Start-Notebook $case[0]
    $window=$process.MainWindowHandle
    Open-ScopeCanvas
    Confirm-CanvasPinRows 'Pin_A_601' @('Move','New','Tie-1','Tie-3')
    Confirm-CanvasPinRows 'Pin_B_904' @()
    $null=Check-ScopedWorkspace $case[1] 'Active/A-Move.md' 1 1 $id
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash $canvasFile).Hash -ine $canvasSha -or (Get-FileHash $baseFile).Hash -cne $baseSha -or
    (Get-FileHash $newPath).Hash -cne $newHash -or
    (Get-FileHash (Join-Path $sourceVault 'Active/A-Move.md')).Hash -cne $originalMoveHash) {
    throw 'An original external file was written by Paperroom.'
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    source_canvas_sha256=$canvasSha
    scopes=@('real-canvas-node-identity','edit-from-canvas-no-view-switch','both-live-pins-refresh','source-aware-admission','same-name-other-vault-excluded','native-move','native-rename-changes-tie-order','no-duplicate-id','trash','reopen','isolated-restore')
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: actual inline Canvas editing and Vault source admission, move, rename, deletion and isolated restore.'
