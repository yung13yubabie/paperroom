# Capture the real final linker invocation for the executable that will be staged.
param(
    [Parameter(Mandatory)][string]$Inventory,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$SourceCommit
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$target = 'x86_64-pc-windows-msvc'
$root = (Get-Location).ProviderPath
$private = Join-Path $env:RUNNER_TEMP ('paperroom-link-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $private)
$map = Join-Path $private 'final.map'
$wrapper = Join-Path $private 'paperroom-link.exe'

function NumericVersion([string]$Binary) {
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($Binary)
    return ($info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart -join '.')
}

try {
    $finder = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $visualStudio = @((& $finder -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json -utf8) | Out-String | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0 -or $visualStudio.Count -ne 1) { throw 'Expected one native Visual Studio toolchain.' }
    $instance = $visualStudio[0]
    $vs = $instance.installationPath
    Import-Module (Join-Path $vs 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll') *> $null
    Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' *> $null
    $vcRoot = [IO.Path]::GetFullPath($env:VCToolsInstallDir).TrimEnd('\','/')
    $sdkRoot = [IO.Path]::GetFullPath($env:WindowsSdkDir).TrimEnd('\','/')
    $sdkVersion = $env:WindowsSDKVersion.TrimEnd('\','/')
    $nativeBin = [IO.Path]::GetFullPath((Join-Path $vcRoot 'bin/Hostx64/x64'))
    $linker = Join-Path $nativeBin 'link.exe'
    $compiler = Join-Path $nativeBin 'cl.exe'
    if (!(Test-Path -LiteralPath $linker -PathType Leaf) -or !(Test-Path -LiteralPath $compiler -PathType Leaf)) {
        throw 'The selected native developer shell has no x64 MSVC linker or compiler.'
    }
    # Point the C build and the final Rust link at the same chosen native tools.
    $env:PATH = "$nativeBin;$env:PATH"
    $env:CC_x86_64_pc_windows_msvc = $compiler
    $sysroot = rustc --print sysroot
    if ($LASTEXITCODE -ne 0 -or !$sysroot -or !(Test-Path -LiteralPath $sysroot -PathType Container)) {
        throw 'The pinned Rust sysroot was unavailable.'
    }

    $csc = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $csc /nologo /target:exe /platform:x64 "/out:$wrapper" (Join-Path $PSScriptRoot 'windows-link-capture.cs') *> (Join-Path $private 'csc.log')
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $wrapper -PathType Leaf)) {
        throw 'Could not compile the private final-link wrapper.'
    }

    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
    $flags = @('-C', 'target-feature=+crt-static', '-C', 'link-arg=/PDBALTPATH:%_PDB%')
    $roots = @{ '/paperroom' = $root; '/cargo' = $cargoHome; '/toolchain' = $rustupHome; '/build-temp' = $env:RUNNER_TEMP }
    foreach ($token in $roots.Keys | Sort-Object { $roots[$_].Length }) {
        $path = [IO.Path]::GetFullPath($roots[$token]).TrimEnd('\', '/')
        $flags += "--remap-path-prefix=$($path.Replace('/', '\'))=$token"
        $flags += "--remap-path-prefix=$($path.Replace('\', '/'))=$token"
    }
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
    $env:PAPERROOM_REAL_LINK = $linker
    $env:PAPERROOM_LINK_TRACE = $private
    $env:VSLANG = '1033'
    $env:CARGO_INCREMENTAL = '0'
    cargo rustc --locked --release --target $target --bin paperroom -- -C "linker=$wrapper" -C linker-flavor=msvc -C "link-arg=/MAP:$map" *> (Join-Path $private 'cargo.log')
    if ($LASTEXITCODE -ne 0) {
        $failureCodes = @(Select-String -LiteralPath (Join-Path $private 'cargo.log') -Pattern '\b(?:LNK[0-9]{4}|E[0-9]{4})\b' -AllMatches |
            ForEach-Object { $_.Matches.Value } | Sort-Object -Unique | Select-Object -First 12)
        throw "The captured release build failed. Diagnostic codes: $($failureCodes -join ', ')."
    }

    $exe = Join-Path $root "target/$target/release/paperroom.exe"
    $sha = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    $mapSha = (Get-FileHash -LiteralPath $map -Algorithm SHA256).Hash.ToLowerInvariant()
    $stdout = Join-Path $private 'link.stdout'
    $stderr = Join-Path $private 'link.stderr'
    python -B ./scripts/windows-link-inventory.py --exe $exe --exe-sha256 $sha --map $map --verbose $stdout --verbose-stderr $stderr --commit $SourceCommit --cargo-lock Cargo.lock --root "msvc=$vcRoot" --root "sdk=$sdkRoot" --root "rust=$sysroot" --root "target=$(Join-Path $root 'target')" --linker-version (NumericVersion $linker) --cl-version (NumericVersion $compiler) --vc-toolset-version $env:VCToolsVersion --sdk-version $sdkVersion --vs-version $instance.installationVersion --vs-product-id $instance.productId > $Inventory
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate the sanitized same-link inventory.' }
    $inventoryDigest = (Get-FileHash -LiteralPath $Inventory -Algorithm SHA256).Hash.ToLowerInvariant()
    $receipt = Get-Content -LiteralPath $Inventory -Raw | ConvertFrom-Json
    if ($receipt.source_commit -cne $SourceCommit -or $receipt.exe_sha256 -cne $sha) {
        throw 'The linked evidence is not bound to the just-built executable.'
    }
    $traceBytes = (Get-Item -LiteralPath $stdout).Length
    $errorBytes = (Get-Item -LiteralPath $stderr).Length
    $trace = $receipt.verbose_selections
    Write-Host "Private trace stdout $traceBytes bytes, stderr $errorBytes bytes; search rows $($trace.library_search_rows), loaded attributed $($trace.attributed_loaded_rows), unattributed $($trace.unattributed_loaded_rows)."
    Write-Host ("Private trace encodings " + ($trace.trace_streams | ConvertTo-Json -Compress -Depth 4))
    Write-Host ("Private trace format counters " + ($trace.trace_marker_counts | ConvertTo-Json -Compress))
    $mapFamilies = @($receipt.map_observations.rows | Where-Object kind -ceq 'msvc-crt' |
        ForEach-Object family | Sort-Object -Unique)
    if (@('libcmt', 'libucrt', 'libvcruntime' | Where-Object { $_ -cnotin $mapFamilies }).Count) {
        throw "Same-link CRT MAP evidence is incomplete. MAP families $($mapFamilies -join ', ')."
    }
    if ($trace.attributed_loaded_rows -lt 1) {
        Write-Warning 'No verbose member could be attributed. This draft retains the verified MAP evidence and explicitly unresolved verbose trace.'
    }
    Write-Host "Same-link evidence SHA-256 $inventoryDigest for EXE $sha."
    Write-Host "MAP CRT families $($mapFamilies -join ', '), selected $($receipt.verbose_selections.attributed_loaded_rows) and unresolved $($receipt.verbose_selections.unattributed_loaded_rows) linker rows."
    return [pscustomobject]@{ ExeHash = $sha; MapHash = $mapSha; InventoryHash = $inventoryDigest }
} finally {
    Remove-Item Env:PAPERROOM_REAL_LINK, Env:PAPERROOM_LINK_TRACE -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $private -Recurse -Force
}
