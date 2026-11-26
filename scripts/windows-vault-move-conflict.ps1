# Run after the real pinned Canvas is bound. The #26 normal-move case is not replayed.
$old='Active/Flip.md'
$next='Moved/Flip.md'
$sourceOld=Join-Path $sourceVault $old
$sourceNext=Join-Path $sourceVault $next
$initialDisk=(Get-FileHash -LiteralPath $sourceOld).Hash

function Witness-ScopedConflict([string]$Database,[string]$Path,[string]$Approval,[string]$Baseline,[string]$Prior='') {
    $identity=@'
import sqlite3,sys
from contextlib import closing
from pathlib import Path
db,path,value,baseline,prior,canvas,base=sys.argv[1:]
with closing(sqlite3.connect(Path(db).as_uri()+'?mode=ro',uri=True)) as c:
    row=c.execute("""select n.id,i.token,n.body,b.body 
          from vault_paths p join notes n on n.id=p.note_id join note_link_identities i on i.note_id=n.id
          join vault_sync_base b on b.note_id=n.id join notes r on r.id=p.import_root
          where r.title='匯入：VaultScopeA' and p.relative_path=?""",(path,)).fetchone()
    assert row is not None and f"approved: {value}" in row[2] and f"approved: {baseline}" in row[3],row
    assert "Stable source Active/Flip.md." in row[2]
    others=c.execute("""select p.relative_path,n.body,n.version from vault_paths p join notes n on n.id=p.note_id
      join notes r on r.id=p.import_root where
      (r.title='匯入：VaultScopeA' and p.relative_path='Archive/Flip.md')
      or (r.title='匯入：VaultScopeB' and p.relative_path='Active/Flip.md') order by r.title""").fetchall()
    assert len(others)==2 and all(x[2]==1 for x in others),others
    assert 'Scope A decoy.' in others[0][1] and 'Scope B decoy.' in others[1][1]
    assert all('approved: false' in x[1] for x in others)
    assert c.execute("select count(*) from vault_paths p join notes r on r.id=p.import_root where r.title='匯入：VaultScopeA' and p.relative_path in ('Active/Flip.md','Moved/Flip.md')").fetchone()==(1,)
    assert c.execute("select source from saved_canvases where title='PinNative'").fetchone()[0].encode()==Path(canvas).read_bytes()
    assert c.execute("select content from attachments where name='Shipping.base'").fetchone()[0]==Path(base).read_bytes()
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    marker=f"{row[0]}:{row[1]}"
    assert not prior or marker==prior
    print(marker)
'@ | python -B - $Database $Path $Approval $Baseline $Prior $canvasFile $baseFile
    if ($LASTEXITCODE -ne 0 -or $identity -notmatch '^\d+:[a-f0-9]{32}$') { throw 'Exact source, decoys, original files, or stable note identity changed.' }
    return [string]$identity
}
function Select-OriginalFlip([string]$Path,[string]$NoteId) {
    Native-Menu 109 24 '快速切換筆記…'
    Collection-Prompt '快速切換筆記' $Path
    $labels=@(Collection-MenuLabels)
    $i=@(for ($j=0;$j -lt $labels.Count;$j++) {
        if ($labels[$j].EndsWith("[#$NoteId]",[StringComparison]::Ordinal) -and
            $labels[$j].Contains(" / $Path")) { $j+1 }
    })
    if ($i.Count -ne 1) { throw 'The visible path/identity pick was missing or ambiguous.' }
    Native-Menu 109 $i[0] $labels[$i[0]-1] -MenuOpen
}
function Vendor-Flip([string]$From,[string]$To,[switch]$First) {
    $vendorArgs=@('-STA','-NoProfile','-File',(Join-Path $PSScriptRoot 'windows-obsidian-move-fixture.ps1'),'-Vault',$sourceVault,'-From',$From,'-To',$To)
    if ($First) { $vendorArgs += '-Initialize' }
    & pwsh @vendorArgs
    if ($LASTEXITCODE -ne 0) { throw 'Official Obsidian movement was not completed.' }
    [void][PaperroomHighlightGui]::SetForegroundWindow($window)
}
function Preview-FlipMove([string]$Path) {
    Select-OriginalFlip $old $noteId
    Native-Menu 109 23 '命令面板…'
    Collection-Prompt '命令面板' '核對 Vault 外部改名或搬移'
    Native-Menu 109 1 '核對 Vault 外部改名或搬移' -MenuOpen
    Pick-NativeVaultFolder $sourceVault
    Collection-Prompt '新的 Vault 來源路徑' $Path
}
function Confirm-BothConflictPins([string[]]$First) {
    Open-ScopeCanvas
    Confirm-CanvasPinRows 'Pin_A_601' $First
    Confirm-CanvasPinRows 'Pin_B_904' @()
}

$identity=Witness-ScopedConflict $db $old 'false' 'false'
$noteId=($identity -split ':')[0]
Confirm-CanvasPinRows 'Pin_A_601' @('Tie-1','Tie-3')
Confirm-CanvasPinRows 'Pin_B_904' @('Flip')
Write-CanvasFlag 'Pin_B_904' 'Flip' $old
Confirm-BothConflictPins @('Flip','Tie-1') # capped after filter and sort; Tie-3 is displaced.
$null=Witness-ScopedConflict $db $old 'true' 'false' $identity
if ((Get-FileHash -LiteralPath $sourceOld).Hash -cne $initialDisk) { throw 'Canvas wrote to the Vault.' }

Vendor-Flip $old $next -First
Preview-FlipMove $next
$refusal=Wait-For 'paperroom refused uncoordinated source association' {
    [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
}
if (![PaperroomHighlightGui]::DialogContains($refusal,$process.Id,'未協調變更')) { throw 'Unexpected source reassociation refusal.' }
Finish-Revision $refusal 1
$null=Witness-ScopedConflict $db $old 'true' 'false' $identity
if (Test-Path -LiteralPath $sourceOld) { throw 'Relocation refusal wrote to the original path.' }
if ((Get-FileHash -LiteralPath $sourceNext).Hash -cne $initialDisk) { throw 'Relocation refusal overwrote the already moved file.' }

Confirm-BothConflictPins @('Flip','Tie-1')
$conflictBackup=Join-Path $root 'before-coordination.sqlite3'
Native-File 4 '備份工作區…' $conflictBackup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$restore=Join-Path $root 'uncoordinated-backup-proof'
$restoreDb=Join-Path $restore 'paperroom-data/notes.sqlite3'
[void][IO.Directory]::CreateDirectory((Split-Path $restoreDb -Parent))
Copy-Item -LiteralPath $conflictBackup -Destination $restoreDb
Copy-Item -LiteralPath $Exe -Destination (Join-Path $restore 'Paperroom.exe')
foreach ($workspace in @(@($app,$db),@($restore,$restoreDb))) {
    $process=Start-Notebook $workspace[0]
    $window=$process.MainWindowHandle
    Confirm-BothConflictPins @('Flip','Tie-1')
    $null=Witness-ScopedConflict $workspace[1] $old 'true' 'false' $identity
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
$process=Start-Notebook $app
$window=$process.MainWindowHandle

# The supported path establishes the common version while the old, verified mapping still exists.
Vendor-Flip $next $old
Native-Menu 109 21 'Vault 批次寫回磁碟…'
Choose-CollectionRow '匯入：VaultScopeA (#'
Pick-NativeVaultFolder $sourceVault
$pushReview=Wait-For 'review of the single local edit' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Vault 批次寫回預覽',1)
}
if (![PaperroomHighlightGui]::Read((Control $pushReview 101)).Contains($old)) { throw 'Writable source missing from explicit preview.' }
Finish-Revision $pushReview 1
Collection-Prompt '選取寫回項目'
$confirm=Wait-For 'explicit source writeback approval' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認批次寫回',6)
}
Finish-Revision $confirm 6
$push=Native-File 1 'Vault 寫回前備份' (Join-Path $root 'coordination-push.sqlite') '' -AlreadyOpened -SaveName 'vault-before-push.sqlite' -NextDialog 'Vault 寫回結果'
Finish-Revision $push 1
$null=Witness-ScopedConflict $db $old 'true' 'true' $identity
$coordinatedDisk=(Get-FileHash -LiteralPath $sourceOld).Hash
if ($coordinatedDisk -ceq $initialDisk -or !(Get-Content -LiteralPath $sourceOld -Raw).Contains('approved: true')) { throw 'The explicit writeback did not preserve the Paperroom value.' }

Vendor-Flip $old $next
Preview-FlipMove $next
$preview=Wait-For 'coordinated source may be reassociated' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','Vault 外部改名或搬移預覽',1)
}
Finish-Revision $preview 1
$confirm=Wait-For 'confirm coordinated source reassociation' {
    [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認配對新的 Vault 來源',6)
}
Finish-Revision $confirm 6
Native-File 1 'Vault 來源搬移備份' (Join-Path $root 'coordinated-relocation.sqlite') "已配對 Vault 來源：$next" -AlreadyOpened -SaveName 'vault-before-relocation.sqlite'
Confirm-BothConflictPins @('Tie-1','Tie-3')
$null=Witness-ScopedConflict $db $next 'true' 'true' $identity
if ((Get-FileHash -LiteralPath $sourceNext).Hash -cne $coordinatedDisk) { throw 'Reassociation overwrote the coordinated disk file.' }
@{
    exe_sha256=$ExeSha256
    vendor='Obsidian 1.15.1 signed Windows desktop'
    vendor_installer_sha256='218c7d30acc065355ce94719dd996d5e2a2347cde3811ea5dc29380583f02b58'
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    passed=@('limited sorted view refresh','native edit before vendor move','native refusal protects unsynced data','exact local and foreign same-name notes unchanged','both pins on reopen and isolated restore','explicit verified push from original path','vendor removes old path','same note ID after explicit reassociation')
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
Write-Host 'PASS: preserved an edited shared note across official Obsidian moves, a blocked reassociation, restart, backup, and explicit resolution.'

