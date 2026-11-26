# Windows PowerShell 5.1+, built-in Windows components only. No program is launched.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OuterZip,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$OuterSha256,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$InnerSha256,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExeSha256,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{40}$')][string]$SourceCommit,
    [Parameter(Mandatory)][ValidatePattern('^[1-9][0-9]*$')][string]$RunId,
    [Parameter(Mandatory)][ValidatePattern('^[1-9][0-9]*$')][string]$ArtifactId,
    [Parameter(Mandatory)][ValidateSet('Windows 10','Windows 11')][string]$ExpectedFamily,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = 'Stop'
function Fingerprint([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

if ($env:OS -cne 'Windows_NT' -or -not [Environment]::Is64BitProcess) {
    throw 'Use 64-bit Windows PowerShell in the isolated client.'
}
if ((Get-Process Paperroom -ErrorAction SilentlyContinue) -or
    (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'Paperroom'))) {
    throw 'This profile already has Paperroom state. Use an unused test account.'
}
$inputZip = Get-Item -LiteralPath $OuterZip
if ($inputZip.PSIsContainer -or $inputZip.Length -gt 64MB -or
    (Fingerprint $inputZip.FullName) -ine $OuterSha256) {
    throw 'The outer ZIP does not match the independent download receipt.'
}
$zoneProbe = Join-Path $PSScriptRoot 'observe-zone-identifiers.ps1'
$outerZone = (& $zoneProbe -OuterZip $inputZip.FullName | ConvertFrom-Json).outer_zip
$root = [IO.Path]::GetFullPath($Destination)
if ($root.StartsWith('\\') -or (Test-Path -LiteralPath $root)) {
    throw 'Use a new, local destination directory. Existing files were left in place.'
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$wrapper = [IO.Compression.ZipFile]::OpenRead($inputZip.FullName)
try {
    if ($wrapper.Entries.Count -ne 1) { throw 'Expected one Windows package in the outer ZIP.' }
    $entry = $wrapper.Entries[0]
    if ($entry.FullName -cnotmatch '^Paperroom-[a-zA-Z0-9_.-]+\.zip$' -or
        $entry.Length -le 0 -or $entry.Length -gt 48MB) {
        throw 'The inner Windows package name or size differs from the expected artifact shape.'
    }
    $null = New-Item -ItemType Directory -Path $root
    $inner = Join-Path $root $entry.FullName
    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$inner,$false)
} finally {
    $wrapper.Dispose()
}
$innerActual = Fingerprint $inner
if ($innerActual -ine $InnerSha256) { throw 'The inner Windows ZIP SHA-256 differs.' }
$package = [IO.Compression.ZipFile]::OpenRead($inner)
try {
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $bytes = 0
    foreach ($entry in $package.Entries) {
        $bytes += $entry.Length
        if ($bytes -gt 64MB -or $entry.FullName -cnotmatch '^[a-zA-Z0-9_.+/-]+$' -or
            @($entry.FullName.Split('/') | Where-Object { $_ -eq '' -or $_.EndsWith('.') }).Count -ne 0 -or
            -not $seen.Add($entry.FullName)) {
            throw 'The inner archive has an invalid or repeated member.'
        }
    }
    if ($seen.Count -gt 1024 -or -not $seen.Contains('Paperroom.exe')) {
        throw 'The inner archive did not contain the bounded, expected EXE.'
    }
} finally {
    $package.Dispose()
}
$copies = [ordered]@{}
foreach ($mode in 'default','portable','restore') {
    $folder = Join-Path $root $mode
    [IO.Compression.ZipFile]::ExtractToDirectory($inner,$folder)
    $binary = Join-Path $folder 'Paperroom.exe'
    $actual = Fingerprint $binary
    if ($actual -ine $ExeSha256) { throw "The EXE differs in staged mode $mode." }
    $copies[$mode] = $actual
}
$stagedZone = & $zoneProbe -InnerZip $inner -DefaultExe (Join-Path $root 'default/Paperroom.exe') -PortableExe (Join-Path $root 'portable/Paperroom.exe') -RestoreExe (Join-Path $root 'restore/Paperroom.exe') | ConvertFrom-Json
$fixture = Join-Path $root 'fixture.md'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'fixture.md') -Destination $fixture
$checkpointScript = Join-Path $root 'checkpoint.ps1'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'checkpoint.ps1') -Destination $checkpointScript
$record = & $checkpointScript -Phase before -Exe (Join-Path $root 'default/Paperroom.exe') -Archive $inner -ExpectedArchiveSha256 $InnerSha256 -ExpectedExeSha256 $ExeSha256
$status = $LASTEXITCODE
$record | Set-Content -LiteralPath (Join-Path $root 'before.json') -Encoding UTF8
$checkpoint = $record | ConvertFrom-Json
[ordered]@{
    source_commit = $SourceCommit
    run_id = $RunId
    artifact_id = $ArtifactId
    expected_os_family = $ExpectedFamily
    outer_zip_sha256 = $OuterSha256.ToLowerInvariant()
    inner_zip_name = [IO.Path]::GetFileName($inner)
    inner_zip_sha256 = $innerActual
    exe_sha256_by_mode = $copies
    zone_identifier = [ordered]@{
        outer_zip = $outerZone
        inner_zip = $stagedZone.inner_zip
        default_exe = $stagedZone.default_exe
        portable_exe = $stagedZone.portable_exe
        restore_exe = $stagedZone.restore_exe
    }
    fixture_sha256 = Fingerprint $fixture
    checkpoint_sha256 = Fingerprint $checkpointScript
    client_check_ok = ($status -eq 0 -and $checkpoint.checkpoint_ok -eq $true -and $checkpoint.os_family -ceq $ExpectedFamily)
    gui_result = 'not-run'
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'stage.json') -Encoding UTF8
if ($status -ne 0 -or !$checkpoint.checkpoint_ok -or $checkpoint.os_family -cne $ExpectedFamily) {
    throw 'Staged files preserved. The client eligibility checkpoint did not pass. Do not launch.'
}
'The candidate is staged. Follow the offline GUI checklist and keep this directory.'
