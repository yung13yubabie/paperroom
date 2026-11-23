# Requires PowerShell 7, a Windows desktop session and Python 3 with sqlite3.
param([Parameter(Mandatory)][string]$Exe, [string]$Screenshot)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path -LiteralPath $Exe).Path
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PaperroomWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr SetText(IntPtr window, uint msg, IntPtr zero, string text);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr GetText(IntPtr window, uint msg, IntPtr limit, StringBuilder text);
    [DllImport("user32.dll", EntryPoint="SendMessageW")]
    public static extern IntPtr Send(IntPtr window, uint msg, IntPtr w, IntPtr l);
}

// Store only names and notification kinds. No observer files inside the test roots.
public sealed class PaperroomFileEvents : IDisposable {
    readonly System.IO.FileSystemWatcher watcher;
    readonly System.Collections.Generic.Dictionary<string,int> counts =
        new System.Collections.Generic.Dictionary<string,int>(StringComparer.OrdinalIgnoreCase);
    readonly object gate = new object();
    string phase = "first launch";
    bool lost;
    public PaperroomFileEvents(string root) {
        watcher = new System.IO.FileSystemWatcher(root);
        watcher.IncludeSubdirectories = true;
        watcher.NotifyFilter = System.IO.NotifyFilters.FileName | System.IO.NotifyFilters.DirectoryName |
            System.IO.NotifyFilters.Size | System.IO.NotifyFilters.LastWrite;
        watcher.InternalBufferSize = 64 * 1024;
        watcher.Created += (o,e) => Record("Created", e.Name);
        watcher.Deleted += (o,e) => Record("Deleted", e.Name);
        watcher.Changed += (o,e) => Record("Changed", e.Name);
        watcher.Renamed += (o,e) => Record("Renamed", e.OldName + " -> " + e.Name);
        watcher.Error += (o,e) => { lock (gate) { lost = true; } };
        watcher.EnableRaisingEvents = true;
    }
    void Record(string kind, string name) {
        lock (gate) {
            string key = phase + " | " + kind + " | " + name;
            int count;
            if (counts.TryGetValue(key, out count)) counts[key] = count + 1;
            else if (counts.Count < 256) counts.Add(key, 1);
            else lost = true;
        }
    }
    public void SetPhase(string next) { lock (gate) { phase = next; } }
    public bool HasFirstCreation(string name) {
        lock (gate) { return counts.ContainsKey("first launch | Created | " + name); }
    }
    public bool Lost { get { lock (gate) { return lost; } } }
    public string[] Summary() {
        lock (gate) {
            var lines = new System.Collections.Generic.List<string>();
            foreach (var pair in counts) lines.Add(pair.Key + " | " + pair.Value);
            return lines.ToArray();
        }
    }
    public void Dispose() { watcher.Dispose(); }
}
'@

function Start-Notebook($Bin, $Local, $Temp, $Portable) {
    $info = [Diagnostics.ProcessStartInfo]::new((Join-Path $Bin 'paperroom.exe'))
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $Bin
    $info.Environment['LOCALAPPDATA'] = $Local
    $info.Environment['TEMP'] = $Temp
    $info.Environment['TMP'] = $Temp
    if ($Portable) { $info.ArgumentList.Add('--portable') }
    $process = [Diagnostics.Process]::Start($info)
    try {
        for ($i = 0; $i -lt 150; $i++) {
            $process.Refresh()
            if ($process.HasExited) { throw "Early exit: $($process.ExitCode)" }
            if ([PaperroomWindow]::GetDlgItem($process.MainWindowHandle, 103) -ne [IntPtr]::Zero) { return $process }
            Start-Sleep -Milliseconds 100
        }
        throw 'Timed out waiting for the Paperroom editor.'
    } catch {
        if (!$process.HasExited) { $process.Kill(); $process.WaitForExit() }
        throw
    }
}

function Read-Control($Window, $Id) {
    $control = [PaperroomWindow]::GetDlgItem($Window, $Id)
    $text = [Text.StringBuilder]::new(512)
    [void][PaperroomWindow]::GetText($control, 13, [IntPtr]512, $text)
    return $text.ToString()
}

function Save-FirstWindow($Window, $Path) {
    Add-Type -AssemblyName System.Drawing
    $rect = [PaperroomWindow+Rect]::new()
    if (![PaperroomWindow]::GetWindowRect($Window, [ref]$rect)) { throw 'Could not measure window.' }
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try {
                if (![PaperroomWindow]::PrintWindow($Window, $dc, 2)) { throw 'PrintWindow failed.' }
            } finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $colors = [Collections.Generic.HashSet[int]]::new()
        for ($y = 0; $y -lt $bitmap.Height; $y += 9) {
            for ($x = 0; $x -lt $bitmap.Width; $x += 9) {
                [void]$colors.Add($bitmap.GetPixel($x, $y).ToArgb())
            }
        }
        if ($colors.Count -lt 12) { throw 'Window capture appears blank.' }
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
}

function Close-Notebook($Process) {
    if (!$Process.CloseMainWindow() -or !$Process.WaitForExit(10000) -or $Process.ExitCode -ne 0) {
        throw 'Normal window close failed.'
    }
}

function Check-Files($Root, $Phase, $RelativeDb, $DuringRun) {
    $actual = @(Get-ChildItem -LiteralPath $Root -File -Recurse -Force |
        ForEach-Object { [IO.Path]::GetRelativePath($Root, $_.FullName) } | Sort-Object)
    Write-Host "$Phase files: $($actual -join ', ')"
    $expected = @('bin\paperroom.exe')
    if ($RelativeDb) { $expected += $RelativeDb }
    if ($DuringRun) {
        $expected += "$RelativeDb-wal"
        $expected += "$RelativeDb-shm"
    }
    if (Compare-Object @($expected | Sort-Object) $actual) {
        throw "Unexpected file inventory at $Phase"
    }
}

$base = Join-Path ([IO.Path]::GetTempPath()) ('paperroom-smoke-' + [guid]::NewGuid())
try {
    foreach ($mode in 'default','portable') {
        $root = Join-Path $base $mode
        $bin = Join-Path $root 'bin'
        $local = Join-Path $root 'local'
        $temp = Join-Path $root 'temp'
        foreach ($dir in $bin,$local,$temp) { [void][IO.Directory]::CreateDirectory($dir) }
        Copy-Item -LiteralPath $Exe -Destination $bin
        $portable = $mode -eq 'portable'
        $relative = if ($portable) { 'bin\paperroom-data\notes.sqlite3' } else { 'local\Paperroom\notes.sqlite3' }
        $db = Join-Path $root $relative
        $process = $null
        # root contains only the synthetic bin, local and temp directories.
        $events = [PaperroomFileEvents]::new($root)
        try {
            Check-Files $root "$mode before first launch" $null $false
            $process = Start-Notebook $bin $local $temp $portable
            Check-Files $root "$mode first launch" $relative $true
            $deadline = [DateTime]::UtcNow.AddSeconds(5)
            while (!$events.HasFirstCreation($relative)) {
                if ([DateTime]::UtcNow -gt $deadline) { throw 'Watcher missed synthetic database creation.' }
                Start-Sleep -Milliseconds 50
            }
            if ($Screenshot -and !$portable) {
                try { Save-FirstWindow $process.MainWindowHandle $Screenshot }
                catch { Write-Warning "Optional first-window capture: $_" }
            }
            $events.SetPhase('normal close')
            Close-Notebook $process
            Check-Files $root "$mode normal close" $relative $false

            $events.SetPhase('reopen and edit')
            $process = Start-Notebook $bin $local $temp $portable
            $window = $process.MainWindowHandle
            $marker = "Paperroom recovery $mode"
            [void][PaperroomWindow]::Send($window, 273, [IntPtr]110, [IntPtr]::Zero) # Edit button
            $title = [PaperroomWindow]::GetDlgItem($window, 103)
            [void][PaperroomWindow]::SetText($title, 12, [IntPtr]::Zero, $marker)
            if ((Read-Control $window 103) -ne $marker -or (Read-Control $window 116) -ne '有尚未儲存的修改…') {
                throw 'The UI did not register the synthetic edit.'
            }
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while ((Read-Control $window 116) -ne '已儲存至此電腦') {
                if ([DateTime]::UtcNow -gt $deadline) { throw 'Autosave was not acknowledged.' }
                Start-Sleep -Milliseconds 100
            }
            $events.SetPhase('forced exit')
            $process.Kill()
            $process.WaitForExit()
            Check-Files $root "$mode forced exit" $relative $true
            $events.SetPhase('recovery')
            $process = Start-Notebook $bin $local $temp $portable
            if ((Read-Control $process.MainWindowHandle 103) -ne $marker) {
                throw 'The acknowledged synthetic edit was missing after recovery.'
            }
            Close-Notebook $process
            Check-Files $root "$mode recovery close" $relative $false
            Start-Sleep -Milliseconds 300
            $events.Dispose() # Stop before the separate SQLite reader opens the database.
            python -c "import sqlite3,sys; c=sqlite3.connect(__import__('pathlib').Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True); assert c.execute('pragma integrity_check').fetchone()==('ok',); assert c.execute('select count(*) from notes where title=?',(sys.argv[2],)).fetchone()==(1,); c.close()" $db $marker
            if ($LASTEXITCODE -ne 0) { throw 'Recovered SQLite verification failed.' }
            Write-Host "$mode recovery and SQLite integrity passed."
        } finally {
            if ($process -and !$process.HasExited) { $process.Kill(); $process.WaitForExit() }
            $events.Dispose()
            Write-Host "$mode FileSystemWatcher notifications from bin/local/temp, grouped by delivery phase, kind and relative path. Counts may coalesce; process IDs and paths outside those roots are unavailable."
            foreach ($line in $events.Summary()) { Write-Host $line }
            if ($events.Lost) { throw 'Watcher reported an error or exceeded its event-group limit; results are incomplete.' }
        }
    }
} finally {
    if (Test-Path -LiteralPath $base) { Remove-Item -LiteralPath $base -Recurse -Force }
}
