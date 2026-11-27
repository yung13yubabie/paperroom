$srcName='CopySource32'
$dstName='CopyDestination32'
Open-EdgeBoard31
Native-Menu 120 3 '新增白板…'
Collection-Prompt '新增白板' $dstName
$null=Wait-For 'empty destination board created' {[PaperroomHighlightGui]::Read((Control $window 115)).Contains($dstName)}
Create-EdgeBoard31 $srcName exact $false
$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=$targetMembers;edges=@(,@('canonical','exact'))}}
[IO.File]::WriteAllText($manifest27,($spec|ConvertTo-Json -Depth 8),$utf27)
function BoardState32([string]$Database,[string]$Stage) {
    $raw=python -B (Join-Path $PSScriptRoot 'check_shared_fixture31.py') $Database $manifest27 'native-board-transfer' $Stage
    if ($LASTEXITCODE -ne 0 -or !$raw) {throw 'Source/destination card and edge identities differ'}
    return [string]$raw
}
$before=BoardState32 $db27 before
$destinationId=($before|ConvertFrom-Json).boards.destination.id
foreach ($slot in 0,1) {[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,$slot,$false,$false,$true)}
foreach ($decision in 7,6) {
    Native-Menu 120 29 '將選取的卡片與配置複製到另一張白板…'
    Choose-CollectionRow "[#$destinationId]　$dstName"
    Rule-Prompt '跨白板複製配置' '0,0' '0,135'
    $confirm=Wait-For 'explicit cross-board copy preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認複製白板配置',7)
    }
    Finish-Revision $confirm $decision
    if ($decision -eq 7 -and (BoardState32 $db27 before) -cne $before) {throw 'Cancelled copy changed state'}
}
$null=Wait-For 'confirmed two selected cards and source retained' {
    [PaperroomHighlightGui]::Read((Control $window 116)) -ceq "已將 2 張卡片與配置複製到「$dstName」，原白板已保留。"
}
$after=BoardState32 $db27 after
$originalState=($before|ConvertFrom-Json)
$copiedState=($after|ConvertFrom-Json)
$copiedState.boards.destination=$originalState.boards.destination
if (($copiedState|ConvertTo-Json -Depth 15 -Compress) -cne ($originalState|ConvertTo-Json -Depth 15 -Compress)) {
    throw 'Copy changed original cards, underlying note identity, source data or attachment bytes'
}
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash $backup27).Hash
Copy-Item $backup27 (Join-Path $copy27 'paperroom-data/notes.sqlite3')
foreach ($folder in $home27,$copy27) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    Open-EdgeBoard31 $dstName
    $dbCopy=Join-Path $folder 'paperroom-data/notes.sqlite3'
    if ((BoardState32 $dbCopy after) -cne $after) {throw 'Copied shared layout differs after reopening'}
    Open-EdgeBoard31 $srcName
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
    Native-Menu 120 9 '定位連線另一端…' -MenuOpen
    Choose-CollectionRow "[#$($ids27.exact)]　$($notes27.exact.title)"
    $null=Wait-For 'retained original edge remains navigable' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位連線另一端的卡片。'
    }
    Close-Notebook $process
    $process.Dispose()
    $process=$null
}
if ((Get-FileHash $backup27).Hash -cne $backupHash) {throw 'Source backup changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupHash;identity=($after|ConvertFrom-Json);scenarios=@('native-board-transfer','cancel and confirm on two selected cards','same canonical IDs and edge at requested offset','original and isolated restore')} |
    ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
