# Reuse the actual Windows attachment, typed-field, and backup UI.
$typedHome=Join-Path $root 'notion-typed-live'
$typedDb=Join-Path $typedHome 'paperroom-data/notes.sqlite3'
$typedZip=Join-Path $root 'NotionTyped.zip'
$typedBackup=Join-Path $root 'notion-typed-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $typedDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $typedHome 'Paperroom.exe')
@'
import sys,zipfile
with zipfile.ZipFile(sys.argv[1],"x") as out:
    out.writestr("任務.md","# 專用合成任務\n")
    out.writestr("A.csv","項目,工時,核准,期限,編號\r\n零值,0,false,2028-02-29,00042\r\n空值,,,,00007\r\n")
    out.writestr("B.csv","項目,工時,核准,期限\r\n第一列,8,true,2028-02-29\r\n無效列,7,maybe,2028-02-29\r\n")
'@ | python - $typedZip
if ($LASTEXITCODE -ne 0) { throw 'Cannot build synthetic Notion ZIP.' }
$archiveHash=(Get-FileHash -LiteralPath $typedZip).Hash

function Declare-TypedNotionField([string]$Key,[int]$CommandId,[string]$Caption) {
    Native-Menu 109 14 '屬性型別…'
    Native-Menu 109 1 '設定屬性型別…' -MenuOpen
    Rule-Prompt '設定工作區屬性型別' '' $Key
    Native-Menu 109 $CommandId $Caption -MenuOpen
    $null=Wait-For "workspace field $Key defined" {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新工作區屬性型別，原有筆記值已保留。'
    }
}
function Choose-TypedNotionAttachment([string]$Title,[string]$Name) {
    Select-Note $Title
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Native-Menu 113 1 "$Name  (" -MenuOpen -CaptionPrefix
    Native-Menu 113 11 '依工作區欄位型別建立集合…' -MenuOpen
}
function Read-TypedNotionState([string]$Db,[string]$Phase,[string]$Canonical='') {
    $identity=@'
from contextlib import closing
from pathlib import Path
import sqlite3,sys,zipfile
path, archive, phase, canonical = sys.argv[1:]
with zipfile.ZipFile(archive) as original, closing(sqlite3.connect(Path(path).as_uri()+"?mode=ro",uri=True)) as db:
    attachments=db.execute("select id,note_id,name,content from attachments where name in ('A.csv','B.csv') order by name").fetchall()
    assert [(name,body) for _,_,name,body in attachments] == [(name,original.read(name)) for name in ("A.csv","B.csv")]
    assert db.execute("select key,kind from property_definitions").fetchall() and dict(db.execute("select key,kind from property_definitions"))=={"核准":"checkbox","工時":"number","期限":"date"}
    names=db.execute("select id,name from collections").fetchall()
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    if phase in {"cancel","refusal"}:
        assert not names and db.execute("select count(*) from collection_notes").fetchone()==(0,)
        assert db.execute("select count(*) from note_properties").fetchone()==(0,)
        assert not db.execute("select id from notes where title in ('零值','空值','第一列','無效列')").fetchall()
        print("no-write")
    else:
        assert len(names)==1 and names[0][1]=="型別測試集合",names
        rows=db.execute("""select n.id,n.title,parent.parent,i.token from collection_notes m join notes n on n.id=m.note_id
              join notes parent on parent.id=n.parent join note_link_identities i on i.note_id=n.id where m.collection_id=?""", (names[0][0],)).fetchall()
        assert {r[1] for r in rows}=={"零值","空值"} and all(r[2]==attachments[0][1] for r in rows),rows
        by_title={title:(n,token) for n,title,_,token in rows}
        props=lambda name:dict(db.execute("select key,value from note_properties where note_id=?",(by_title[name][0],)))
        assert props("零值")=={"項目":"零值","工時":"0" if phase=="import" else "12.5","核准":"false","期限":"2028-02-29","編號":"00042"},props("零值")
        assert props("空值")=={"項目":"空值","編號":"00007"},props("空值")
        assert db.execute("select count(*) from note_relations").fetchone()==(0,)
        identity=":".join(map(str,by_title["零值"]))
        assert not canonical or identity==canonical
        print(identity)
'@ | python - $Db $typedZip $Phase $Canonical
    if ($LASTEXITCODE -ne 0 -or !$identity) { throw "Typed CSV workspace differs during $Phase." }
    return [string]$identity
}

$process=Start-Notebook $typedHome
$window=$process.MainWindowHandle
foreach ($def in @(@('工時',1,'數字'),@('核准',2,'核取方塊'),@('期限',3,'日期'))) {
    Declare-TypedNotionField $def[0] $def[1] $def[2]
}
$report=Native-File 2 '匯入 Notion Markdown ZIP…' $typedZip '' -NextDialog '匯入報告'
Finish-Revision $report 1
Choose-TypedNotionAttachment '1｜A.csv' 'A.csv'
$mapping=Wait-For 'typed CSV preview' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認 CSV 欄位型別',7)
}
foreach ($key in '工時','核准','期限') {
    if (![PaperroomHighlightGui]::DialogContains($mapping,$process.Id,$key)) { throw "The native preview omitted $key." }
}
Finish-Revision $mapping 7
$null=Read-TypedNotionState $typedDb 'cancel'

Choose-TypedNotionAttachment '2｜B.csv' 'B.csv'
$invalid=Wait-For 'one invalid typed cell is rejected before creating any row' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
}
if (![PaperroomHighlightGui]::DialogContains($invalid,$process.Id,'核准')) { throw 'The refusal omitted the original typed column.' }
[PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
$null=Wait-For 'typed refusal dismissed' { ![PaperroomHighlightGui]::IsWindowVisible($invalid) }
$null=Read-TypedNotionState $typedDb 'refusal'

Choose-TypedNotionAttachment '1｜A.csv' 'A.csv'
$mapping=Wait-For 'typed CSV confirmation through native UI' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認 CSV 欄位型別',6)
}
Finish-Revision $mapping 6
Collection-Prompt '從 Notion CSV 建立新集合' '型別測試集合'
$null=Wait-For 'native typed collection completed' {
    [PaperroomHighlightGui]::Read((Control $window 116)).Contains('匯入 2 列')
}
$identity=Read-TypedNotionState $typedDb 'import'

Select-Note '零值'
Native-Menu 113 4 '設定文字屬性…'
Collection-Prompt '文字屬性' '工時'
Rule-Prompt '設定文字屬性' '0' '12.5'
$null=Wait-For 'native UI saved the typed numeric field' { 
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
}
$null=Read-TypedNotionState $typedDb 'edited' $identity
Native-File 4 '備份工作區…' $typedBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$rest=Join-Path $root 'notion-typed-restore'
$restDb=Join-Path $rest 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restDb -Parent))
$backupHash=(Get-FileHash -LiteralPath $typedBackup).Hash
Copy-Item -LiteralPath $typedBackup -Destination $restDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $rest 'Paperroom.exe')
foreach ($check in @(@($typedHome,$typedDb),@($rest,$restDb))) {
    $process=Start-Notebook $check[0]
    $window=$process.MainWindowHandle
    Select-Note '零值'
    Native-Menu 113 6 '檢視文字屬性…'
    $read=Wait-For 'native typed values visible after reopening' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記的文字屬性',1)
    }
    $values=[PaperroomHighlightGui]::Read((Control $read 101))
    foreach ($item in '工時：12.5','核准：false','期限：2028-02-29','編號：00042') {
        if (!$values.Contains($item)) { throw "Typed CSV restore lost $item." }
    }
    Finish-Revision $read 1
    $null=Read-TypedNotionState $check[1] 'edited' $identity
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash -LiteralPath $typedZip).Hash -cne $archiveHash -or (Get-FileHash -LiteralPath $typedBackup).Hash -cne $backupHash) {
    throw 'An original source or backup was modified.'
}
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('declared-types-confirmation','cancel-no-write','invalid-cell-no-write','zero-false-and-empty-distinct','native-edit','original-attachment-bytes','reopen','isolated-restore')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: typed Notion CSV conversion, rejection, native edit and isolated restore.'
