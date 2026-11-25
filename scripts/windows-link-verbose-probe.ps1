# Native diagnostic only. Compile and link synthetic inputs, never launch them.
# Use the already-initialized x64 Visual Studio developer environment; emit
# only numeric tool versions, fixed labels, marker counts and artifact hashes.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Compiler,
    [Parameter(Mandatory)][string]$Linker,
    [Parameter(Mandatory)][string]$Librarian
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or ![Environment]::Is64BitProcess -or !$env:LIB) {
    throw 'The diagnostic needs PowerShell 7 in the native Windows x64 developer shell.'
}

function SameTool([string]$Path, [string]$ExpectedName) {
    $file = Get-Item -LiteralPath $Path
    if ($file.PSIsContainer -or $file.Name -ine $ExpectedName) {
        throw 'Expected the selected native toolchain executable.'
    }
    return $file.FullName
}

function NumericVersion([string]$Path) {
    $file = [Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    return ($file.FileMajorPart, $file.FileMinorPart, $file.FileBuildPart, $file.FilePrivatePart -join '.')
}

function Read-PrivateTrace([string]$Path) {
    $size = (Get-Item -LiteralPath $Path).Length
    if ($size -gt 4MB) {
        throw 'A synthetic diagnostic trace exceeded the private size limit.'
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $decoder = [Text.Encoding]::UTF8
    $encoding = 'utf-8-or-ansi'
    $offset = 0
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 255 -and $bytes[1] -eq 254) {
        $decoder = [Text.Encoding]::Unicode; $encoding = 'utf-16-le-bom'; $offset = 2
    } elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 254 -and $bytes[1] -eq 255) {
        $decoder = [Text.Encoding]::BigEndianUnicode; $encoding = 'utf-16-be-bom'; $offset = 2
    } elseif ($bytes.Length -ge 16) {
        $pairs = [int][Math]::Floor([Math]::Min($bytes.Length, 8192) / 2)
        $even = 0; $odd = 0
        for ($i = 0; $i -lt 2 * $pairs; $i += 2) {
            if ($bytes[$i] -eq 0) { $even++ }
            if ($bytes[$i + 1] -eq 0) { $odd++ }
        }
        if ($odd * 100 -ge $pairs * 35 -and $even * 100 -le $pairs * 8) {
            $decoder = [Text.Encoding]::Unicode; $encoding = 'utf-16-le-no-bom'
        } elseif ($even * 100 -ge $pairs * 35 -and $odd * 100 -le $pairs * 8) {
            $decoder = [Text.Encoding]::BigEndianUnicode; $encoding = 'utf-16-be-no-bom'
        }
    }
    $text = $decoder.GetString($bytes, $offset, $bytes.Length - $offset)
    return [pscustomobject]@{ ByteCount = $size; Encoding = $encoding; Text = $text }
}

function Count-Trace([object]$InputStream) {
    $plain = [regex]::Replace($InputStream.Text, '\x1b\[[0-9;]{0,32}m', '')
    return [ordered]@{
        bytes = $InputStream.ByteCount
        encoding = $InputStream.Encoding
        searching_rows = [regex]::Matches($plain, '(?im)^\s*Searching\s+').Count
        found_rows = [regex]::Matches($plain, '(?im)^\s*Found\s+').Count
        loaded_rows = [regex]::Matches($plain, '(?im)^\s*Loaded\s+').Count
        found_tokens = [regex]::Matches($plain, '(?i)\bfound\b').Count
        loaded_tokens = [regex]::Matches($plain, '(?i)\bloaded\b').Count
        finished_rows = [regex]::Matches($plain, '(?im)^\s*Finished searching libraries\s*$').Count
        custom_loaded_rows = [regex]::Matches(
            $plain, '(?im)^\s*Loaded\s+[^\r\n]*\bPaperroomCanary\.lib[" ]*\s*(?:\(|:)'
        ).Count
    }
}

function Run-Private([string]$Binary, [string[]]$NativeArguments, [string]$Step) {
    $start = [Diagnostics.ProcessStartInfo]::new($Binary)
    $start.WorkingDirectory = $scratch
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    $start.Environment['VSLANG'] = '1033'
    # Ensure the comparison controls these switches, while keeping the
    # developer shell's normal LIB/INCLUDE selection. Child changes only.
    foreach ($name in 'CL','_CL_','LINK','_LINK_') {
        [void]$start.Environment.Remove($name)
    }
    foreach ($argument in $NativeArguments) {
        [void]$start.ArgumentList.Add($argument)
    }
    $stdoutPath = Join-Path $scratch ($Step + '.stdout')
    $stderrPath = Join-Path $scratch ($Step + '.stderr')
    $output = $null; $errors = $null; $process = $null
    try {
        $output = [IO.File]::Open($stdoutPath, [IO.FileMode]::CreateNew)
        $errors = [IO.File]::Open($stderrPath, [IO.FileMode]::CreateNew)
        $process = [Diagnostics.Process]::Start($start)
        $copyOut = $process.StandardOutput.BaseStream.CopyToAsync($output)
        $copyErr = $process.StandardError.BaseStream.CopyToAsync($errors)
        if (!$process.WaitForExit(45000)) {
            try { $process.Kill($true) } catch [InvalidOperationException] {}
            throw 'A synthetic native diagnostic tool timed out.'
        }
        [void]$copyOut.GetAwaiter().GetResult()
        [void]$copyErr.GetAwaiter().GetResult()
        $status = $process.ExitCode
    } finally {
        if ($process) { $process.Dispose() }
        if ($errors) { $errors.Dispose() }
        if ($output) { $output.Dispose() }
    }
    $stdout = Read-PrivateTrace $stdoutPath
    $stderr = Read-PrivateTrace $stderrPath
    if ($status -ne 0) {
        # Retain conventional codes only. No compiler/linker diagnostics,
        # paths, invocation arguments or source symbols enter the transcript.
        $codes = @([regex]::Matches($stdout.Text + $stderr.Text, '\b(?:LNK|D|C)[0-9]{4}\b') |
            ForEach-Object Value | Sort-Object -Unique | Select-Object -First 8)
        throw "Synthetic diagnostic phase '$Step' returned $status. Codes: $($codes -join ', ')."
    }
    return [pscustomobject]@{ Stdout = (Count-Trace $stdout); Stderr = (Count-Trace $stderr) }
}

$cl = SameTool $Compiler 'cl.exe'
$link = SameTool $Linker 'link.exe'
$lib = SameTool $Librarian 'lib.exe'
if ([IO.Path]::GetDirectoryName($cl) -ine [IO.Path]::GetDirectoryName($link) -or
    [IO.Path]::GetDirectoryName($lib) -ine [IO.Path]::GetDirectoryName($link)) {
    throw 'The diagnostic tools must all come from the same selected native tool directory.'
}
$temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$scratch = Join-Path $temp ('paperroom-link-canary-' + [guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $scratch)
try {
    $caller = Join-Path $scratch 'caller.c'
    $member = Join-Path $scratch 'member.c'
    $callerObj = Join-Path $scratch 'caller.obj'
    $memberObj = Join-Path $scratch 'member.obj'
    $archive = Join-Path $scratch 'PaperroomCanary.lib'
    @'
int __cdecl puts(const char *);
int paperroom_canary_selected(int);
int main(int count, char **argv) {
    (void)argv;
    return paperroom_canary_selected(count) + puts("synthetic native link probe");
}
'@ | Set-Content -LiteralPath $caller -Encoding ascii
    @'
__declspec(noinline) int paperroom_canary_selected(int value) {
    return value * 7 + 19;
}
'@ | Set-Content -LiteralPath $member -Encoding ascii
    [void](Run-Private $cl @('/nologo','/TC','/c','/MT','/Od','/Oi-','/Gy',"/Fo$callerObj",$caller) 'compile-caller')
    [void](Run-Private $cl @('/nologo','/TC','/c','/MT','/Od','/Oi-','/Gy',"/Fo$memberObj",$member) 'compile-member')
    [void](Run-Private $lib @('/NOLOGO','/MACHINE:X64',"/OUT:$archive",$memberObj) 'archive-member')
    $cases = @(
        [pscustomobject]@{ Id = 'lib-verbosity-default-crt'; Flag = '/VERBOSE:LIB'; Explicit = @() },
        [pscustomobject]@{ Id = 'full-verbosity-default-crt'; Flag = '/VERBOSE'; Explicit = @() },
        [pscustomobject]@{ Id = 'lib-verbosity-explicit-crt'; Flag = '/VERBOSE:LIB'; Explicit = @('libcmt.lib','libucrt.lib','libvcruntime.lib') }
    )
    $results = foreach ($case in $cases) {
        $exe = Join-Path $scratch ($case.Id + '.exe')
        $map = Join-Path $scratch ($case.Id + '.map')
        $arguments = @('/NOLOGO','/MACHINE:X64','/SUBSYSTEM:CONSOLE','/INCREMENTAL:NO',
            '/OPT:REF,ICF',"/OUT:$exe","/MAP:$map",$callerObj,$archive) + $case.Explicit + @($case.Flag)
        $trace = Run-Private $link $arguments $case.Id
        $mapText = (Read-PrivateTrace $map).Text
        $canaryRows = [regex]::Matches($mapText, '(?im)\bPaperroomCanary(?:\.lib)?:member\.obj\s*$').Count
        $crtRows = [regex]::Matches($mapText, '(?im)\b(?:libcmt|libucrt|libvcruntime)(?:\.lib)?:[^\s]+\.(?:obj|o)\s*$').Count
        if ($canaryRows -lt 1 -or $crtRows -lt 1 -or !(Test-Path -LiteralPath $exe -PathType Leaf)) {
            throw "Synthetic diagnostic phase '$($case.Id)' did not prove CRT and custom archive symbol selection in its MAP."
        }
        [ordered]@{
            case = $case.Id
            verbosity_flag = $case.Flag
            explicit_crt_arguments = ($case.Explicit.Count -gt 0)
            canary_map_symbol_rows = $canaryRows
            crt_map_symbol_rows = $crtRows
            stdout = $trace.Stdout
            stderr = $trace.Stderr
            synthetic_exe_sha256 = (Get-FileHash -LiteralPath $exe).Hash.ToLowerInvariant()
        }
    }
    [ordered]@{
        scope = 'Synthetic native C /MT canary, linked but never run. The marker comparison is not evidence about the Paperroom EXE.'
        linker_version = (NumericVersion $link)
        compiler_version = (NumericVersion $cl)
        librarian_version = (NumericVersion $lib)
        cases = @($results)
    } | ConvertTo-Json -Depth 8
} finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
