# Real Markdown inputs and normal native operations; SQLite witnesses are read-only.
$vault=Join-Path $root 'OccurrenceVault'
$homeA=Join-Path $root 'backlink-workspace'
$homeB=Join-Path $root 'backlink-restored'
$backup=Join-Path $root 'positions-backup.sqlite3'
$utf8=[Text.UTF8Encoding]::new($false)
$sourceLines=@('🎬 開場','第一次 [first](A/Target.md)','同一行 🎞️ [repeat](A/Target.md) 和 [repeat](A/Target.md)','[[Target]]','[peer](B/Target.md)','收尾')
$inputs=[ordered]@{
    'A/Target.md'='TARGET-A'
    'B/Target.md'='TARGET-B'
    'Index.md'=($sourceLines -join "`r`n")
    'Empty.md'='NO-REFERENCES'
}
$hashes=@{}
foreach ($item in $inputs.GetEnumerator()) {
    $path=Join-Path $vault $item.Key
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $path))
    [IO.File]::WriteAllText($path,$item.Value,$utf8)
    $hashes[$item.Key]=(Get-FileHash -LiteralPath $path).Hash
}
foreach ($homePath in $homeA,$homeB) {
    [void][IO.Directory]::CreateDirectory((Join-Path $homePath 'paperroom-data'))
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $homePath 'Paperroom.exe')
}

function Get-BacklinkSnapshot([string]$Database) {
    $result=@'
from contextlib import closing
from pathlib import Path
import json,sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    db.execute("begin")
    assert db.execute("pragma integrity_check").fetchone()==("ok",)
    records=db.execute("""select p.relative_path,n.id,n.title,n.version,n.body,i.token,p.import_root
        from vault_paths p join notes n on n.id=p.note_id
        join note_link_identities i on i.note_id=n.id join notes root on root.id=p.import_root
        where root.title='匯入：OccurrenceVault' and n.trash_root is null order by p.relative_path""").fetchall()
    assert [r[0] for r in records]==["A/Target.md","B/Target.md","Empty.md","Index.md"]
    for path,id,title,version,body,token,root in records:
        assert body.replace("\r\n","\n")== (Path(sys.argv[2])/path).read_text(encoding="utf8")
    print(json.dumps({r[0]:list(r[1:]) for r in records},ensure_ascii=True,sort_keys=True))
'@ | python -B - $Database $vault
    if ($LASTEXITCODE -ne 0 -or !$result) {throw 'Vault path, body, or identity evidence differs'}
    return [string]$result
}
function Open-BacklinkTarget([string]$Path) {
    Native-Menu 109 24 '快速切換筆記…'
    Collection-Prompt '快速切換筆記' $Path
    $labels=@(Collection-MenuLabels)
    $id=$identities[$Path][0]
    $hits=@(for ($n=0;$n -lt $labels.Count;$n++) {
        if ($labels[$n].EndsWith("[#$id]",[StringComparison]::Ordinal) -and $labels[$n].Contains(" / $Path")) {$n+1}
    })
    if ($hits.Count -ne 1) {throw "Expected one exact imported note for $Path #$id. Menu: $labels"}
    Native-Menu 109 $hits[0] $labels[$hits[0]-1] -MenuOpen
    Native-Menu 120 60006 '查看 Vault 反向連結原文位置…'
}
function Test-LinkOccurrence([string]$Path,[string[]]$Expected,[int]$Ordinal,[string]$Raw,[int]$Occurrence=1) {
    Open-BacklinkTarget $Path
    $labels=@(Collection-MenuLabels)
    if (($labels -join "`n") -cne ($Expected -join "`n")) {throw "Wrong occurrences for $Path. Actual rows: $labels"}
    Native-Menu 120 $Ordinal $labels[$Ordinal-1] -MenuOpen
    $null=Wait-For 'original Markdown source and exact UTF-16 selection' {
        $field=Control $window 104
        if (![PaperroomHighlightGui]::IsWindowVisible($field) -or
            [PaperroomHighlightGui]::Read((Control $window 103)) -cne 'Index' -or
            [PaperroomHighlightGui]::Read((Control $window 116)) -cne '已定位 Vault 反向連結原文。') {return $false}
        $body=[PaperroomHighlightGui]::Read($field)
        $start=$body.IndexOf($Raw,[StringComparison]::Ordinal)
        for ($i=1;$i -lt $Occurrence -and $start -ge 0;$i++) {
            $start=$body.IndexOf($Raw, $start+$Raw.Length,[StringComparison]::Ordinal)
        }
        $selection=[PaperroomHighlightGui]::Send($field,0x00B0,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64()
        $start -ge 0 -and ($selection -band 0xffff) -eq $start -and (($selection -shr 16) -band 0xffff) -eq ($start+$Raw.Length)
    }
}
$mainRows=@('Index.md　第 2 行　[first](A/Target.md)','Index.md　第 3 行　[repeat](A/Target.md)','Index.md　第 3 行　[repeat](A/Target.md)　（本行第 2 處）')
function Test-BacklinkSet {
    Test-LinkOccurrence 'A/Target.md' $mainRows 3 '[repeat](A/Target.md)' 2
    Test-LinkOccurrence 'A/Target.md' $mainRows 2 '[repeat](A/Target.md)'
    Test-LinkOccurrence 'B/Target.md' @('Index.md　第 5 行　[peer](B/Target.md)') 1 '[peer](B/Target.md)'
    Open-BacklinkTarget 'Empty.md'
    $null=Wait-For 'empty incoming occurrences retain exact current note' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這篇筆記沒有可定位的 Vault 反向連結。' -and
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'Empty'
    }
}
$process=Start-Notebook $homeA
$window=$process.MainWindowHandle
Native-VaultFolder $vault '匯入 4 篇筆記'
$state=Get-BacklinkSnapshot (Join-Path $homeA 'paperroom-data/notes.sqlite3')
$identities=$state|ConvertFrom-Json -AsHashtable
Test-BacklinkSet
Native-File 4 '備份工作區…' $backup '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$stamp=(Get-FileHash $backup).Hash
$restoreDb=Join-Path $homeB 'paperroom-data/notes.sqlite3'
Copy-Item $backup $restoreDb
if ((Get-FileHash $restoreDb).Hash -cne $stamp) {throw 'Restore copy differs from backup'}
foreach ($dir in $homeA,$homeB) {
    $process=Start-Notebook $dir
    $window=$process.MainWindowHandle
    Test-BacklinkSet
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    if ((Get-BacklinkSnapshot (Join-Path $dir 'paperroom-data/notes.sqlite3')) -cne $state) {throw 'Navigation changed note versions, body, identity or mapping'}
}
foreach ($key in $hashes.Keys) {
    if ((Get-FileHash -LiteralPath (Join-Path $vault $key)).Hash -cne $hashes[$key]) {throw "Source file changed: $key"}
}
if ((Get-FileHash $backup).Hash -cne $stamp) {throw 'Original backup changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$stamp;scenarios=@('vault-deep-backlinks','distinct identical same-line UTF-16 selection and exact path','ambiguous target excluded','unchanged source, reopen, and isolated restore')} |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
