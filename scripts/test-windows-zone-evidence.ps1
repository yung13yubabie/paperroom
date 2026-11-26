# Native Windows PowerShell 5.1 verification with inert synthetic data.
$ErrorActionPreference = 'Stop'
$qa = Join-Path $PSScriptRoot '../qa/windows-client-offline'
$probe = Join-Path $qa 'observe-zone-identifiers.ps1'
$stage = Join-Path $qa 'stage-gnu-candidate.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ('paperroom-zone-qa-' + [guid]::NewGuid())
[void][IO.Directory]::CreateDirectory($root)
function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Assert-Observation($Row,[string]$State,$Zone) {
    $names = @($Row.PSObject.Properties.Name)
    if ($names.Count -ne 2 -or $names[0] -cne 'status' -or $names[1] -cne 'zone_id' -or
        $Row.status -cne $State -or $Row.zone_id -ne $Zone -or
        ($null -ne $Zone -and $Row.zone_id -isnot [int] -and $Row.zone_id -isnot [long])) {
        throw 'Unexpected observation schema or value.'
    }
}
function Assert-Private([string]$Json) {
    if ($Json -match 'https?:|HostUrl|ReferrerUrl|RootMarker|SYNTHETIC-PRIVATE' -or
        $Json -match [regex]::Escape((Split-Path $root -Leaf))) {
        throw 'The receipt contains source data or a local path.'
    }
}
try {
    $untagged = Join-Path $root 'ordinary [file].exe'
    $tagged = Join-Path $root 'marked.zip'
    $malformed = Join-Path $root 'malformed.zip'
    $wide = Join-Path $root 'bom.zip'
    $large = Join-Path $root 'oversize.zip'
    foreach ($path in $untagged,$tagged,$malformed,$wide,$large) {
        [IO.File]::WriteAllBytes($path, [byte[]](48,49,50))
    }
    $tag = '[ZoneTransfer]' + [Environment]::NewLine + 'ZoneId=3' + [Environment]::NewLine +
        'HostUrl=https://example.invalid/SYNTHETIC-PRIVATE' + [Environment]::NewLine +
        'ReferrerUrl=https://example.invalid/RootMarker'
    Set-Content -LiteralPath $tagged -Stream Zone.Identifier -Encoding ASCII -Value $tag
    Set-Content -LiteralPath $malformed -Stream Zone.Identifier -Encoding ASCII -Value '[ZoneTransfer]','ZoneId=3','ZoneId=2'
    Set-Content -LiteralPath $wide -Stream Zone.Identifier -Encoding Unicode -Value '[ZoneTransfer]','ZoneId=0'
    Set-Content -LiteralPath $large -Stream Zone.Identifier -Encoding ASCII -Value ('x' * 16385)
    $original = Get-Content -LiteralPath $tagged -Stream Zone.Identifier -Raw
    $originalHash = Hash $tagged
    $json = & $probe -OuterZip $tagged -InnerZip $malformed -DefaultExe $untagged -PortableExe $wide -RestoreExe $large
    Assert-Private $json
    $receipt = $json | ConvertFrom-Json
    Assert-Observation $receipt.outer_zip 'read' 3
    Assert-Observation $receipt.inner_zip 'undetermined' $null
    Assert-Observation $receipt.default_exe 'absent' $null
    Assert-Observation $receipt.portable_exe 'read' 0
    Assert-Observation $receipt.restore_exe 'undetermined' $null
    if ((Hash $tagged) -cne $originalHash -or
        (Get-Content -LiteralPath $tagged -Stream Zone.Identifier -Raw) -cne $original) {
        throw 'Observing changed the fixture data or source mark.'
    }
    $locked = [IO.File]::Open($untagged,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    try {
        $json = & $probe -DefaultExe $untagged -RestoreExe (Join-Path $root 'missing')
        Assert-Private $json
        $unknown = $json | ConvertFrom-Json
        Assert-Observation $unknown.default_exe 'undetermined' $null
        Assert-Observation $unknown.restore_exe 'undetermined' $null
    } finally {
        $locked.Dispose()
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $packageDir = Join-Path $root 'package'
    $wrapperDir = Join-Path $root 'wrapper'
    [void][IO.Directory]::CreateDirectory($packageDir)
    [void][IO.Directory]::CreateDirectory($wrapperDir)
    $exe = Join-Path $packageDir 'Paperroom.exe'
    [IO.File]::WriteAllBytes($exe, [byte[]](48,49,50))
    $innerName = 'Paperroom-Synthetic.zip'
    $inner = Join-Path $wrapperDir $innerName
    $outer = Join-Path $root 'download.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($packageDir,$inner)
    [IO.Compression.ZipFile]::CreateFromDirectory($wrapperDir,$outer)
    Set-Content -LiteralPath $outer -Stream Zone.Identifier -Encoding ASCII -Value $tag
    $outerMarkBefore = Get-Content -LiteralPath $outer -Stream Zone.Identifier -Raw
    $outerHashBefore = Hash $outer
    $destination = Join-Path $root 'result'
    $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $ErrorActionPreference = 'Continue'
    try {
        & $powershell -NoProfile -NonInteractive -File $stage -OuterZip $outer -OuterSha256 $outerHashBefore -InnerSha256 (Hash $inner) -ExeSha256 (Hash $exe) -SourceCommit ('0' * 40) -RunId 1 -ArtifactId 1 -ExpectedFamily 'Windows 10' -Destination $destination *> $null
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = 'Stop'
    }
    if ($exitCode -eq 0) { throw 'Expected the non-client host to fail the original gate.' }
    $staged = Get-Content -LiteralPath (Join-Path $destination 'stage.json') -Raw | ConvertFrom-Json
    $before = Get-Content -LiteralPath (Join-Path $destination 'before.json') -Raw | ConvertFrom-Json
    if ($staged.client_check_ok -ne $false -or $staged.gui_result -cne 'not-run' -or $before.client -ne $false) {
        throw 'Unexpected client eligibility or GUI result.'
    }
    Assert-Observation $staged.zone_identifier.outer_zip 'read' 3
    $json = & $probe -OuterZip $outer -InnerZip (Join-Path $destination $innerName) -DefaultExe (Join-Path $destination 'default/Paperroom.exe') -PortableExe (Join-Path $destination 'portable/Paperroom.exe') -RestoreExe (Join-Path $destination 'restore/Paperroom.exe')
    Assert-Private $json
    $actual = $json | ConvertFrom-Json
    foreach ($role in 'outer_zip','inner_zip','default_exe','portable_exe','restore_exe') {
        Assert-Observation $staged.zone_identifier.$role $actual.$role.status $actual.$role.zone_id
    }
    if ((Hash $outer) -cne $outerHashBefore -or
        (Get-Content -LiteralPath $outer -Stream Zone.Identifier -Raw) -cne $outerMarkBefore -or
        $staged.inner_zip_sha256 -cne (Hash $inner) -or
        (@('default','portable','restore' | Where-Object { $staged.exe_sha256_by_mode.$_ -cne (Hash $exe) })).Count -ne 0) {
        throw 'Staging changed input data/mark or the staged SHA-256 evidence differs.'
    }
    Assert-Private ($staged.zone_identifier | ConvertTo-Json -Depth 3 -Compress)
    Write-Output 'PASS: Windows ADS observation, absent and indeterminate cases, privacy, unchanged input, and five roles through real staging. Client eligibility refused as expected.'
    Write-Output $json
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force
}
# The nested staging command deliberately exited nonzero on this non-client host.
exit 0
