param(
    [Parameter(Mandatory = $true)]
    [string]$Exe
)

$ErrorActionPreference = 'Stop'
if (!(Test-Path -LiteralPath $Exe -PathType Leaf)) {
    throw 'The Windows candidate executable is missing.'
}
$exePath = (Resolve-Path -LiteralPath $Exe).ProviderPath
$finder = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $finder -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $vs) {
    throw 'Visual Studio C++ toolset unavailable.'
}
Import-Module (Join-Path $vs 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll') *> $null
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' *> $null
$dumpbin = (Get-Command dumpbin.exe -CommandType Application).Source

$imports = foreach ($option in '/DEPENDENTS', '/IMPORTS') {
    $lines = @(& $dumpbin $option $exePath)
    if ($LASTEXITCODE -ne 0) {
        throw "DUMPBIN $option failed."
    }
    $names = @($lines | ForEach-Object {
        if ("$_" -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') {
            $Matches[1].ToLowerInvariant()
        }
    } | Sort-Object -Unique)
    if ($names.Count -eq 0) {
        throw "DUMPBIN $option returned no recognizable DLL imports."
    }
    $names
}

$forbidden = @($imports | Where-Object {
    $_ -match '^(?:(?:vcruntime|msvcp|concrt|msvcr|vcomp|mfc|mfcm|vccorlib)[0-9][a-z0-9_]*|ucrtbased)\.dll$'
} | Sort-Object -Unique)
if ($forbidden.Count -ne 0) {
    throw "The candidate imports separately installed or debug VC runtime DLLs: $($forbidden -join ', ')."
}
Write-Host 'Windows candidate PE import check passed (VC runtime DLL families).'
