# Verify real native board card→child and exact return tile through a saved edit.
$boardApp=Join-Path $root 'native-child-board'
$boardDb=Join-Path $boardApp 'paperroom-data/notes.sqlite3'
$boardBackup=Join-Path $root 'child-board-backup.sqlite3'
$noteFile=Join-Path $root 'board-source.md'
[void][IO.Directory]::CreateDirectory((Split-Path $boardDb -Parent))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $boardApp 'Paperroom.exe')
[IO.File]::WriteAllText($noteFile,"# Canonical card"+[Environment]::NewLine+"A single shared note.",[Text.UTF8Encoding]::new($false))

function Add-BoardCardTo([string]$Name) {
    Native-Menu 120 3 '新增白板…'
    Collection-Prompt '新增白板' $Name
    $null=Wait-For "new native board $Name visible" {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Name)
    }
    Native-Menu 120 1 '放入筆記…'
    Choose-CollectionRow 'Canonical card'
}
function Select-NativeBoard([string]$Name) {
    Native-Menu 120 2 '切換白板…'
    Choose-CollectionRow $Name
    $null=Wait-For "native board $Name visible" {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Name)
    }
}
function Return-ToExactChildTile {
    Native-Menu 120 10 '子白板與文字…'
    Native-Menu 120 3 '返回上層並定位子白板卡片' -MenuOpen
    $null=Wait-For 'saved exact parent tile revealed' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains('ParentView') -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位上層的子白板卡片。'
    }
}
function Set-SelectedChildTile([string]$Database,[int]$X,[int]$Y) {
    $delta=@'
import sqlite3,sys
from contextlib import closing
from pathlib import Path
path,x,y=sys.argv[1:]
with closing(sqlite3.connect(Path(path).as_uri()+"?mode=ro",uri=True)) as db:
    px,py = db.execute("select o.x,o.y from board_objects o join boards b on b.id=o.child_id where b.title='ChildView'").fetchone()
    print(f"{int(x)-px},{int(y)-py}")
'@ | python - $Database $X $Y
    if ($LASTEXITCODE -ne 0 -or $delta -notmatch '^-?[0-9]+,-?[0-9]+$') { throw 'Could not determine the saved tile movement delta.' }
    Native-Menu 120 12 '移動選取的物件…'
    $dialog = Wait-For 'native tile displacement editor' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','移動選取的物件',1)
    }
    $edit = Control $dialog 101
    if ([PaperroomHighlightGui]::Read($edit) -cne '0,0') { throw 'Tile displacement is not initialized to zero.' }
    $point = [PaperroomHighlightGui]::FilenamePoint($dialog,$process.Id,$edit)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
    $null = Wait-For 'tile displacement edit has the keyboard focus' {
        [PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$dialog,$edit)
    }
    [void][PaperroomHighlightGui]::Send($edit,0x00B1,[IntPtr]::Zero,[IntPtr](-1))
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x2E)
    $null = Wait-For 'old tile displacement is cleared' { [PaperroomHighlightGui]::Read($edit) -ceq '' }
    try {
        foreach ($digit in $delta.ToCharArray()) {
            $key = switch ($digit) {
                ',' { 0xBC }
                '-' { 0xBD }
                default { [int][char]$digit }
            }
            [PaperroomHighlightGui]::TapKey($window,$process.Id,$key)
        }
        $null = Wait-For 'exact physical key displacement entered' { [PaperroomHighlightGui]::Read($edit) -ceq $delta }
    } catch {
        throw "Displacement dialog contains '$([PaperroomHighlightGui]::Read($edit))' after typing '$delta': $($_.Exception.Message)"
    }
    Finish-Revision $dialog 1
    $null = Wait-For 'exactly one native selected object moved' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已移動 1 個白板物件。'
    }
}
function Confirm-ChildBoardIdentity([string]$Database,[int]$X,[int]$Y,[string]$OriginalId='') {
    $identity=@'
import sqlite3,sys
from pathlib import Path
db,x,y,old=sys.argv[1:]
with sqlite3.connect(Path(db).as_uri()+"?mode=ro",uri=True) as c:
    note,token=c.execute("select n.id,i.token from notes n join note_link_identities i on i.note_id=n.id where n.title='Canonical card'").fetchone()
    parents=dict(c.execute("select title,id from boards"))
    tile,tx,ty=c.execute("select id,x,y from board_objects where board_id=? and child_id=?",(parents['ParentView'],parents['ChildView'])).fetchone()
    assert (tx,ty)==(int(x),int(y)),(tx,ty)
    placements=c.execute("select board_id,note_id,x,y from board_cards order by board_id").fetchall()
    assert len(placements)==3 and all(p[1]==note for p in placements)
    assert {p[0] for p in placements}=={parents[k] for k in ("ParentView","ChildView","OtherBoard")}
    assert all((p[2],p[3])!=(int(x),int(y)) for p in placements),(tx,ty,placements)
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    marker=f"{note}:{token}:{tile}:{parents['ChildView']}"
    assert not old or old==marker
    print(marker)
'@ | python - $Database $X $Y $OriginalId
    if ($LASTEXITCODE -ne 0 -or !$identity) { throw 'Child board tile, paths, note instance, or other board shifted.' }
    return [string]$identity
}

$process=Start-Notebook $boardApp
$window=$process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $noteFile '已匯入 1 篇 Markdown'
Click-WorkspaceButton 119
$null=Wait-For 'native board ready' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
}
Add-BoardCardTo 'OtherBoard'
Add-BoardCardTo 'ParentView'
[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
Native-Menu 120 7 '從此卡建立子白板…' -MenuOpen
Collection-Prompt '從此卡建立子白板' 'ChildView'
$null=Wait-For 'child directly created from physical card' {
    [PaperroomHighlightGui]::Read((Control $window 115)).Contains('ChildView') -and
    [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('已建立子白板並放入同一張筆記卡。')
}
Return-ToExactChildTile
Set-SelectedChildTile $boardDb 4200 3600
$initial=Confirm-ChildBoardIdentity $boardDb 4200 3600

Select-NativeBoard 'ChildView'
Return-ToExactChildTile
Set-SelectedChildTile $boardDb 4400 3900
$null=Confirm-ChildBoardIdentity $boardDb 4400 3900 $initial
Native-File 4 '備份工作區…' $boardBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$restored=Join-Path $root 'isolated-child-board'
$restoredDb=Join-Path $restored 'paperroom-data/notes.sqlite3'
$sourceBackupHash=(Get-FileHash -LiteralPath $boardBackup).Hash
[void][IO.Directory]::CreateDirectory((Split-Path $restoredDb -Parent))
Copy-Item -LiteralPath $boardBackup -Destination $restoredDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restored 'Paperroom.exe')
foreach ($workspace in @(@($boardApp,$boardDb,4500,4000),@($restored,$restoredDb,4700,4200))) {
    $process=Start-Notebook $workspace[0]
    $window=$process.MainWindowHandle
    $null=Confirm-ChildBoardIdentity $workspace[1] 4400 3900 $initial
    Select-Note 'Canonical card'
    Click-WorkspaceButton 119
    $null=Wait-For 'board surface active after reopen' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
    }
    Select-NativeBoard 'ChildView'
    Return-ToExactChildTile
    Set-SelectedChildTile $workspace[1] $workspace[2] $workspace[3]
    $null=Confirm-ChildBoardIdentity $workspace[1] $workspace[2] $workspace[3] $initial
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash -LiteralPath $boardBackup).Hash -cne $sourceBackupHash) { throw 'Isolated restore changed the backup itself.' }
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    scenarios=@('physical-source-card-menu','one-canonical-note-on-three-boards','saved-parent-child-tile','off-screen-parent-tile-return','moved-revealed-selected-tile','reopen','isolated-restore')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: native child board created from existing note; returns to the exact saved tile even after isolate restore.'
