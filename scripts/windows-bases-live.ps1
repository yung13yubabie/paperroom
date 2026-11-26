# Verify shared Bases fields using real Paperroom menus, editing and restore.
$baseHome = Join-Path $root 'bases-live'
$baseDb = Join-Path $baseHome 'paperroom-data/notes.sqlite3'
$sourceVault = Join-Path $root 'LiveVault'
$boardSource = Join-Path $root 'LiveBoard.canvas'
$aBase = Join-Path $root 'A.base'
$bBase = Join-Path $root 'B.base'
$liveBackup = Join-Path $root 'bases-live-backup.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $baseDb -Parent))
foreach ($path in '甲','乙') { [void][IO.Directory]::CreateDirectory((Join-Path $sourceVault $path)) }
Copy-Item -LiteralPath $Exe -Destination (Join-Path $baseHome 'Paperroom.exe')
$utf8 = [Text.UTF8Encoding]::new($false)
$aFile = Join-Path $sourceVault '甲/鏡頭.md'
$bFile = Join-Path $sourceVault '乙/鏡頭.md'
[IO.File]::WriteAllText($aFile,"---`nstatus: 待辦`n---`n甲正文",$utf8)
[IO.File]::WriteAllText($bFile,"---`nstatus: 乙保留`n---`n乙正文",$utf8)
[IO.File]::WriteAllText($boardSource,@'
{"nodes":[{"id":"live-status-card","type":"file","x":30,"y":40,"width":275,"height":154,"file":"甲/鏡頭.md"}],"edges":[]}
'@,$utf8)
$liveAliasBase = @'
formulas:
  finished: 'note.status == "完成"'
properties:
  note.status: {displayName: __LABEL__}
  formula.finished: {displayName: 已完成}
views:
  - type: table
    name: 進度表
    filters: 'file.folder == "甲"'
    order: [file.path, note.status, formula.finished]
'@
[IO.File]::WriteAllText($aBase,$liveAliasBase.Replace('__LABEL__','進度'),$utf8)
[IO.File]::WriteAllText($bBase,$liveAliasBase.Replace('__LABEL__','狀態'),$utf8)
$originalFiles=@{}
foreach ($f in $aFile,$bFile,$boardSource,$aBase,$bBase) { $originalFiles[$f]=(Get-FileHash -LiteralPath $f).Hash }

function Open-CorrectVaultNote {
    Open-LinkedFixture '鏡頭' '甲'
    $null=Wait-For 'correct same-title source note' {
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲正文')
    }
}
function Import-NamedLiveBase([string]$Path) {
    Open-CorrectVaultNote
    Native-MenuFromKeyboard 109 40 '匯入 Obsidian Bases 檢視…'
    $confirmation=Native-File 40 '匯入 Obsidian Bases 檢視…' $Path '' -AlreadyOpened -NextDialog '匯入 Bases 檢視' -NextDialogButton 6
    Finish-Revision $confirmation 6
    Native-Menu 109 2 '連結工作區 status（可編輯）' -MenuOpen
    $null=Wait-For 'Base imported' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已匯入 1 個 Bases 檢視。'
    }
}
function Open-NamedLiveBase([string]$Name) {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Choose-CollectionRow "$Name／進度表"
    Open-SortNamedView 1 '進度表'
}
function Read-LiveColumn([string]$Alias,[string]$Value,[string]$Formula) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 4 '查看資料列的顯示欄位…' -MenuOpen
    Choose-CollectionRow '已匯入　鏡頭  ('
    $report=Wait-For 'visible native collection cells' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','資料列的顯示欄位',1)
    }
    $cells=[PaperroomHighlightGui]::Read((Control $report 101))
    if (!$cells.Contains("file.path　甲/鏡頭.md") -or !$cells.Contains("$Alias　$Value") -or !$cells.Contains("已完成　$Formula")) {
        throw "Wrong saved preview or formula: $cells"
    }
    Finish-Revision $report 1
    Write-Host "Actual collection displayed $Alias=$Value, dependent formula=$Formula."
}
function Edit-LiveColumn([string]$Alias,[string]$Value) {
    Native-Menu 120 24 '表格與清單欄位…'
    Native-Menu 120 5 '編輯連結的 status…' -MenuOpen
    Native-Menu 120 1 $Alias -MenuOpen
    Choose-CollectionRow '鏡頭 (#'
    Collection-Prompt '編輯工作區 status' $Value
}
function Assert-LiveDatabase([string]$Db,[string]$Expected,[string]$PriorId='') {
    $identity=@'
from contextlib import closing
from pathlib import Path
import sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    q="select n.id,n.body,n.version,i.token from notes n join vault_paths p on n.id=p.note_id join note_link_identities i on i.note_id=n.id where p.relative_path=?"
    a=db.execute(q,("甲/鏡頭.md",)).fetchall()
    b=db.execute(q,("乙/鏡頭.md",)).fetchall()
    assert len(a)==len(b)==1 and a[0][0]!=b[0][0]
    n,body,version,token=a[0]
    assert body.startswith("---\nstatus:") and body.endswith("---\n甲正文") and sys.argv[2] in body.splitlines()[1]
    assert b[0][1]=="---\nstatus: 乙保留\n---\n乙正文",b
    assert db.execute("select key,value from note_properties where note_id=? order by key",(n,)).fetchall()==[("進度","原生值")]
    assert db.execute("select c.name from collection_notes cn join collections c on c.id=cn.collection_id where cn.note_id=? order by c.name",(n,)).fetchall()==[("A／進度表",),("B／進度表",)]
    assert db.execute("pragma user_version").fetchone()==(39,)
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    print(f"{n}:{token}")
'@ | python - $Db $Expected
    if ($LASTEXITCODE -ne 0 -or !$identity -or ($PriorId -and $identity -cne $PriorId)) { throw 'The canonical note or unrelated alias changed.' }
    return [string]$identity
}
function Assert-CanvasSourceBody([string]$Expected) {
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'LiveBoard' -MenuOpen
    Native-Menu 120 5 '開啟畫布來源筆記…'
    Native-Menu 120 1 '甲/鏡頭.md 〔live-status-card〕' -MenuOpen -CaptionPrefix
    $null=Wait-For 'Canvas opened the same Markdown note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲正文')
    }
    Click-WorkspaceButton 110
    $null=Wait-For 'body editor shows canonical status and same prose' {
        [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) -and
        [PaperroomHighlightGui]::Read((Control $window 104)).Contains($Expected) -and
        [PaperroomHighlightGui]::Read((Control $window 104)).Contains('甲正文')
    }
    Click-WorkspaceButton 110
}

$process=Start-Notebook $baseHome
$window=$process.MainWindowHandle
Native-VaultFolder $sourceVault '附件綁定 0 筆，未解析 0 筆'
Open-CorrectVaultNote
Native-Menu 113 4 '設定文字屬性…'
Collection-Prompt '文字屬性' '進度'
Collection-Prompt '設定文字屬性' '原生值'
Import-NamedLiveBase $aBase
Import-NamedLiveBase $bBase
$noteId=Assert-LiveDatabase $baseDb '待辦'

Native-File 11 '匯入 JSON Canvas…' $boardSource '中鍵拖曳平移'
Native-Menu 120 4 '綁定匯入的 Vault…'
Native-Menu 120 1 '匯入：LiveVault  (' -MenuOpen -CaptionPrefix
Open-CorrectVaultNote
Open-NamedLiveBase A
Read-LiveColumn '進度' '待辦' 'false'
Edit-LiveColumn '進度' '完成'
$null=Assert-LiveDatabase $baseDb '完成' $noteId
Click-WorkspaceButton 119
Click-WorkspaceButton 110
$null=Wait-For 'returning directly to the cached note loads the new source' {
    [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) -and
    [PaperroomHighlightGui]::Read((Control $window 104)).Contains('完成') -and
    [PaperroomHighlightGui]::Read((Control $window 104)).Contains('甲正文')
}
Click-WorkspaceButton 110
Open-NamedLiveBase B
Read-LiveColumn '狀態' '完成' 'true'
Assert-CanvasSourceBody '完成'
Open-NamedLiveBase B
Edit-LiveColumn '狀態' '待確認'
Open-NamedLiveBase A
Read-LiveColumn '進度' '待確認' 'false'
$null=Assert-LiveDatabase $baseDb '待確認' $noteId

# A true source editor write must also update both dependent views.
$find=Open-WorkspaceSearch
Type-SearchFilter $find 101 '待確認'
Type-SearchFilter $find 103 '甲'
Search-Workspace $find '找到 1 筆筆記。' 1
Finish-Revision $find 111
Assert-SearchSelection 104 '待確認' '全域搜尋：正文第 2 行'
[PaperroomHighlightGui]::TypeEditorText($window,$process.Id,(Control $window 104),'完成')
$null=Wait-For 'source note edit saved' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
}
Open-NamedLiveBase B
Read-LiveColumn '狀態' '完成' 'true'
$null=Assert-LiveDatabase $baseDb '完成' $noteId
Native-File 4 '備份工作區…' $liveBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null

$backupHash=(Get-FileHash -LiteralPath $liveBackup).Hash
$restHome=Join-Path $root 'bases-live-restore'
$restDb=Join-Path $restHome 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restDb -Parent))
Copy-Item -LiteralPath $liveBackup -Destination $restDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restHome 'Paperroom.exe')
foreach ($check in @(@($baseHome,$baseDb),@($restHome,$restDb))) {
    $process=Start-Notebook $check[0]
    $window=$process.MainWindowHandle
    Open-NamedLiveBase A
    Read-LiveColumn '進度' '完成' 'true'
    Open-NamedLiveBase B
    Read-LiveColumn '狀態' '完成' 'true'
    Assert-CanvasSourceBody '完成'
    $null=Assert-LiveDatabase $check[1] '完成' $noteId
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
foreach ($path in $originalFiles.Keys) {
    if ((Get-FileHash -LiteralPath $path).Hash -cne $originalFiles[$path]) { throw 'A source fixture changed.' }
}
if ((Get-FileHash -LiteralPath $liveBackup).Hash -cne $backupHash) { throw 'Opening the backup copy changed the original.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('A-write-B-recalculated','B-write-A-recalculated','canonical-source-editor','Canvas-open-original','reopen','isolated-backup','same-title-and-native-alias-untouched')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host "PASS: exact EXE edits both aliases and recomputes the same Markdown note across Bases, Canvas, restart and isolated restore."

