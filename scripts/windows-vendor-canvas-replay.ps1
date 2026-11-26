$vendor=$env:PAPERROOM_VERIFIED_VENDOR_CANVAS
if (!$vendor -or (Get-FileHash -LiteralPath $vendor -Algorithm SHA256).Hash -ine '0b4923a272100fc98c617cf24470b508102b72e3bf26ab5200d851ec3d517586') {
    throw 'Missing exact Canvas from the official Obsidian desktop test.'
}
$crossHome=Join-Path $root 'obsidian-desktop-canvas'
[void][IO.Directory]::CreateDirectory((Join-Path $crossHome 'paperroom-data'))
Copy-Item -LiteralPath $Exe -Destination (Join-Path $crossHome 'Paperroom.exe')
$process=Start-Notebook $crossHome
$window=$process.MainWindowHandle
Native-File 11 '匯入 JSON Canvas…' $vendor '中鍵拖曳平移'
Close-Notebook $process
$process.Dispose()
$process=$null
$process=Start-Notebook $crossHome
$window=$process.MainWindowHandle
Native-Menu 109 12 '開啟已匯入的 Canvas…'
Native-Menu 109 1 'Canvas-saved' -MenuOpen
$null=Wait-For 'the exact vendor Canvas finished opening' {
    [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
    [PaperroomHighlightGui]::Read((Control $window 115)) -ceq 'Canvas-saved'
}
$out=Join-Path $root 'obsi-roundtrip.canvas'
Native-Menu 120 1 '匯出原始 JSON Canvas…'
Native-File 1 '匯出原始 JSON Canvas…' $out '已匯出原始 JSON Canvas。' -AlreadyOpened -CanvasSave
$vendorHash=(Get-FileHash -LiteralPath $out -Algorithm SHA256).Hash
if ($vendorHash -ine (Get-FileHash -LiteralPath $vendor -Algorithm SHA256).Hash) {
    throw 'The complete original vendor Canvas bytes changed after re-export.'
}
Close-Notebook $process
$process.Dispose()
$process=$null
Write-Host "Official Obsidian Canvas opened, reopened and exported byte-for-byte through exact Paperroom EXE: $vendorHash."
@{
    exe_sha256=$ExeSha256
    os=(Get-CimInstance Win32_OperatingSystem).Caption
    official_obsidian_canvas_sha256=$vendorHash.ToLowerInvariant()
    scenarios=@('official-obsidian-1.15.1-text-card-edited-and-reopened','paperroom-native-import-reopen-and-byte-identical-export')
} | ConvertTo-Json | Set-Content -LiteralPath $env:PAPERROOM_UNIFIED_RECEIPT -Encoding utf8
