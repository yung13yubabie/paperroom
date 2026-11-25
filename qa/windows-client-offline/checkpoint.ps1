# Windows PowerShell 5.1+. Only observe local state; print a sanitized JSON record.
[CmdletBinding()]
param([Parameter(Mandatory)][ValidateSet('before','reopen','restore','after')][string]$Phase,
      [Parameter(Mandatory)][string]$Exe,
      [string]$Archive,
      [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedArchiveSha256='85eced7fc8aaafa204f729c33aab4df309bbe3f90adbc510c5389328a0a34bca',
      [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedExeSha256='61e6e5ffb694702827c7e6fe031c638429b90105590649b732728b65805665c4')
$ErrorActionPreference='Stop'
try {
    if ($env:OS -cne 'Windows_NT' -or -not [Environment]::Is64BitProcess) {
        throw 'requires 64-bit Windows PowerShell'
    }
    $os=Get-CimInstance Win32_OperatingSystem
    $cpu=@(Get-CimInstance Win32_Processor)
    $archOK=($cpu.Count -gt 0 -and @($cpu | Where-Object Architecture -ne 9).Count -eq 0)
    $build=[int]$os.BuildNumber
    $client=([int]$os.ProductType -eq 1 -and $build -ge 10240)
    $family=if (-not $client) {'other'} elseif ($build -ge 22000) {'Windows 11'} else {'Windows 10'}
    # Inspect access token group SIDs. A split-token administrator still has
    # the local Administrators SID (deny-only) and does not qualify as standard.
    $groups=@(& "$env:SystemRoot\System32\whoami.exe" /groups /fo csv /nh)
    if ($LASTEXITCODE -ne 0 -or $groups.Count -eq 0) { throw 'token query failed' }
    $tokenText=$groups -join "`n"
    $medium=($tokenText -match '"S-1-16-8192"')
    $adminMember=($tokenText -match '"S-1-5-32-544"')
    $interfaces=@([Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces())
    $up=@($interfaces | Where-Object {
        $_.OperationalStatus -eq [Net.NetworkInformation.OperationalStatus]::Up -and
        $_.NetworkInterfaceType -ne [Net.NetworkInformation.NetworkInterfaceType]::Loopback
    }).Count
    $available=[Net.NetworkInformation.NetworkInterface]::GetIsNetworkAvailable()
    $hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash.ToLowerInvariant()
    $zipHash=if ($Archive) { (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash.ToLowerInvariant() } else { $null }
    $zipOK=(-not $Archive -or $zipHash -ieq $ExpectedArchiveSha256)
    $eligible=$zipOK -and $client -and $archOK -and $medium -and -not $adminMember -and $up -eq 0 -and -not $available -and $hash -ieq $ExpectedExeSha256
    [ordered]@{phase=$Phase; utc=[DateTime]::UtcNow.ToString('o'); os_family=$family;
        os_version=[string]$os.Version; build=$build; x64=$archOK; client=$client;
        medium_integrity=$medium; administrators_sid=$adminMember; up_nonloopback=$up;
        network_available=$available; exe_sha256=$hash; archive_sha256=$zipHash; expected_exe=$ExpectedExeSha256; expected_archive=$ExpectedArchiveSha256; checkpoint_ok=$eligible} | ConvertTo-Json -Compress
    if (-not $eligible) { exit 2 }
} catch {
    Write-Output '{"checkpoint_ok":false,"reason":"local checkpoint unavailable"}'
    exit 2
}
exit 0
