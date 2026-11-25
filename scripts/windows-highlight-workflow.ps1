# Requires PowerShell 7, a Windows desktop and Python 3 with sqlite3.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('\A[0-9a-fA-F]{64}\z')][string]$ExeSha256
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows -or $PSVersionTable.PSVersion.Major -lt 7 -or
    ![Environment]::UserInteractive -or ([Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0)) {
    throw 'A Windows desktop session with PowerShell 7 is required.'
}
$Exe = (Resolve-Path -LiteralPath $Exe).ProviderPath
function Assert-Hash([string]$Path) {
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ine $ExeSha256) { throw 'EXE SHA-256 mismatch.' }
}
Assert-Hash $Exe
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class PaperroomHighlightGui {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    private struct GuiThread {
        public uint Size, Flags;
        public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput {
        public int X, Y;
        public uint Data, Flags, Time;
        public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct KeyInput {
        public ushort VirtualKey, Scan;
        public uint Flags, Time;
        public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Explicit)]
    private struct InputData {
        [FieldOffset(0)] public MouseInput Mouse;
        [FieldOffset(0)] public KeyInput Key;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input {
        public uint Kind;
        public InputData Data;
    }
    private delegate bool EnumWindow(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumThreadWindows(uint thread, EnumWindow callback, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, EnumWindow callback, IntPtr state);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect region);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr dc, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr hwnd, uint relation);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread, ref GuiThread info);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint SendInput(uint count, Input[] input, int size);
    [DllImport("user32.dll")] private static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll", ExactSpelling=true, CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int limit);
    [DllImport("user32.dll", ExactSpelling=true, CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int limit);
    [DllImport("user32.dll", ExactSpelling=true)] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", ExactSpelling=true)] private static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l, uint flags, uint millis, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, ExactSpelling=true)]
    private static extern IntPtr GetText(IntPtr hwnd, uint msg, IntPtr capacity, StringBuilder text, uint flags, uint millis, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, ExactSpelling=true)]
    private static extern IntPtr SetText(IntPtr hwnd, uint msg, IntPtr w, string text, uint flags, uint millis, out IntPtr result);
    [DllImport("user32.dll")] private static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")] private static extern uint GetMenuItemID(IntPtr menu, int item);
    [DllImport("user32.dll", ExactSpelling=true, CharSet=CharSet.Unicode)]
    private static extern int GetMenuStringW(IntPtr menu, uint item, StringBuilder text, int limit, uint flags);

    public static IntPtr Send(IntPtr hwnd, uint msg, IntPtr w, IntPtr l) {
        if (hwnd == IntPtr.Zero || SendMessageTimeoutW(hwnd, msg, w, l, 0x23, 3000, out IntPtr result) == IntPtr.Zero)
            throw new InvalidOperationException("Native message timed out: " + msg);
        return result;
    }
    public static string Read(IntPtr hwnd) {
        var text = new StringBuilder(8192);
        if (hwnd == IntPtr.Zero || GetText(hwnd, 13, (IntPtr)text.Capacity, text, 0x23, 3000, out IntPtr length) == IntPtr.Zero ||
            length.ToInt64() >= text.Capacity - 1) throw new InvalidOperationException("Native text missing or truncated.");
        return text.ToString();
    }
    public static void Write(IntPtr hwnd, string text) {
        if (hwnd == IntPtr.Zero || SetText(hwnd, 12, IntPtr.Zero, text, 0x23, 3000, out IntPtr changed) == IntPtr.Zero || changed == IntPtr.Zero)
            throw new InvalidOperationException("Unable to set native edit text.");
    }
    public static string Caption(IntPtr hwnd) {
        var text = new StringBuilder(256);
        GetWindowTextW(hwnd, text, text.Capacity);
        return text.ToString();
    }
    private static string Kind(IntPtr hwnd) {
        var text = new StringBuilder(128);
        GetClassNameW(hwnd, text, text.Capacity);
        return text.ToString();
    }
    public static IntPtr OwnedWindow(IntPtr owner, uint pid, string kind, string title, int button) {
        uint thread = GetWindowThreadProcessId(owner, out uint found);
        if (thread == 0 || found != pid) return IntPtr.Zero;
        var matches = new List<IntPtr>();
        EnumThreadWindows(thread, (hwnd, ignored) => {
            if (!IsWindowVisible(hwnd) || Kind(hwnd) != kind ||
                GetWindowThreadProcessId(hwnd, out uint ownPid) != thread || ownPid != pid) return true;
            if (kind == "#32770" && (GetWindow(hwnd, 4) != owner ||
                (!String.IsNullOrEmpty(title) && Caption(hwnd) != title) ||
                (button != 0 && GetDlgItem(hwnd, button) == IntPtr.Zero))) return true;
            matches.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        if (matches.Count > 1) throw new InvalidOperationException("Ambiguous owned " + kind + " window.");
        return matches.Count == 1 ? matches[0] : IntPtr.Zero;
    }
    public static bool ForegroundIsOwned(IntPtr owner, uint pid) {
        uint thread = GetWindowThreadProcessId(owner, out uint owningPid);
        return thread != 0 && owningPid == pid &&
            GetWindowThreadProcessId(GetForegroundWindow(), out uint foregroundPid) == thread &&
            foregroundPid == pid;
    }
    public static string DescribeWindows(IntPtr owner, uint pid) {
        var text = new StringBuilder();
        IntPtr foreground = GetForegroundWindow();
        GetWindowThreadProcessId(foreground, out uint foregroundPid);
        text.Append("Foreground pid=").Append(foregroundPid).Append(" class=").Append(Kind(foreground));
        uint thread = GetWindowThreadProcessId(owner, out uint owningPid);
        if (thread == 0 || owningPid != pid) return text.Append("; owned main window unavailable").ToString();
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        if (GetGUIThreadInfo(thread, ref gui)) {
            text.Append("; GUI flags=0x").Append(gui.Flags.ToString("X")).Append(" menuOwner=").Append(Kind(gui.MenuOwner));
        }
        EnumThreadWindows(thread, (hwnd, ignored) => {
            if (IsWindowVisible(hwnd) && GetWindowThreadProcessId(hwnd, out uint actual) != 0 && actual == pid) {
                text.Append("; owned ").Append(Kind(hwnd)).Append(" [").Append(Caption(hwnd)).Append(']');
            }
            return true;
        }, IntPtr.Zero);
        return text.ToString();
    }
    private static Input Key(ushort key, bool up) {
        uint flags = key == 0x24 || key == 0x28 ? 1u : 0u; // Extended Home / Down.
        if (up) flags |= 2; // KEYEVENTF_KEYUP.
        return new Input { Kind = 1, Data = new InputData { Key = new KeyInput { VirtualKey = key, Flags = flags } } };
    }
    public static void SelectMenu(IntPtr owner, uint pid, IntPtr popup, uint itemId, string caption) {
        if (OwnedWindow(owner, pid, "#32768", null, 0) != popup || popup == IntPtr.Zero)
            throw new InvalidOperationException("The verified native menu is no longer open.");
        int position = MenuPosition(popup, itemId, caption);
        foreach (int key in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(key) < 0) throw new InvalidOperationException("An input modifier is held; refusing to send menu keys.");
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        uint thread = GetWindowThreadProcessId(owner, out uint ignoredPid);
        if (!GetGUIThreadInfo(thread, ref gui) || (gui.Flags & 0x14) == 0 || gui.MenuOwner != owner)
            throw new InvalidOperationException("The isolated app is not the current native menu owner.");
        var keys = new List<Input> { Key(0x24, false), Key(0x24, true) }; // Home.
        for (int i = 0; i < position; i++) { keys.Add(Key(0x28, false)); keys.Add(Key(0x28, true)); }
        keys.Add(Key(0x0D, false)); keys.Add(Key(0x0D, true)); // Enter.
        if (!ForegroundIsOwned(owner, pid))
            throw new InvalidOperationException("The isolated test app lost foreground before menu input.");
        int size = Marshal.SizeOf<Input>();
        if (size != (IntPtr.Size == 8 ? 40 : 28)) throw new InvalidOperationException("Unexpected Windows INPUT size.");
        uint actual = SendInput((uint)keys.Count, keys.ToArray(), size);
        if (actual != keys.Count)
            throw new InvalidOperationException("SendInput delivered " + actual + "/" + keys.Count + " keyboard changes; Win32 error " + Marshal.GetLastWin32Error() + ".");
    }
    private static int MenuPosition(IntPtr popup, uint itemId, string caption) {
        // Native popup menu windows return their HMENU for MN_GETHMENU.
        IntPtr menu = Send(popup, 0x01E1, IntPtr.Zero, IntPtr.Zero);
        int count = GetMenuItemCount(menu);
        if (count < 1 || count > 64) throw new InvalidOperationException("Unexpected native menu.");
        for (int index = 0; index < count; index++) {
            if (GetMenuItemID(menu, index) != itemId) continue;
            var title = new StringBuilder(256);
            GetMenuStringW(menu, (uint)index, title, title.Capacity, 0x400);
            if (title.ToString() != caption) throw new InvalidOperationException("Unexpected menu label: " + title);
            if ((GetMenuState(menu, (uint)index, 0x400) & 3) != 0)
                throw new InvalidOperationException("The required native menu item is disabled.");
            return index;
        }
        throw new InvalidOperationException("The required menu item is absent: " + caption);
    }
    public static IntPtr FilenameField(IntPtr dialog, uint pid) {
        var matches = new List<IntPtr>();
        EnumChildWindows(dialog, (hwnd, ignored) => {
            if (!IsWindowVisible(hwnd) || !String.Equals(Kind(hwnd), "Edit", StringComparison.OrdinalIgnoreCase) ||
                GetWindowThreadProcessId(hwnd, out uint found) == 0 || found != pid) return true;
            for (IntPtr test = hwnd; test != IntPtr.Zero && test != dialog; test = GetParent(test)) {
                int id = GetDlgCtrlID(test);
                if (id == 0x480 || id == 0x47c) { matches.Add(hwnd); break; } // edt1 / cmb13, the file name field.
            }
            return true;
        }, IntPtr.Zero);
        if (matches.Count != 1) throw new InvalidOperationException("Could not locate a unique file name edit.");
        return matches[0];
    }
}
'@

function Wait-For([string]$Description, [scriptblock]$Probe) {
    $deadline = [DateTime]::UtcNow.AddSeconds(12)
    do {
        $value = & $Probe
        if ($value -and ($value -isnot [IntPtr] -or $value -ne [IntPtr]::Zero)) { return $value }
        Start-Sleep -Milliseconds 80
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out: $Description"
}
function Control([IntPtr]$Owner, [int]$Id) {
    $field = [PaperroomHighlightGui]::GetDlgItem($Owner, $Id)
    if ($field -eq [IntPtr]::Zero) { throw "Expected control $Id is missing." }
    return $field
}
function Command([IntPtr]$Owner, [int]$Id) {
    if (![PaperroomHighlightGui]::PostMessageW($Owner, 0x111, [IntPtr]$Id, (Control $Owner $Id))) {
        throw "Could not invoke control $Id."
    }
}
function Native-Menu([int]$OwnerControl, [uint32]$Item, [string]$Caption) {
    if (![PaperroomHighlightGui]::ForegroundIsOwned($window, $process.Id)) {
        [void][PaperroomHighlightGui]::SetForegroundWindow($window)
        $null = Wait-For 'isolated app foreground' { [PaperroomHighlightGui]::ForegroundIsOwned($window, $process.Id) }
    }
    Command $window $OwnerControl
    $menu = Wait-For "menu containing $Caption" { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32768', $null, 0) }
    [PaperroomHighlightGui]::SelectMenu($window, $process.Id, $menu, $Item, $Caption)
    Write-Host "Verified native menu command $Item and sent Windows keyboard input for $Caption."
    $null = Wait-For "native menu closed after $Caption" { ![PaperroomHighlightGui]::IsWindowVisible($menu) }
}
function Native-File([uint32]$Item, [string]$Caption, [string]$Path, [string]$Status) {
    Native-Menu 109 $Item $Caption
    $dialog = Wait-For "common file dialog for $Caption" { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $null, 1) }
    $edit = [PaperroomHighlightGui]::FilenameField($dialog, $process.Id)
    Write-Host "Verified the common file dialog for $Caption."
    [PaperroomHighlightGui]::Write($edit, $Path)
    if ([PaperroomHighlightGui]::Read($edit) -cne $Path) { throw 'File dialog edited an unexpected path.' }
    Command $dialog 1
    $null = Wait-For "$Caption completed" { [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith($Status, [StringComparison]::Ordinal) }
}
function Start-Notebook([string]$Folder) {
    $binary = Join-Path $Folder 'Paperroom.exe'
    Assert-Hash $binary
    $info = [Diagnostics.ProcessStartInfo]::new($binary, '--portable')
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $Folder
    foreach ($key in 'LOCALAPPDATA','TEMP','TMP') {
        $info.Environment[$key] = Join-Path $Folder $key
        $null = [IO.Directory]::CreateDirectory($info.Environment[$key])
    }
    $started = [Diagnostics.Process]::Start($info)
    try {
        $null = Wait-For 'Paperroom note window' {
            $started.Refresh()
            if ($started.HasExited) { throw "Early EXE exit: $($started.ExitCode)" }
            $candidate = $started.MainWindowHandle
            [uint32]$pidFound = 0
            if ([PaperroomHighlightGui]::GetWindowThreadProcessId($candidate, [ref]$pidFound) -and
                $pidFound -eq $started.Id -and [PaperroomHighlightGui]::GetDlgItem($candidate, 103) -ne [IntPtr]::Zero) {
                $candidate
            }
        }
        return $started
    } catch {
        if (!$started.HasExited) { $started.Kill(); [void]$started.WaitForExit(5000) }
        $started.Dispose()
        throw
    }
}
function Close-Notebook($Notebook) {
    if (!$Notebook.CloseMainWindow() -or !$Notebook.WaitForExit(10000) -or $Notebook.ExitCode -ne 0) {
        throw 'Normal close failed for the isolated EXE.'
    }
}
function Select-Note([string]$Title) {
    $search = Control $window 101
    [PaperroomHighlightGui]::Write($search, $Title)
    $tree = Control $window 102
    $first = [PaperroomHighlightGui]::Send($tree, 0x110A, [IntPtr]::Zero, [IntPtr]::Zero) # TVGN_ROOT
    if ($first -eq [IntPtr]::Zero) { throw 'The synthetic note is absent from the tree.' }
    [void][PaperroomHighlightGui]::Send($tree, 0x110B, [IntPtr]9, $first) # TVGN_CARET
    $null = Wait-For 'synthetic note selected' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $Title }
}
function Open-Editor {
    Command $window 110
    $field = Control $window 104
    $null = Wait-For 'native source editor visible' { [PaperroomHighlightGui]::IsWindowVisible($field) }
    return $field
}
function Read-PreviewMark([bool]$Expected) {
    $preview = Control $window 121
    $null = Wait-For 'native RichEdit preview visible' { [PaperroomHighlightGui]::IsWindowVisible($preview) }
    if (![PaperroomHighlightGui]::Read($preview).Contains($selected)) { throw 'The RichEdit preview does not contain the selected text.' }
    $rect = [PaperroomHighlightGui+Rect]::new()
    if (![PaperroomHighlightGui]::GetClientRect($preview, [ref]$rect) -or
        $rect.Right -lt 100 -or $rect.Bottom -lt 100 -or $rect.Right -gt 3840 -or $rect.Bottom -gt 2160) {
        throw 'Unexpected RichEdit client dimensions.'
    }
    $bitmap = [Drawing.Bitmap]::new($rect.Right, $rect.Bottom)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try {
                if (![PaperroomHighlightGui]::PrintWindow($preview, $dc, 1)) { throw 'RichEdit client capture failed.' }
            } finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $mark = [Drawing.Color]::FromArgb(255,232,131).ToArgb() -band 0xFFFFFF
        $colors = [Collections.Generic.HashSet[int]]::new()
        $hits = 0
        for ($y = 1; $y -lt $rect.Bottom; $y += 3) {
            for ($x = 1; $x -lt $rect.Right; $x += 3) {
                $value = $bitmap.GetPixel($x, $y).ToArgb() -band 0xFFFFFF
                if ($value -eq $mark) { $hits++ }
                [void]$colors.Add($value)
            }
        }
        if ($colors.Count -lt 3 -or ($Expected -and $hits -lt 5) -or (!$Expected -and $hits -ne 0)) {
            throw "Unexpected RichEdit capture (colors=$($colors.Count), highlight pixels=$hits, expected=$Expected)."
        }
        Write-Host "RichEdit screenshot samples: $hits highlight pixels; expected visible=$Expected."
    } finally { $bitmap.Dispose() }
}

function Assert-Database([string]$Database, [bool]$Marked, [string]$ExportZip = '') {
    $markValue = if ($Marked) { 'yes' } else { 'no' }
    $zipValue = if ($ExportZip) { $ExportZip } else { '-' }
    @'
from pathlib import Path
from contextlib import closing
from zipfile import ZipFile
import sqlite3, sys
database, source, title, selected, marked, export = sys.argv[1:]
raw = Path(source).read_bytes()
text = selected.encode("utf-8")
assert raw.count(text) == 1, "ambiguous synthetic range"
begin = raw.index(text)
with closing(sqlite3.connect(Path(database).as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
    rows = db.execute("SELECT id, CAST(body AS BLOB), version FROM notes WHERE title=? AND trash_root IS NULL",
                      (title,)).fetchall()
    assert len(rows) == 1
    note, body, version = rows[0]
    assert body == raw and version == 1, "highlight changed original Markdown bytes or note revision"
    marks = db.execute("SELECT start_byte,end_byte FROM note_highlights WHERE note_id=?", (note,)).fetchall()
    assert marks == ([(begin, begin + len(text))] if marked == "yes" else []), marks
if export != "-":
    with ZipFile(export) as archive:
        assert archive.testzip() is None
        assert archive.read(f"markdown/{note}.md") == raw, "HTML ZIP changed raw Markdown"
        html = archive.read(f"pages/{note}.html")
        assert html.count(b"<mark>") == 1 and b"<mark>" + text + b"</mark>" in html
        assert "==\u4fdd\u7559==".encode("utf-8") in html, "unmarked literal was changed"
print(f"Read-only SQLite check: note version {version}, exact raw bytes, {len(marks)} exact byte range(s).")
'@ | python - $Database $source $title $selected $markValue $zipValue
    if ($LASTEXITCODE -ne 0) { throw 'SQLite or HTML ZIP verification failed.' }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('paperroom-highlight-' + [guid]::NewGuid().ToString('N'))
$live = Join-Path $root 'live'
$restored = Join-Path $root 'restored'
$source = Join-Path $root 'QA Window Highlight.md'
$backup = Join-Path $root 'backup.sqlite3'
$restoreDb = Join-Path $restored 'paperroom-data\notes.sqlite3'
$liveDb = Join-Path $live 'paperroom-data\notes.sqlite3'
$markdown = Join-Path $root 'raw.md'
$html = Join-Path $root 'workspace.zip'
$title = 'QA Window Highlight'
$selected = '選取📝'
$process = $null
$window = [IntPtr]::Zero
try {
    foreach ($folder in $live,$restored) {
        [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
        Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
    }
    $manuscript = "---`nqa: highlight`n---`n`n前方 📘選取📝 後方`n`n==保留==`n"
    [IO.File]::WriteAllText($source, $manuscript, [Text.UTF8Encoding]::new($false))
    Write-Host "Exercise exact Windows EXE SHA-256 $ExeSha256."
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-File 1 '匯入 Markdown…' $source '已匯入 1 篇 Markdown'
    $field = Open-Editor
    $editor = [PaperroomHighlightGui]::Read($field)
    if ($editor.Replace("`r`n", "`n") -cne $manuscript) { throw 'The native editor did not load the synthetic Markdown.' }
    $start = $editor.IndexOf($selected, [StringComparison]::Ordinal)
    if ($start -lt 0) { throw 'Synthetic selection was absent.' }
    [void][PaperroomHighlightGui]::Send($field, 0x00B1, [IntPtr]$start, [IntPtr]($start + $selected.Length))
    Native-Menu 113 25 '切換選取文字的螢光標記'
    $null = Wait-For 'mark acknowledgment' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已加上螢光標記。切換閱讀可查看。' }
    if ([PaperroomHighlightGui]::Read($field) -cne $editor) { throw 'Mark action changed editor text.' }
    Command $window 110
    Read-PreviewMark $true
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Assert-Database $liveDb $true

    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note $title
    Read-PreviewMark $true
    Native-File 3 '匯出目前筆記為 Markdown…' $markdown 'Markdown 已匯出'
    if ((Get-FileHash -LiteralPath $markdown).Hash -cne (Get-FileHash -LiteralPath $source).Hash) {
        throw 'Standalone Markdown export changed the original bytes.'
    }
    Native-File 4 '備份工作區…' $backup '工作區備份已完成'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Assert-Database $liveDb $true
    Assert-Database $backup $true

    $originalBackupHash = (Get-FileHash -LiteralPath $backup).Hash
    Copy-Item -LiteralPath $backup -Destination $restoreDb
    $process = Start-Notebook $restored
    $window = $process.MainWindowHandle
    Select-Note $title
    Read-PreviewMark $true
    Native-File 30 '匯出工作區 HTML ZIP…' $html '已匯出 '
    $null = Open-Editor
    Native-Menu 113 26 '清除本篇螢光標記…'
    $cancel = Wait-For 'clear-highlight confirmation' { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '清除螢光標記', 7) }
    Command $cancel 7
    $null = Wait-For 'clear canceled' { ![PaperroomHighlightGui]::IsWindowVisible($cancel) }
    Command $window 110
    Read-PreviewMark $true
    $null = Open-Editor
    Native-Menu 113 26 '清除本篇螢光標記…'
    $confirm = Wait-For 'clear-highlight confirmation' { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '清除螢光標記', 6) }
    Command $confirm 6
    $null = Wait-For 'clear acknowledgment' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已清除 1 個螢光標記。' }
    Command $window 110
    Read-PreviewMark $false
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Assert-Database $restoreDb $false
    Assert-Database $backup $true $html
    Assert-Database $liveDb $true
    if ((Get-FileHash -LiteralPath $backup).Hash -cne $originalBackupHash) { throw 'Backup bytes changed.' }
    Write-Host 'PASS: native highlight, preview, normal reopen, exact Markdown, Backup, isolated restore, HTML and cancel/clear.'
} catch {
    if ($process -and !$process.HasExited -and $window -ne [IntPtr]::Zero) {
        Write-Host ("Native GUI diagnostics: " + [PaperroomHighlightGui]::DescribeWindows($window, $process.Id))
    }
    throw
} finally {
    if ($process) {
        if (!$process.HasExited) { $process.Kill(); [void]$process.WaitForExit(5000) }
        $process.Dispose()
    }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
