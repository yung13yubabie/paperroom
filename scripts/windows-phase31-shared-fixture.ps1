# Uses real imports, canonical IDs, two same-title targets, two same-name assets and a scoped relation field.
Collection27 'Source'
BranchRelation27 canonical $false 1 exact
$targetMembers=@('exact','decoy')
function Open-EdgeBoard31([string]$Title='') {
    if ([PaperroomHighlightGui]::Read((Control $window 119)) -ceq '返回筆記' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -cne '白板操作') {
        # Close the collection in setup, then use a single physical click for the board.
        Command $window 119
        $null=Wait-For 'closed collection before opening native board' {
            [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板'
        }
    }
    if ([PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板') {
        Click-WorkspaceButton 119
    }
    $null=Wait-For 'native board mode after real toolbar click' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '返回筆記' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
    }
    if ($Title) {
        Native-Menu 120 2 '切換白板…'
        Choose-CollectionRow $Title
        $null=Wait-For "exact native board $Title opened" {
            [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Title)
        }
    }
}
function Choose-Edge31([string]$Board,[string]$Peer,[bool]$Far) {
    Open-EdgeBoard31 $Board
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
    Native-Menu 120 9 '定位連線另一端…' -MenuOpen
    $options=@(Collection-MenuLabels)
    $expected="[#$($ids27[$Peer])]　$($notes27[$Peer].title)"
    if ($options.Count -ne 1 -or $options[0] -cne $expected) {throw "Wrong neighboring card: $options"}
    Choose-CollectionRow $expected
    $null=Wait-For 'focused linked card in exact native board' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Board) -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位連線另一端的卡片。'
    }
    if ($Far) {[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false,$true,$false)}
    else {[PaperroomHighlightGui]::BoardCardClick($window,$process.Id,1,$false)}
    Wait-Marker29 $Peer
}
function Create-EdgeBoard31([string]$Name,[string]$Peer,[bool]$Far) {
    Open-EdgeBoard31
    Native-Menu 120 3 '新增白板…'
    Collection-Prompt '新增白板' $Name
    $null=Wait-For "exact new board $Name" {[PaperroomHighlightGui]::Read((Control $window 115)).Contains($Name)}
    Add-ExactBoardNote29 'canonical'
    Add-ExactBoardNote29 $Peer
    if ($Far) {Add-ExactBoardNote29 'decoy'}
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
    Native-Menu 120 2 '與另一張卡片連線…' -MenuOpen
    Choose-CollectionRow "[#$($ids27[$Peer])]　$($notes27[$Peer].title)"
    if ($Far) {
        [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,1,$false,$false,$true)
        Native-Menu 120 12 '移動選取的物件…'
        Collection-Prompt '移動選取的物件' '4200,3600'
        $null=Wait-For 'moved the true endpoint beyond viewport' {
            [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已移動 1 個白板物件。'
        }
    }
}
function Open-BulkView31 {
    Collection27 'Target'
    Open-SortNamedView 1 '批次場景'
}
function Open-BulkPreview31 {
    Open-BulkView31
    Native-Menu 120 28 '批次編輯資料列屬性…'
    Choose-CollectionRow 'Duration'
    foreach ($role in 'exact','decoy') {Choose-CollectionRow "□ 場景　[#$($ids27[$role])]"}
    Choose-CollectionRow '繼續（已選 2 筆）'
    Collection-Prompt '批次設定屬性' '21'
    $review=Wait-For 'selected stable rows in bulk preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','確認批次編輯',1)
    }
    $body=[PaperroomHighlightGui]::Read((Control $review 101))
    foreach ($role in 'exact','decoy') {
        $old=if ($role -eq 'exact') {'8'} else {'1'}
        if (!$body.Contains("場景　[#$($ids27[$role])]　$old → 21")) {throw "Missing selected ID in preview: $body"}
    }
    if ($body.Contains("[#$($ids27.reserve)]") -or $body.Contains("[#$($ids27.other)]")) {throw 'Unselected row appeared in bulk preview'}
    return $review
}
function Start-Bulk31 {
    $notes27['reserve']=@{title='場景備用';marker='BULK-UNSELECTED'}
    $file=Join-Path $root '場景備用.md'
    [IO.File]::WriteAllText($file,"# 場景備用`r`n`r`nBULK-UNSELECTED",$utf27)
    Native-File 1 '匯入 Markdown…' $file '已匯入 1 篇 Markdown'
    $ids27['reserve']=NoteId27 'BULK-UNSELECTED'
    foreach ($role in 'reserve','other') {
        Collection27 'Target'
        Native-Menu 120 1 '加入筆記…'
        Choose-CollectionRow "$($notes27[$role].title)  ($($ids27[$role]))"
        Collection-Prompt '加入集合'
    }
    Collection27 'Target'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 1 '儲存新檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' '批次場景'
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '集合檢視'
    Collection-Prompt '集合檢視' '場景'
}
function Witness31([string]$Db,[string]$Stage) {
    $data=& python -B (Join-Path $PSScriptRoot 'check_shared_fixture31.py') $Db $manifest27 $SharedFixtureCase $Stage
    if ($LASTEXITCODE -ne 0 -or !$data) {throw "Failed $SharedFixtureCase SQLite evidence at $Stage"}
    return [string]$data
}
if ($SharedFixtureCase -eq 'native-board-transfer') {
    . (Join-Path $PSScriptRoot 'windows-board-transfer32.ps1')
    return
}
if ($SharedFixtureCase -eq 'collection-bulk-edit') {
    Start-Bulk31
    $targetMembers+=@('reserve','other')
} else {
    Create-EdgeBoard31 EdgeFar exact $true
    Create-EdgeBoard31 EdgeOther decoy $false
}
$spec=@{notes=$notes27;attachments=$assets27;relation=@{name='關聯';source_collection='Source';target_collection='Target';source_members=@('canonical','other');target_members=$targetMembers;edges=@(,@('canonical','exact'))}}
[IO.File]::WriteAllText($manifest27,($spec | ConvertTo-Json -Depth 8),$utf27)
$before=Witness31 $db27 'before'
if ($SharedFixtureCase -eq 'collection-bulk-edit') {
    $review=Open-BulkPreview31
    Finish-Revision $review 2
    if ((Witness31 $db27 'before') -cne $before) {throw 'Cancelled bulk preview wrote to workspace'}
    $review=Open-BulkPreview31
    Finish-Revision $review 1
    $null=Wait-For 'bulk commit observed' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已選 2 筆，實際更新 2 筆。'
    }
} else {
    Choose-Edge31 EdgeFar exact $true
    Choose-Edge31 EdgeOther decoy $false
}
$after=Witness31 $db27 'after'
if ($SharedFixtureCase -eq 'native-board-edge-navigation' -and $before -cne $after) {throw 'Navigating changed saved edges or notes'}
Native-File 4 '備份工作區…' $backup27 '工作區備份已完成'
Close-Notebook $process
$process.Dispose()
$process=$null
$backupHash=(Get-FileHash $backup27).Hash
Copy-Item $backup27 (Join-Path $copy27 'paperroom-data/notes.sqlite3')
foreach ($folder in $home27,$copy27) {
    $process=Start-Notebook $folder
    $window=$process.MainWindowHandle
    if ($SharedFixtureCase -eq 'collection-bulk-edit') {
        Open-BulkView31
        Native-Menu 120 3 '開啟資料列…'
        $rows=@(Collection-MenuLabels)
        if ($rows.Count -ne 3) {throw "Unexpected filtered rows after reopening: $rows"}
        Choose-CollectionRow "待處理　場景  ($($ids27.exact))"
        Wait-Marker29 exact
        Native-Menu 113 6 '檢視文字屬性…'
        $report=Wait-For 'saved original row properties' {[PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','筆記的文字屬性',1)}
        if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains('Duration：21')) {throw 'Wrong restored row value'}
        Finish-Revision $report 1
    } else {
        Choose-Edge31 EdgeFar exact $true
        Choose-Edge31 EdgeOther decoy $false
    }
    Close-Notebook $process
    $process.Dispose()
    $process=$null
    if ((Witness31 (Join-Path $folder 'paperroom-data/notes.sqlite3') 'after') -cne $after) {throw 'Restored canonical state differs'}
}
if ((Get-FileHash $backup27).Hash -cne $backupHash) {throw 'Source backup was changed'}
@{exe_sha256=$ExeSha256;os=(Get-CimInstance Win32_OperatingSystem).Caption;backup_sha256=$backupHash;identity=($after|ConvertFrom-Json);scenarios=@($SharedFixtureCase,'stable note, relation and attachment identities','original reopen and isolated backup restore')} |
    ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
