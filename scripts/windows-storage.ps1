# Requires PowerShell 7, a Windows desktop session and Python 3 with sqlite3.
param([Parameter(Mandatory)][string]$Exe, [string]$Screenshot, [string]$UpgradeBaseline, [switch]$RequireGuideScreenshot)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if ($RequireGuideScreenshot -and (!$Screenshot -or
    [IO.Path]::GetExtension($Screenshot) -ine '.png' -or
    (Test-Path -LiteralPath $Screenshot))) {
    throw 'The strict guide capture requires a new PNG path.'
}
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PaperroomWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
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

function Save-FirstWindow($Window, $Path, [uint32]$ExpectedProcess = 0) {
    Add-Type -AssemblyName System.Drawing
    $rect = [PaperroomWindow+Rect]::new()
    $mode = 2
    if ($ExpectedProcess -ne 0) {
        [uint32]$owner = 0
        if ([PaperroomWindow]::GetWindowThreadProcessId($Window, [ref]$owner) -eq 0 -or
            $owner -ne $ExpectedProcess -or ![PaperroomWindow]::IsWindowVisible($Window) -or
            (Read-Control $Window 103) -cne '歡迎使用紙間') {
            throw 'Guide capture expected the new Paperroom process and its built-in welcome note.'
        }
        if (![PaperroomWindow]::GetClientRect($Window, [ref]$rect)) { throw 'Could not measure the app client area.' }
        $mode = 1 # PW_CLIENTONLY; the app renders into the bitmap, not a desktop capture.
    } elseif (![PaperroomWindow]::GetWindowRect($Window, [ref]$rect)) {
        throw 'Could not measure window.'
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($ExpectedProcess -ne 0 -and ($width -lt 600 -or $height -lt 300 -or $width -gt 3840 -or $height -gt 2160)) {
        throw 'Guide capture has unexpected client dimensions.'
    }
    $bitmap = [Drawing.Bitmap]::new($width, $height)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try {
                if (![PaperroomWindow]::PrintWindow($Window, $dc, $mode)) { throw 'PrintWindow failed.' }
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

function Assert-UpgradeDatabase($Db, $Schema, $Title, $PriorDb = '') {
    # Connected through a read-only, immutable URI only after a graceful close.
    # Reject live sidecars so immutable reads can't conceal unapplied WAL data.
    foreach ($suffix in '-wal','-shm','-journal') {
        if (Test-Path -LiteralPath ($Db + $suffix)) { throw "Database has a live sidecar: $suffix" }
    }
    @'
import sqlite3
import sys
from contextlib import closing
from pathlib import Path

def inspect(database):
    with closing(sqlite3.connect(Path(database).as_uri() + "?mode=ro&immutable=1", uri=True)) as con:
        assert con.execute("pragma integrity_check").fetchall() == [("ok",)]
        assert con.execute("pragma foreign_key_check").fetchall() == []
        version = con.execute("pragma user_version").fetchone()[0]
        if version == 35:
            assert con.execute("SELECT count(*) FROM collection_field_rules").fetchone() == (0,)
            assert con.execute("SELECT count(*) FROM notion_page_paths").fetchone() == (0,)
        notes = con.execute("select id,parent,title,body,pinned,trash_root,version from notes order by id").fetchall()
        cards = con.execute("select note_id,x,y from board_cards order by note_id").fetchall()
        return version, notes, cards

version, notes, cards = inspect(sys.argv[1])
schema, title = int(sys.argv[2]), sys.argv[3]
assert version == schema, (version, schema)
assert sum(row[2] == title for row in notes) == 1
if len(sys.argv) > 4 and sys.argv[4]:
    old_version, old_notes, old_cards = inspect(sys.argv[4])
    assert old_version == 2 and cards == old_cards and len(notes) == len(old_notes)
    for before, after in zip(old_notes, notes):
        if before == after:
            continue
        assert before[0] == after[0] and after[2] == title
        assert before[1] == after[1] and before[3:6] == after[3:6]
        assert after[6] == before[6] + 1
'@ | python - $Db $Schema $Title $PriorDb
    if ($LASTEXITCODE -ne 0) { throw 'Published workspace upgrade SQLite verification failed.' }
}

function Set-UpgradeTitle($Process, $Title) {
    $window = $Process.MainWindowHandle
    if ((Read-Control $window 103) -eq $Title) { throw 'Upgrade marker was not unique.' }
    [void][PaperroomWindow]::Send($window, 273, [IntPtr]110, [IntPtr]::Zero)
    $control = [PaperroomWindow]::GetDlgItem($window, 103)
    [void][PaperroomWindow]::SetText($control, 12, [IntPtr]::Zero, $Title)
    if ((Read-Control $window 103) -cne $Title -or (Read-Control $window 116) -ne '有尚未儲存的修改…') {
        throw 'The native UI did not register its upgrade marker.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ((Read-Control $window 116) -ne '已儲存至此電腦') {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Upgrade marker autosave was not acknowledged.' }
        Start-Sleep -Milliseconds 100
    }
}

function Test-PublishedWorkspaceUpgrade($Root, $Baseline, $Candidate) {
    $Baseline = (Resolve-Path -LiteralPath $Baseline).Path
    $published = '0b281c84d7cf82c03c6262307a2b5b3585ba180e107119dc02b7518f1473de82'
    if ((Get-FileHash -LiteralPath $Baseline).Hash -ine $published) {
        throw 'Upgrade baseline is not the pinned public v0.1.1 executable.'
    }

    $original = Join-Path $Root 'published-v011'
    $upgrade = Join-Path $Root 'candidate-upgrade'
    foreach ($isolated in $original,$upgrade) {
        foreach ($dir in 'bin','local','temp') {
            [void][IO.Directory]::CreateDirectory((Join-Path $isolated $dir))
        }
    }
    Copy-Item -LiteralPath $Baseline -Destination (Join-Path $original 'bin/paperroom.exe')
    Copy-Item -LiteralPath $Candidate -Destination (Join-Path $upgrade 'bin/paperroom.exe')
    $oldDb = Join-Path $original 'bin/paperroom-data/notes.sqlite3'
    $newDb = Join-Path $upgrade 'bin/paperroom-data/notes.sqlite3'
    $oldTitle = '紙間 v0.1.1 升級原本'
    $newTitle = '紙間升級測試只改副本'
    $process = $null
    try {
        $process = Start-Notebook (Join-Path $original 'bin') (Join-Path $original 'local') (Join-Path $original 'temp') $true
        Set-UpgradeTitle $process $oldTitle
        Close-Notebook $process
        $originalHash = (Get-FileHash -LiteralPath $oldDb).Hash
        Assert-UpgradeDatabase $oldDb 2 $oldTitle
        [void][IO.Directory]::CreateDirectory((Split-Path $newDb -Parent))
        Copy-Item -LiteralPath $oldDb -Destination $newDb
        if ((Get-FileHash -LiteralPath $newDb).Hash -cne $originalHash) { throw 'Upgrade input copy changed bytes.' }

        $process = Start-Notebook (Join-Path $upgrade 'bin') (Join-Path $upgrade 'local') (Join-Path $upgrade 'temp') $false
        if ((Read-Control $process.MainWindowHandle 103) -cne $oldTitle) { throw 'Candidate did not open the published original note.' }
        Assert-UpgradeDatabase $oldDb 2 $oldTitle
        Set-UpgradeTitle $process $newTitle
        Close-Notebook $process
        Assert-UpgradeDatabase $newDb 35 $newTitle $oldDb
        $process = Start-Notebook (Join-Path $upgrade 'bin') (Join-Path $upgrade 'local') (Join-Path $upgrade 'temp') $false
        if ((Read-Control $process.MainWindowHandle 103) -cne $newTitle) { throw 'Candidate did not retain the upgraded note on reopen.' }
        Close-Notebook $process
        Assert-UpgradeDatabase $newDb 35 $newTitle $oldDb
        Assert-UpgradeDatabase $oldDb 2 $oldTitle
        if ((Get-FileHash -LiteralPath $oldDb).Hash -cne $originalHash) { throw 'Published original database bytes changed.' }
        Write-Host 'Pinned v0.1.1 native UI → schema 2 → migration of exact copy → schema 35 → native UI save/reopen, integrity, and original byte preservation passed.'
    } finally {
        if ($process -and !$process.HasExited) { $process.Kill(); $process.WaitForExit() }
    }
}

$base = Join-Path ([IO.Path]::GetTempPath()) ('paperroom-smoke-' + [guid]::NewGuid())
try {
    # Refuse ambiguous startup instead of creating state in an unintended folder.
    foreach ($invalidMode in 'missing-local','relative-local','marker-conflict') {
        $invalidRoot = Join-Path $base ('invalid-' + [guid]::NewGuid())
        $bin = Join-Path $invalidRoot 'bin'
        $temp = Join-Path $invalidRoot 'temp'
        $working = Join-Path $invalidRoot 'working'
        foreach ($dir in $bin,$temp,$working) { [void][IO.Directory]::CreateDirectory($dir) }
        Copy-Item -LiteralPath $Exe -Destination $bin
        $info = [Diagnostics.ProcessStartInfo]::new((Join-Path $bin 'paperroom.exe'))
        $info.UseShellExecute = $false
        $info.WorkingDirectory = $working
        $info.Environment['TEMP'] = $temp
        $info.Environment['TMP'] = $temp
        $markerConflict = Join-Path $bin 'paperroom-data'
        switch ($invalidMode) {
            'missing-local' { [void]$info.Environment.Remove('LOCALAPPDATA') }
            'relative-local' { $info.Environment['LOCALAPPDATA'] = 'relative-local' }
            'marker-conflict' {
                $info.Environment['LOCALAPPDATA'] = Join-Path $invalidRoot 'local'
                [IO.File]::WriteAllText($markerConflict, 'preserve the existing file')
            }
        }
        $bad = [Diagnostics.Process]::Start($info)
        try {
            $sawDialog = $false
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
            while (!$bad.HasExited) {
                $bad.Refresh()
                if ($bad.MainWindowTitle -eq '紙間無法啟動' -and
                    $bad.MainWindowHandle -ne [IntPtr]::Zero) {
                    $sawDialog = $true
                    [void]$bad.CloseMainWindow()
                    break
                }
                if ([DateTime]::UtcNow -gt $deadline) { throw "$invalidMode did not produce the startup error dialog." }
                Start-Sleep -Milliseconds 100
            }
            if (!$sawDialog) { throw "$invalidMode exited without an observed startup error dialog." }
            if (!$bad.WaitForExit(10000) -or $bad.ExitCode -ne 1) { throw "$invalidMode should exit with status 1." }
            if ($invalidMode -eq 'marker-conflict') {
                if ([IO.File]::ReadAllText($markerConflict) -cne 'preserve the existing file') {
                    throw 'An invalid portable marker was modified.'
                }
                [IO.File]::Delete($markerConflict)
            }
            Check-Files $invalidRoot "$invalidMode after failed startup" $null $false
            $extraDirs = @(Get-ChildItem -LiteralPath $invalidRoot -Directory -Recurse -Force |
                ForEach-Object { [IO.Path]::GetRelativePath($invalidRoot, $_.FullName) } |
                Where-Object { $_ -notin @('bin','temp','working') })
            if ($extraDirs.Count -ne 0) { throw 'Invalid startup created additional directories.' }
        } finally {
            if (!$bad.HasExited) { $bad.Kill(); $bad.WaitForExit() }
            $bad.Dispose()
        }
    }
    if ($UpgradeBaseline) { Test-PublishedWorkspaceUpgrade $base $UpgradeBaseline $Exe }
    foreach ($mode in 'default','portable','portable-marker') {
        $root = Join-Path $base $mode
        $bin = Join-Path $root 'bin'
        $local = Join-Path $root 'local'
        $temp = Join-Path $root 'temp'
        foreach ($dir in $bin,$local,$temp) { [void][IO.Directory]::CreateDirectory($dir) }
        Copy-Item -LiteralPath $Exe -Destination $bin
        $portable = $mode -eq 'portable' # Pass --portable only in the explicit mode.
        if ($mode -eq 'portable-marker') {
            [void][IO.Directory]::CreateDirectory((Join-Path $bin 'paperroom-data'))
        }
        $relative = if ($mode -ne 'default') { 'bin\paperroom-data\notes.sqlite3' } else { 'local\Paperroom\notes.sqlite3' }
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
            if ($Screenshot -and $mode -eq 'default') {
                if ($RequireGuideScreenshot) {
                    Save-FirstWindow $process.MainWindowHandle $Screenshot $process.Id
                    if (!(Test-Path -LiteralPath $Screenshot -PathType Leaf)) { throw 'No client PNG was written.' }
                    Write-Host 'Captured the new Paperroom client area displaying the built-in welcome note.'
                } else {
                    try { Save-FirstWindow $process.MainWindowHandle $Screenshot }
                    catch { Write-Warning "Optional first-window capture: $_" }
                }
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
