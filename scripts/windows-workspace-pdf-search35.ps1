# Both annotated PDFs have the same display name on identically titled notes.
$home35=Join-Path $root 'search-pdf-home'
$copy35=Join-Path $root 'search-pdf-restore'
$backup35=Join-Path $root 'search-pdf-backup.sqlite3'
$pdf35=Join-Path $root 'Check.pdf'
[void][IO.Directory]::CreateDirectory($root)
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../tests/fixtures/storyboard-zh/剪輯範圍.pdf') -Destination $pdf35
$original35=(Get-FileHash $pdf35).Hash
foreach ($place in $home35,$copy35) {
    [void][IO.Directory]::CreateDirectory((Join-Path $place 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $place 'Paperroom.exe')
}
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class SearchPdfList35 {
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    private static extern IntPtr SendText(IntPtr h, uint m, IntPtr idx, StringBuilder b);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    public static string Item(IntPtr list, int i, int len) {
        if (len < 1 || len > 1024) throw new ArgumentException("Unexpected native list item length");
        var content=new StringBuilder(len+1);
        if (SendText(list,0x0189,new IntPtr(i),content).ToInt64()!=len) throw new InvalidOperationException("Native list text changed");
        return content.ToString();
    }
}
'@
function Fill-Annotation35([string]$Caption,[string]$Hint,[string]$Value) {
    $dialog=Wait-For "PDF input: $Hint" {
        $d=[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$Caption,1)
        if ($d -ne [IntPtr]::Zero -and [PaperroomHighlightGui]::DialogContains($d,$process.Id,$Hint)) {$d}
    }
    Type-SearchFilter $dialog 101 $Value
    Finish-Revision $dialog 1
}
function Add-SearchSource35([int]$Page) {
    $dir=Join-Path $root "pdf-search-source-$Page"
    [void][IO.Directory]::CreateDirectory($dir)
    $src=Join-Path $dir 'SearchTwin.md'
    [IO.File]::WriteAllText($src,"SOURCE-MARKER-$Page")
    Native-File 1 '匯入 Markdown…' $src '已匯入 1 篇 Markdown'
    $null=Wait-For 'new canonical source note active' {[PaperroomHighlightGui]::Read((Control $window 121)).Contains("SOURCE-MARKER-$Page")}
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 1 '加入本機附件…' -MenuOpen
    Native-File 1 '加入本機附件…' $pdf35 '附件已保存到本機工作區。' -AlreadyOpened
    Native-Menu 113 8 '附件與 PDF 摘錄…'
    Native-Menu 113 2 '管理附件…' -MenuOpen
    Choose-CollectionRow "Check.pdf  ($((Get-Item $pdf35).Length) B)"
    Native-Menu 113 6 '新增 PDF 矩形座標摘錄…' -MenuOpen
    Fill-Annotation35 'PDF 摘錄' '來源 PDF 頁碼' "$Page"
    Fill-Annotation35 'PDF 摘錄' '貼上原文' "clapper35 quote-$Page"
    Fill-Annotation35 'PDF 摘錄' '輸入批註' "specific35-$Page"
    Fill-Annotation35 'PDF 矩形座標摘錄' '輸入 x,y,寬,高' $(if ($Page -eq 1) {'0.10,0.12,0.30,0.14'} else {'0.45,0.52,0.35,0.12'})
}
function Witness-Pdf35([string]$Db) {
    $raw=@'
import json,sqlite3,sys
from pathlib import Path
from contextlib import closing
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    db.execute("BEGIN")
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    rows=db.execute("""select n.id,i.token,n.body,a.id,a.content,m.id,m.page,m.quote,m.comment,r.x,r.y,r.width,r.height
        from pdf_annotations m join attachments a on a.id=m.attachment_id join notes n on n.id=a.note_id
        join note_link_identities i on i.note_id=n.id join pdf_annotation_regions r on r.annotation_id=m.id
        where n.title='SearchTwin' and a.name='Check.pdf' and n.trash_root is null order by m.page""").fetchall()
    assert len(rows)==2
    result=[]
    for p,(nid,token,body,aid,content,mid,page,quote,comment,x,y,w,h) in enumerate(rows,1):
        assert page==p and token and body.strip()==f"SOURCE-MARKER-{p}" and content==Path(sys.argv[2]).read_bytes()
        assert (quote,comment)==(f"clapper35 quote-{p}",f"specific35-{p}")
        assert (x,y,w,h)==[(.10,.12,.30,.14),(.45,.52,.35,.12)][p-1]
        result.append(dict(note=nid,token=token,attachment=aid,mark=mid,page=page))
    assert all(result[0][k]!=result[1][k] for k in ('note','token','attachment','mark'))
    print(json.dumps(result,ensure_ascii=True,sort_keys=True))
'@ | python -B - $Db $pdf35
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Saved PDF identity, exact bytes or region differ'}
    return [string]$raw
}
function Exercise-WorkspacePdf35([int]$Page) {
    $dialog=Open-WorkspaceSearch
    $query=if ($Page -eq 2) {'clapper35'} else {'specific35-1'}
    Type-SearchFilter $dialog 101 $query
    $checkbox=Control $dialog 107
    if ([PaperroomHighlightGui]::Send($checkbox,0x00F0,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64() -ne 0) {throw 'PDF mode unexpectedly enabled'}
    Search-Workspace $dialog '沒有符合條件的筆記。' 0
    $point=[PaperroomHighlightGui]::ButtonPoint($dialog,$process.Id,$checkbox)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$checkbox,$point,0)
    $null=Wait-For 'PDF mode retains keyword and disables note-only filters' {
        [PaperroomHighlightGui]::Read((Control $dialog 101)) -ceq $query -and
        ![SearchPdfList35]::IsWindowEnabled((Control $dialog 102)) -and ![SearchPdfList35]::IsWindowEnabled((Control $dialog 103))
    }
    Type-SearchFilter $dialog 101 'NO-ANNOTATION-35'
    Search-Workspace $dialog '沒有符合條件的 PDF 標註。' 0
    Type-SearchFilter $dialog 101 $query
    Search-Workspace $dialog "找到 $Page 筆 PDF 標註。" $Page "specific35-$Page"
    $list=Control $dialog 104
    for ($i=0;$i -lt $Page;$i++) {
        $hit=$hits35[$Page-1-$i]
        $len=[PaperroomHighlightGui]::Send($list,0x018A,[IntPtr]$i,[IntPtr]::Zero).ToInt32()
        $item=[SearchPdfList35]::Item($list,$i,$len)
        $expected="Check.pdf　〔附件 #$($hit.attachment)／標註 #$($hit.mark)〕　｜ 第 $($hit.page) 頁　｜ SearchTwin"
        if ($item -cne $expected) {throw "Unexpected disambiguation label: $item"}
    }
    if (![PaperroomHighlightGui]::Read((Control $dialog 105)).Contains("clapper35 quote-$Page")) {throw 'Wrong original annotation quote'}
    Finish-Revision $dialog 111
    $null=Wait-For 'actual selected PDF page rendered' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq "Check.pdf · $Page/2" -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'PDF 操作'
    }
    Command $window 119
    $null=Wait-For 'closing search hit PDF returns to exact canonical source body' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板' -and
        [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains("SOURCE-MARKER-$Page")
    }
}
$process=Start-Notebook $home35
$window=$process.MainWindowHandle
Add-SearchSource35 1
Add-SearchSource35 2
$state=Witness-Pdf35 (Join-Path $home35 'paperroom-data/notes.sqlite3')
$hits35=$state|ConvertFrom-Json
Exercise-WorkspacePdf35 2
Exercise-WorkspacePdf35 1
Native-File 4 '備份工作區…' $backup35 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash $backup35).Hash
$dest=Join-Path $copy35 'paperroom-data/notes.sqlite3'
Copy-Item $backup35 $dest
if ((Get-FileHash $dest).Hash -cne $backupHash) {throw 'Restore copy differs'}
foreach ($folder in $home35,$copy35) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    Exercise-WorkspacePdf35 2
    Exercise-WorkspacePdf35 1
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    if ((Witness-Pdf35 (Join-Path $folder 'paperroom-data/notes.sqlite3')) -cne $state) {throw 'Stored marks or note/attachment IDs changed after reopening'}
}
if ((Get-FileHash $backup35).Hash -cne $backupHash -or (Get-FileHash $pdf35).Hash -cne $original35) {throw 'Source PDF or backup changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupHash;pdf_sha256=$original35;identities=$hits35;scenarios=@('workspace-pdf-search','Ctrl+Shift+F same-name PDFs exact marks and pages','no-match and note-only filters','original and restored workspace')} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
