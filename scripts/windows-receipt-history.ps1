# Synthetic user-created Vault, Bases text and JSON Canvas. No vendor application required.
$vault=Join-Path $root 'ReceiptVault'
$base=Join-Path $vault 'Shipping.base'
$canvas=Join-Path $vault 'Review.canvas'
$asset=Join-Path $root 'Loose.txt'
$app=Join-Path $root 'receipt-app'
$restored=Join-Path $root 'receipt-restored'
$db=Join-Path $app 'paperroom-data/notes.sqlite3'
$backup=Join-Path $root 'receipt-before-reopen.sqlite'
$encoding=[Text.UTF8Encoding]::new($false)
[void][IO.Directory]::CreateDirectory($vault)
[IO.File]::WriteAllText((Join-Path $vault 'Seed.md'),'Receipt scope source',$encoding)
[IO.File]::WriteAllText($base,'views: [{type: table, name: Dispatch, order: [file.path]}]',$encoding)
[IO.File]::WriteAllText($canvas,'{"nodes":[{"id":"receipt-node","type":"file","file":"Shipping.base","subpath":"#Dispatch","x":40,"y":40,"width":220,"height":160}],"edges":[]}',$encoding)
[IO.File]::WriteAllText($asset,'Ordinary history attachment',$encoding)
foreach ($folder in $app,$restored) {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
}
function Receipt-State([string]$Database,[bool]$HasLoose) {
    $raw=@'
from contextlib import closing
from pathlib import Path
import hashlib,json,sqlite3,sys
db,base,canvas,asset,loose=sys.argv[1:]
with closing(sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True)) as c:
    c.execute("begin")
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    assert not c.execute("pragma foreign_key_check").fetchall()
    records=c.execute("""select r.receipt,r.import_root,r.attachment,a.content,v.view_id,p.canvas_id,s.source,i.token
      from base_card_receipts r join attachments a on a.id=r.attachment and a.note_id=r.receipt
      join base_card_views v on v.receipt=r.receipt
      join canvas_base_paths p on p.receipt=r.receipt and p.import_root=r.import_root
      join saved_canvases s on s.id=p.canvas_id join note_link_identities i on i.note_id=r.receipt
      where p.file='Shipping.base' and v.name='Dispatch'""").fetchall()
    assert len(records)==1,records
    receipt,root,attachment,content,view,board,document,token=records[0]
    assert content==Path(base).read_bytes() and document.encode()==Path(canvas).read_bytes()
    files=c.execute("select name,content from attachments where note_id=? order by name",(receipt,)).fetchall()
    expected=[("Shipping.base",content)]
    if loose=="True": expected.insert(0,("Loose.txt",Path(asset).read_bytes()))
    assert files==expected
    print(json.dumps(dict(note=receipt,root=root,asset=attachment,view=view,canvas=board,token=token,source_sha256=hashlib.sha256(content).hexdigest()),sort_keys=True))
'@ | python -B - $Database $base $canvas $asset $HasLoose
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Receipt, source bytes, named view, bound card or attachment set differs'}
    return [string]$raw
}
function Test-ReceiptPin {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Choose-CollectionRow 'Review'
    Native-Menu 120 16 '預覽或開啟 Bases 具名檢視…'
    Choose-CollectionRow 'Shipping.base#Dispatch 〔receipt-node〕'
    $preview=Wait-For 'named Bases preview after receipt operation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Bases 卡片預覽',1)
    }
    $body=[PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$body.Contains('Dispatch') -or !$body.Contains('Seed')) {throw "Wrong bound preview: $body"}
    Finish-Revision $preview 1
    Native-Menu 120 1 '開啟此具名檢視' -MenuOpen
    $null=Wait-For 'named view opened from the original card' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Dispatch' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作'
    }
}
$process=Start-Notebook $app
$window=$process.MainWindowHandle
Native-VaultFolder $vault '匯入 1 篇筆記'
Select-Note 'Seed'
Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
$confirm=Native-File 40 '匯入 Obsidian Bases 檢視…' $base '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
Finish-Revision $confirm 6
$null=Wait-For 'one receipt imported' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 1 個 Bases 檢視。'}
Native-File 11 '匯入 JSON Canvas…' $canvas '中鍵拖曳平移'
Native-Menu 120 4 '綁定匯入的 Vault…'
Choose-CollectionRow '匯入：ReceiptVault  ('
$null=Wait-For 'the imported Vault is bound' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'}
Native-Menu 120 15 '手動配對 Bases 檔案來源…'
Choose-CollectionRow 'Shipping.base#Dispatch 〔receipt-node〕'
Choose-CollectionRow '來源收據 #'
$confirm=Wait-For 'receipt pairing confirmation' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認 Bases 來源路徑',7)}
Finish-Revision $confirm 6
$null=Wait-For 'explicit binding saved' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已保存這份畫布的 Bases 來源配對。'}
$initial=Receipt-State $db $false
Select-Note 'Bases 來源：Shipping.base'
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 2 '管理附件…' -MenuOpen
Choose-CollectionRow 'Shipping.base  ('
Native-Menu 113 2 '刪除附件…' -MenuOpen
$alert=Wait-For 'immediate refusal when deleting a bound source' {[PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')}
if (![PaperroomHighlightGui]::DialogContains($alert,$process.Id,'Bases 來源收據')) {throw 'Unexpected source-protection explanation'}
[PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
$null=Wait-For 'bound-source refusal closed' {![PaperroomHighlightGui]::IsWindowVisible($alert)}
if ((Receipt-State $db $false) -cne $initial) {throw 'Declined source deletion changed the bound receipt'}
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 1 '加入本機附件…' -MenuOpen
Native-File 1 '加入本機附件…' $asset '附件已保存到本機工作區。' -AlreadyOpened
if ((Receipt-State $db $true) -cne $initial) {throw 'Ordinary attachment changed the bound source'}
$delta=Open-Revision 1
if ([PaperroomHighlightGui]::Read((Control $delta 1)) -cne '還原此版本') {throw 'Expected an applicable real revision'}
Finish-Revision $delta 1
$null=Wait-For 'ordinary history revision restored' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已還原選取的筆記版本。'}
if ((Receipt-State $db $false) -cne $initial) {throw 'Restoring the ordinary attachment altered bound identities'}
Test-ReceiptPin
Native-File 4 '備份工作區…' $backup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash $backup).Hash
Copy-Item $backup (Join-Path $restored 'paperroom-data/notes.sqlite3')
foreach ($folder in $app,$restored) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    Test-ReceiptPin
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    if ((Receipt-State (Join-Path $folder 'paperroom-data/notes.sqlite3') $false) -cne $initial) {throw 'Receipt changed after reopen/restore'}
}
if ((Get-FileHash $backup).Hash -cne $backupHash) {throw 'Isolated restore changed backup bytes'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupHash;identity=($initial | ConvertFrom-Json);scenarios=@('refused direct deletion of bound Bases source','restored ordinary attachment checkpoint','exact source bytes and view identities','Canvas named view after reopen and isolated restore')} |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
