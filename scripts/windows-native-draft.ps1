# Run a known native user-entry scenario on a previously verified Windows draft download.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExeSha256,
    [Parameter(Mandatory)][ValidateSet('shared-workflow','vault-heading','receipt-history','collection-bulk-edit','native-board-edge-navigation','native-board-transfer','notion-reverse-rows','pdf-history-text','vault-deep-backlinks')][string]$Scenario,
    [Parameter(Mandatory)][string]$Receipt
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if (!$IsWindows -or $PSVersionTable.PSVersion.Major -lt 7 -or
    ![Environment]::UserInteractive -or ([Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0)) {
    throw 'An interactive Windows desktop session with PowerShell 7 is required.'
}
$app=(Get-Item -LiteralPath $Exe).FullName
$out=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Receipt)
if (!(Test-Path -LiteralPath (Split-Path -Parent $out) -PathType Container) -or (Test-Path -LiteralPath $out)) {
    throw 'Specify a fresh receipt path in an existing evidence directory.'
}
function Assert-Candidate {
    if ((Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash -ine $ExeSha256) {
        throw 'The native executable differs from the selected candidate.'
    }
}
Assert-Candidate
$os=(Get-CimInstance Win32_OperatingSystem).Caption
Write-Host "Native desktop: $os. Scenario: $Scenario. Candidate: $ExeSha256."
$control=switch ($Scenario) {
    'receipt-history' { @{ReceiptHistoryOnly=$true} }
    'vault-heading' { @{VaultHeadingOnly=$true} }
    'collection-bulk-edit' { @{SharedNoteWorkflowOnly=$true; SharedFixtureCase='collection-bulk-edit'} }
    'native-board-edge-navigation' { @{SharedNoteWorkflowOnly=$true; SharedFixtureCase='native-board-edge-navigation'} }
    'native-board-transfer' { @{SharedNoteWorkflowOnly=$true; SharedFixtureCase='native-board-transfer'} }
    'notion-reverse-rows' { @{NotionSourceOnly=$true; NotionReverseRows=$true} }
    'pdf-history-text' { @{PdfHistoryTextOnly=$true} }
    'vault-deep-backlinks' { @{VaultDeepBacklinksOnly=$true} }
    'shared-workflow' { @{SharedNoteWorkflowOnly=$true} }
}
$temp=Join-Path (Split-Path -Parent $out) ([guid]::NewGuid().ToString()+'.partial.json')
$prior=[Environment]::GetEnvironmentVariable('PAPERROOM_UNIFIED_RECEIPT','Process')
try {
    $env:PAPERROOM_UNIFIED_RECEIPT=$temp
    & (Join-Path $PSScriptRoot 'windows-highlight-workflow.ps1') -Exe $app -ExeSha256 $ExeSha256 @control
    Assert-Candidate
    $proof=Get-Content -LiteralPath $temp -Raw | ConvertFrom-Json
    if ($proof.exe_sha256 -ine $ExeSha256 -or $proof.os -cne $os -or @($proof.scenarios).Count -lt 1) {
        throw 'The scenario receipt does not identify the current Windows host, candidate, and observed checks.'
    }
    [IO.File]::Move($temp,$out)
} finally {
    [Environment]::SetEnvironmentVariable('PAPERROOM_UNIFIED_RECEIPT',$prior,'Process')
    if ([IO.File]::Exists($temp)) { [IO.File]::Delete($temp) }
}
