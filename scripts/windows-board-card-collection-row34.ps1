# Dot-sourced after real Markdown, attachment, collection and relation imports in the shared fixture.
$boardName34='CollectionCards34'
$extraName34='Another34'
$hashes34=@{}
foreach ($role in $notes27.Keys) {
    $folder=Join-Path $root "typed-fixture-$role"
    foreach ($file in Get-ChildItem -LiteralPath $folder -File) {$hashes34[$file.FullName]=(Get-FileHash -LiteralPath $file.FullName).Hash}
}
Collection27 $extraName34 -Create
Native-Menu 120 1 '加入筆記…'
Choose-CollectionRow "場景  ($($ids27.decoy))"
Collection-Prompt '加入集合' '完成'
Open-EdgeBoard31
Native-Menu 120 3 '新增白板…'
Collection-Prompt '新增白板' $boardName34
$null=Wait-For 'new navigation fixture board' {[PaperroomHighlightGui]::Read((Control $window 115)).Contains($boardName34)}
Add-ExactBoardNote29 'exact'
Add-ExactBoardNote29 'decoy'
$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=@('exact','decoy');edges=@(,@('canonical','exact'))}}
[IO.File]::WriteAllText($manifest27,($spec|ConvertTo-Json -Depth 8),$utf27)
function Read-CardRow34([string]$Database,[string]$Value) {
    $identity=python -B $checker27 $Database $manifest27
    if ($LASTEXITCODE -ne 0 -or !$identity) {throw 'Canonical note, relation or attachment changed'}
    $raw=@'
import json,sqlite3,sys
from pathlib import Path
from contextlib import closing
path,exact,decoy,value=sys.argv[1:]
exact,decoy=int(exact),int(decoy)
with closing(sqlite3.connect(Path(path).as_uri()+"?mode=ro",uri=True)) as c:
    c.execute("BEGIN")
    assert c.execute("pragma integrity_check").fetchone()==("ok",)
    boards=c.execute("select b.id,n.note_id,n.x,n.y from boards b join board_cards n on n.board_id=b.id where b.title='CollectionCards34' order by n.x,n.y").fetchall()
    assert [p[1] for p in boards]==[exact,decoy],boards
    memberships=c.execute("""select c.name,c.id,m.note_id,m.status from collections c join collection_notes m on m.collection_id=c.id where m.note_id in (?,?) order by c.name,m.note_id""",(exact,decoy)).fetchall()
    assert [(name,note,status) for name,_,note,status in memberships]==[("Another34",decoy,"完成"),("Target",exact,"待處理"),("Target",decoy,"待處理")],memberships
    values=c.execute("select note_id,value from note_properties where key='Duration' and note_id in (?,?) order by note_id",(exact,decoy)).fetchall()
    assert dict(values)=={exact:"8",decoy:value},values
    print(json.dumps({"extra": memberships[0][1], "target":memberships[1][1],"boards":boards,"memberships":memberships},ensure_ascii=True))
'@ | python -B - $Database $ids27.exact $ids27.decoy $Value
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Durable row data or exact card memberships differ'}
    return [ordered]@{shared=($identity|ConvertFrom-Json);placement=($raw|ConvertFrom-Json)}
}
$identity34=Read-CardRow34 $db27 '1'
$targetLabel34="Target　[#$($identity34.placement.target)]　｜ 待處理"
$extraLabel34="$extraName34　[#$($identity34.placement.extra)]　｜ 完成"
function Locate-Card34([string]$Role,[string]$Collection) {
    Open-EdgeBoard31 $boardName34
    $slot=if ($Role -eq 'exact') {0} else {1}
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,$slot,$true)
    Native-Menu 120 10 '在集合中定位這張卡片…' -MenuOpen
    $expected=if ($Role -eq 'exact') {@($targetLabel34)} else {@($extraLabel34,$targetLabel34)}
    $labels=@(Collection-MenuLabels)
    if (($labels -join '|') -cne ($expected -join '|')) {throw "Unexpected memberships for $Role : $labels"}
    $selected=if ($Collection -eq 'Target') {$targetLabel34} else {$extraLabel34}
    Choose-CollectionRow $selected
    $null=Wait-For 'exact card stable ID positioned in chosen default collection' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq "已定位集合「$Collection」的資料列 #$($ids27[$Role])。" -and
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Collection)
    }
}
function Show-CardNote34([string]$Role,[string]$Lane,[string]$Value) {
    Native-Menu 120 3 '開啟資料列…'
    Choose-CollectionRow "$Lane　場景  ($($ids27[$Role]))"
    Wait-Marker29 $Role
    Native-Menu 113 6 '檢視文字屬性…'
    $report=Wait-For 'shared note visible typed property' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記的文字屬性',1)}
    if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains("Duration：$Value")) {throw 'Visible note differs from chosen board card'}
    Finish-Revision $report 1
}
Locate-Card34 'decoy' $extraName34
Native-Menu 120 18 '編輯資料列的型別欄位…'
Choose-CollectionRow "完成　場景  ($($ids27.decoy))"
Choose-CollectionRow 'Duration　1'
Rule-Prompt 'Duration' '1' '34'
$null=Wait-For 'saved typed edit on canonical board note' {[PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新資料列欄位。'}
function Test-CardNotes34 {
    Locate-Card34 decoy Target
    Show-CardNote34 decoy '待處理' '34'
    Locate-Card34 exact Target
    Show-CardNote34 exact '待處理' '8'
    Locate-Card34 decoy $extraName34
    Show-CardNote34 decoy '完成' '34'
}
Test-CardNotes34
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupStamp34=(Get-FileHash $backup27).Hash
$restore34=Join-Path $copy27 'paperroom-data/notes.sqlite3'
Copy-Item $backup27 $restore34
if ((Get-FileHash $restore34).Hash -cne $backupStamp34) {throw 'Restored copy differs from source backup'}
foreach ($dir in $home27,$copy27) {
    $process=Start-Notebook $dir
    $window=$process.MainWindowHandle
    Test-CardNotes34
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    $actual=Read-CardRow34 (Join-Path $dir 'paperroom-data/notes.sqlite3') '34'
    if (($actual|ConvertTo-Json -Compress -Depth 14) -cne ($identity34|ConvertTo-Json -Compress -Depth 14)) {throw 'Source identities or placements changed'}
}
foreach ($path in $hashes34.Keys) {
    if ((Get-FileHash -LiteralPath $path).Hash -cne $hashes34[$path]) {throw 'Input file changed'}
}
if ((Get-FileHash $backup27).Hash -cne $backupStamp34) {throw 'Source backup changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupStamp34;identity=$identity34;scenarios=@('board-card-collection-row','same-title membership exclusion','actual typed edit from one membership read through the other','original and isolated restore')} |
    ConvertTo-Json -Depth 14 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
