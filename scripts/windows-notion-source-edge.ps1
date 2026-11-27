# Native linkage from an editable CSV row to an exact original Markdown page and attachment.
$notionHome=Join-Path $root 'notion-precise-source'
$notionZip=Join-Path $root 'NotionIdentity.zip'
$notionDb=Join-Path $notionHome 'paperroom-data/notes.sqlite3'
$notionBackup=Join-Path $root 'notion-source-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $notionDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $notionHome 'Paperroom.exe')
$folder='Pages 11111111111111111111111111111111'
$targetPath="$folder/乙/Scene 33333333333333333333333333333333.md"
@'
import sys,zipfile
from urllib.parse import quote
p='Pages 11111111111111111111111111111111'
a='Scene 22222222222222222222222222222222'
b='Scene 33333333333333333333333333333333'
with zipfile.ZipFile(sys.argv[1],"x") as z:
    z.writestr(p+".md","# 工作資料\n\n[精確甲](./"+quote(p+"/甲/"+a+".md")+")")
    z.writestr(f"{p}/甲/{a}.md","# 同名圖卡\n\n甲：另一份內容。\n\n[在地附件](local.txt)")
    z.writestr(f"{p}/乙/{b}.md","# 同名圖卡\n\n乙：待核對的正文。\n\n[在地附件](local.txt)")
    z.writestr(f"{p}/甲/local.txt","不同的甲附件。")
    z.writestr(f"{p}/乙/local.txt","乙原始附件：逐位元保留。")
    z.writestr("Shots.csv","項目,備註\r\n場景資料列,手動確認來源\r\n")
'@ | python - $notionZip
if ($LASTEXITCODE -ne 0) { throw 'Could not create structured synthetic Notion files.' }
$notionSourceHash=(Get-FileHash -LiteralPath $notionZip).Hash

function Assert-NotionExactEdge([string]$Database,[string]$State,[string]$Prior='') {
  $identity=@'
from pathlib import Path
from contextlib import closing
import sqlite3,sys,zipfile
db,arch,target,state,prior=sys.argv[1:]
with zipfile.ZipFile(arch) as z, closing(sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True)) as c:
    alias,alias_id=c.execute("select title,id from notes where title='場景資料列'").fetchone()
    target_id,token,body=c.execute("select n.id,i.token,n.body from notes n join note_link_identities i on i.note_id=n.id join notion_page_paths p on p.note_id=n.id where p.relative_path=?",(target,)).fetchone()
    assert "乙：待核對的正文" in body
    assert c.execute("""select content from attachments where note_id=? and name='local.txt'""",(target_id,)).fetchone()==(z.read(target.rsplit("/",1)[0]+"/local.txt"),)
    edges=c.execute("select name,target_id from note_relations where source_id=?",(alias_id,)).fetchall()
    assert edges==([("Notion Markdown 來源",target_id)] if state=="linked" else []),edges
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    identity=f"{alias_id}:{target_id}:{token}"
    assert not prior or identity==prior
    print(identity)
'@ | python - $Database $notionZip $targetPath $State $Prior
  if ($LASTEXITCODE -ne 0 -or !$identity) { throw 'The actual Notion row, page, attachment or stable relation differed.' }
  return [string]$identity
}
function Select-NotionSourceConfirmation {
    Select-Note '場景資料列'
    Native-Menu 120 60004 '連結已匯入的 Notion 來源頁…'
    Choose-CollectionRow '匯入：NotionIdentity  (#'
    Choose-CollectionRow "外層 ZIP／$targetPath"
    Wait-For 'explicit Notion source binding confirmation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','連結 Notion 來源頁',7)
    }
}
function Open-ConfirmedNotionSource([string]$Destination) {
    Select-Note '場景資料列'
    Native-Menu 120 60001 '具名關聯的來源與目標…'
    Native-Menu 120 2 '此筆記指向的具名關聯…' -MenuOpen
    Choose-CollectionRow 'Notion Markdown 來源　同名圖卡'
    $null=Wait-For 'specific same-title Notion page opened after traversing row relation' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '同名圖卡' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('乙：待核對的正文')
    }
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Native-Menu 113 1 'local.txt  (' -MenuOpen -CaptionPrefix
    Native-Menu 113 1 '儲存附件副本…' -MenuOpen
    Native-File 1 '儲存附件副本…' $Destination '附件副本已儲存。' -AlreadyOpened -SaveName 'local.txt'
    if ([IO.File]::ReadAllText($Destination) -cne '乙原始附件：逐位元保留。') {
        throw 'The actual attachment came from the wrong same-title Notion page.'
    }
}
$process=Start-Notebook $notionHome
$window=$process.MainWindowHandle
$report=Native-File 2 '匯入 Notion Markdown ZIP…' $notionZip '' -NextDialog '匯入報告'
Finish-Revision $report 1
Select-Note '1｜Shots.csv'
Native-Menu 113 8 '附件與 PDF 摘錄…'
Native-Menu 113 2 '管理附件…' -MenuOpen
Native-Menu 113 1 'Shots.csv  (' -MenuOpen -CaptionPrefix
Native-Menu 113 10 '從 Notion CSV 建立新集合…' -MenuOpen
Collection-Prompt '從 Notion CSV 建立新集合' '導演集合'
$null=Wait-For 'true CSV row was created' {
    [PaperroomHighlightGui]::Read((Control $window 116)).Contains('匯入 1 列')
}
$confirmation=Select-NotionSourceConfirmation
Finish-Revision $confirmation 7
$identity=Assert-NotionExactEdge $notionDb 'cancel'
$confirmation=Select-NotionSourceConfirmation
if (![PaperroomHighlightGui]::DialogContains($confirmation,$process.Id,$targetPath)) { throw 'Confirmed source omitted exact path.' }
Finish-Revision $confirmation 6
$null=Wait-For 'saved exact source binding' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已連結 Notion Markdown 來源頁。'
}
$null=Assert-NotionExactEdge $notionDb 'linked' $identity
if ($NotionReverseRows) {
    . (Join-Path $PSScriptRoot 'windows-notion-reverse-rows.ps1')
    Open-SortCollection '導演集合'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 1 '儲存新檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' '保留的來源檢視'
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '集合檢視'
    Collection-Prompt '集合檢視' '場景'
    Command $window 119
    $null=Wait-For 'left saved view before opening source' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板'
    }
}
Open-ConfirmedNotionSource (Join-Path $root 'local-before.txt')
if ($NotionReverseRows) {Test-NotionReverse32 $notionDb}
Native-File 4 '備份工作區…' $notionBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restore=Join-Path $root 'notion-precise-restore'
$restoreDb=Join-Path $restore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoreDb -Parent))
Copy-Item -LiteralPath $notionBackup -Destination $restoreDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restore 'Paperroom.exe')
foreach ($place in @(@($notionHome,$notionDb,'original'),@($restore,$restoreDb,'restored'))) {
    $process=Start-Notebook $place[0]
    $window=$process.MainWindowHandle
    $null=Assert-NotionExactEdge $place[1] 'linked' $identity
    Open-ConfirmedNotionSource (Join-Path $root ($place[2]+'.txt'))
    if ($NotionReverseRows) {Test-NotionReverse32 $place[1]}
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash -LiteralPath $notionZip).Hash -cne $notionSourceHash) { throw 'The original ZIP was modified.' }
$extraNotionChecks=@()
if ($NotionReverseRows) {$extraNotionChecks+=@('reverse exact CSV row and collection from imported page','same-title other page excluded','collection positioning on reopen and restore')}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('exact-relative-path-same-title','cancel-no-write','confirmed-stable-edge','correct-source-attachment-physical-save','reopen','isolated-restore')+$extraNotionChecks
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: manual CSV row repair opens the exact Notion page and attachment after isolated restore.'
