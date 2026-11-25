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
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo { public uint Size; public Rect Monitor, Work; public uint Flags; }
    public struct MenuSelection { public Point Point; public int Scroll; }
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
    private struct Input { public uint Kind; public MouseInput Mouse; }
    private delegate bool EnumWindow(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumThreadWindows(uint thread, EnumWindow callback, IntPtr state);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, EnumWindow callback, IntPtr state);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect region);
    [DllImport("user32.dll")] private static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr hwnd, uint relation);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread, ref GuiThread info);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll", SetLastError=true)]
    private static extern uint SendInput(uint count, Input[] input, int size);
    [DllImport("user32.dll")] private static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] private static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint item, out Rect rect);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
    [DllImport("user32.dll", ExactSpelling=true)]
    private static extern bool GetMonitorInfoW(IntPtr monitor, ref MonitorInfo info);
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
    private static Point Center(Rect rect) {
        return new Point { X = rect.Left + (rect.Right - rect.Left) / 2, Y = rect.Top + (rect.Bottom - rect.Top) / 2 };
    }
    private static bool Contains(Rect rect, Point point) {
        return point.X >= rect.Left && point.X < rect.Right && point.Y >= rect.Top && point.Y < rect.Bottom;
    }
    private static Rect VisibleRect(IntPtr hwnd) {
        if (!GetWindowRect(hwnd, out Rect rect) || rect.Right <= rect.Left || rect.Bottom <= rect.Top)
            throw new InvalidOperationException("The native window has no real rectangle.");
        var monitor = new MonitorInfo { Size = (uint)Marshal.SizeOf<MonitorInfo>() };
        if (!GetMonitorInfoW(MonitorFromWindow(hwnd, 2), ref monitor))
            throw new InvalidOperationException("Could not locate the native window monitor.");
        return new Rect {
            Left = Math.Max(rect.Left, monitor.Monitor.Left), Top = Math.Max(rect.Top, monitor.Monitor.Top),
            Right = Math.Min(rect.Right, monitor.Monitor.Right), Bottom = Math.Min(rect.Bottom, monitor.Monitor.Bottom)
        };
    }
    public static Rect PreviewArea(IntPtr owner, uint pid, IntPtr preview) {
        if (!ForegroundIsOwned(owner, pid) || !IsWindowVisible(preview) ||
            GetParent(preview) != owner || GetDlgCtrlID(preview) != 121 ||
            GetWindowThreadProcessId(preview, out uint controlPid) == 0 || controlPid != pid ||
            !String.Equals(Kind(preview), "RichEdit50W", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Expected the isolated foreground app's visible RichEdit preview.");
        if (!GetClientRect(preview, out Rect client) || client.Right < 108 || client.Bottom < 108)
            throw new InvalidOperationException("The RichEdit client is too small.");
        var start = new Point { X = 4, Y = 4 };
        var end = new Point { X = Math.Min(client.Right - 4, 1104), Y = Math.Min(client.Bottom - 4, 424) };
        if (!ClientToScreen(preview, ref start) || !ClientToScreen(preview, ref end))
            throw new InvalidOperationException("Unable to map the preview from client to screen coordinates.");
        var outside = VisibleRect(preview);
        var last = new Point { X = end.X - 1, Y = end.Y - 1 };
        if (end.X - start.X < 100 || end.Y - start.Y < 100 ||
            end.X - start.X > 1100 || end.Y - start.Y > 420 ||
            !Contains(outside, start) || !Contains(outside, last))
            throw new InvalidOperationException("The mapped preview region is clipped or outside its monitor.");
        for (int y = 0; y <= 6; y++)
            for (int x = 0; x <= 6; x++) {
                var point = new Point {
                    X = start.X + x * (last.X - start.X) / 6,
                    Y = start.Y + y * (last.Y - start.Y) / 6
                };
                if (WindowFromPoint(point) != preview)
                    throw new InvalidOperationException("An unrelated window obscures the RichEdit sampling area.");
            }
        return new Rect { Left = start.X, Top = start.Y, Right = end.X, Bottom = end.Y };
    }
    public static Point Cursor() {
        if (!GetCursorPos(out Point point)) throw new InvalidOperationException("Could not read the pointer.");
        return point;
    }
    public static void RestoreCursor(Point original, Point expected) {
        if (GetCursorPos(out Point current) && current.X == expected.X && current.Y == expected.Y)
            SetCursorPos(original.X, original.Y);
    }
    public static Point ButtonPoint(IntPtr owner, uint pid, IntPtr button) {
        if (GetParent(button) != owner ||
            GetWindowThreadProcessId(button, out uint buttonPid) == 0 || buttonPid != pid ||
            !IsWindowVisible(button) || Kind(button) != "Button")
            throw new InvalidOperationException("Expected an owned native toolbar button.");
        var rect = VisibleRect(button);
        Point point = Center(rect);
        if (!Contains(rect, point) || WindowFromPoint(point) != button)
            throw new InvalidOperationException("The toolbar button is not exposed at its center.");
        return point;
    }
    public static MenuSelection PlanMenu(IntPtr owner, uint pid, IntPtr popup, uint itemId, string caption) {
        if (popup == IntPtr.Zero || OwnedWindow(owner, pid, "#32768", null, 0) != popup)
            throw new InvalidOperationException("The verified native menu is no longer open.");
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        uint thread = GetWindowThreadProcessId(owner, out uint ignoredPid);
        if (!GetGUIThreadInfo(thread, ref gui) || (gui.Flags & 0x14) == 0 || gui.MenuOwner != owner)
            throw new InvalidOperationException("The isolated app is not the current native menu owner.");
        IntPtr menu = Send(popup, 0x01E1, IntPtr.Zero, IntPtr.Zero); // MN_GETHMENU for inspected #32768.
        int position = MenuPosition(menu, itemId, caption);
        Rect menuRect = VisibleRect(popup);
        // NULL selects the popup's screen-space item geometry.
        bool hasItem = GetMenuItemRect(IntPtr.Zero, menu, (uint)position, out Rect row);
        Point point = Center(row);
        if (hasItem && row.Right > row.Left && row.Bottom > row.Top &&
            Contains(menuRect, point) && WindowFromPoint(point) == popup)
            return new MenuSelection { Point = point };

        int above = -1, below = -1;
        for (int i = 0; i < GetMenuItemCount(menu); i++) {
            if (!GetMenuItemRect(IntPtr.Zero, menu, (uint)i, out Rect visibleRow)) continue;
            Point center = Center(visibleRow);
            if (!Contains(menuRect, center) || WindowFromPoint(center) != popup) continue;
            if (i < position) above = i;
            if (i > position && below < 0) below = i;
        }
        int scroll = above >= 0 && below < 0 ? -1 : below >= 0 && above < 0 ? 1 : 0;
        if (scroll == 0 || menuRect.Bottom - menuRect.Top < 24)
            throw new InvalidOperationException("No verified visible rectangle or scroll direction for menu item " + itemId + ".");
        point = new Point { X = Center(menuRect).X, Y = scroll < 0 ? menuRect.Bottom - 7 : menuRect.Top + 7 };
        if (!Contains(menuRect, point) || WindowFromPoint(point) != popup)
            throw new InvalidOperationException("The clipped menu scroll edge is obscured.");
        return new MenuSelection { Point = point, Scroll = scroll };
    }
    public static void MouseAction(IntPtr owner, uint pid, IntPtr target, Point point, int scroll) {
        foreach (int key in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(key) < 0) throw new InvalidOperationException("An input modifier is held.");
        if (!ForegroundIsOwned(owner, pid) || GetWindowThreadProcessId(target, out uint targetPid) == 0 ||
            targetPid != pid || WindowFromPoint(point) != target)
            throw new InvalidOperationException("The physical input point is no longer on the foreground test window.");
        if (!SetCursorPos(point.X, point.Y)) throw new InvalidOperationException("Could not move to the verified input point.");
        Point actualPoint = Cursor();
        if (!ForegroundIsOwned(owner, pid) || WindowFromPoint(actualPoint) != target ||
            actualPoint.X != point.X || actualPoint.Y != point.Y)
            throw new InvalidOperationException("The pointer target changed before physical input.");
        var input = scroll == 0
            ? new[] { new Input { Mouse = new MouseInput { Flags = 0x0002 } },
                      new Input { Mouse = new MouseInput { Flags = 0x0004 } } }
            : new[] { new Input { Mouse = new MouseInput { Flags = 0x0800, Data = unchecked((uint)(scroll * 120)) } } };
        int size = Marshal.SizeOf<Input>();
        if (size != (IntPtr.Size == 8 ? 40 : 28)) throw new InvalidOperationException("Unexpected Windows INPUT size.");
        uint actual = SendInput((uint)input.Length, input, size);
        if (actual != input.Length)
            throw new InvalidOperationException("SendInput delivered " + actual + "/" + input.Length +
                " mouse changes; Win32 error " + Marshal.GetLastWin32Error() + ".");
    }
    private static int MenuPosition(IntPtr menu, uint itemId, string caption) {
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
    private static IntPtr DialogFocus(IntPtr dialog) {
        uint thread = GetWindowThreadProcessId(dialog, out uint ignoredPid);
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        return thread != 0 && GetGUIThreadInfo(thread, ref gui) ? gui.Focus : IntPtr.Zero;
    }
    private static bool FilenameControlPath(IntPtr child, IntPtr dialog) {
        for (IntPtr current = child; current != IntPtr.Zero && current != dialog; current = GetParent(current)) {
            int id = GetDlgCtrlID(current);
            string kind = Kind(current);
            if ((id == 0x480 && String.Equals(kind, "Edit", StringComparison.OrdinalIgnoreCase)) ||
                (id == 0x47c && kind.StartsWith("ComboBox", StringComparison.OrdinalIgnoreCase))) return true;
        }
        return false;
    }
    private static bool KnownDefaultName(IntPtr edit, string name) {
        if (String.IsNullOrEmpty(name)) return false;
        try {
            string value = Read(edit).Trim().Trim('"');
            return String.Equals(value, name, StringComparison.OrdinalIgnoreCase) ||
                String.Equals(value, System.IO.Path.GetFileNameWithoutExtension(name), StringComparison.OrdinalIgnoreCase);
        } catch (InvalidOperationException) { return false; }
    }
    public static IntPtr FilenameField(IntPtr dialog, uint pid, string defaultName) {
        if (GetWindowThreadProcessId(dialog, out uint dialogPid) == 0 || dialogPid != pid ||
            !IsWindowVisible(dialog) || Kind(dialog) != "#32770") return IntPtr.Zero;
        var canonical = new List<IntPtr>();
        var defaults = new List<IntPtr>();
        EnumChildWindows(dialog, (hwnd, ignored) => {
            if (!IsWindowVisible(hwnd) || GetWindowThreadProcessId(hwnd, out uint found) == 0 || found != pid)
                return true;
            string kind = Kind(hwnd);
            if (!String.Equals(kind, "Edit", StringComparison.OrdinalIgnoreCase) &&
                !kind.StartsWith("RichEdit", StringComparison.OrdinalIgnoreCase)) return true;
            if (FilenameControlPath(hwnd, dialog)) canonical.Add(hwnd);
            if (KnownDefaultName(hwnd, defaultName)) defaults.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        if (canonical.Count == 1) return canonical[0];
        if (defaults.Count == 1) return defaults[0];
        IntPtr focus = DialogFocus(dialog);
        if (canonical.Count > 1 && canonical.Contains(focus) &&
            (String.IsNullOrEmpty(defaultName) || defaults.Contains(focus))) return focus;
        if (defaults.Count > 1 && defaults.Contains(focus)) return focus;
        return IntPtr.Zero;
    }
    public static string DescribeFilenameControls(IntPtr dialog, uint pid) {
        if (GetWindowThreadProcessId(dialog, out uint dialogPid) == 0 || dialogPid != pid)
            return "Owned file dialog unavailable.";
        var text = new StringBuilder("Dialog control IDs and classes:");
        IntPtr focus = DialogFocus(dialog);
        int count = 0;
        EnumChildWindows(dialog, (hwnd, ignored) => {
            if (GetWindowThreadProcessId(hwnd, out uint found) == 0 || found != pid) return true;
            text.Append(" [").Append(Kind(hwnd)).Append(" id=").Append(GetDlgCtrlID(hwnd))
                .Append(" parentID=").Append(GetDlgCtrlID(GetParent(hwnd)))
                .Append(IsWindowVisible(hwnd) ? " visible" : " hidden")
                .Append(hwnd == focus ? " focus]" : "]");
            return ++count < 64;
        }, IntPtr.Zero);
        return text.ToString();
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
    $original = [PaperroomHighlightGui]::Cursor()
    $point = $original
    try {
        $button = Control $window $OwnerControl
        $point = [PaperroomHighlightGui]::ButtonPoint($window, $process.Id, $button)
        [PaperroomHighlightGui]::MouseAction($window, $process.Id, $button, $point, 0)
        $menu = Wait-For "menu containing $Caption" {
            [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32768', $null, 0)
        }
        for ($attempt = 0; $attempt -lt 18; $attempt++) {
            $selection = [PaperroomHighlightGui]::PlanMenu($window, $process.Id, $menu, $Item, $Caption)
            $point = $selection.Point
            [PaperroomHighlightGui]::MouseAction($window, $process.Id, $menu, $point, $selection.Scroll)
            if ($selection.Scroll -eq 0) {
                Write-Host "Verified and clicked native menu command $Item at ($($point.X),$($point.Y)): $Caption."
                $null = Wait-For "native menu closed after $Caption" { ![PaperroomHighlightGui]::IsWindowVisible($menu) }
                return
            }
            Start-Sleep -Milliseconds 180 # A clipped menu may scroll while the pointer remains over the arrow.
        }
        throw "Could not expose native menu command $Item in the bounded scroll."
    } finally {
        [PaperroomHighlightGui]::RestoreCursor($original, $point)
    }
}
function Native-File([uint32]$Item, [string]$Caption, [string]$Path, [string]$Status) {
    Native-Menu 109 $Item $Caption
    $dialog = Wait-For "common file dialog for $Caption" { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $null, 1) }
    $defaultName = switch ($Item) {
        3 { 'note.md' }
        4 { 'paperroom-backup.sqlite3' }
        30 { 'paperroom-html.zip' }
        default { '' }
    }
    try {
        $edit = Wait-For "unique native file name field for $Caption" {
            [PaperroomHighlightGui]::FilenameField($dialog, $process.Id, $defaultName)
        }
    } catch {
        Write-Host ([PaperroomHighlightGui]::DescribeFilenameControls($dialog, $process.Id))
        throw
    }
    Write-Host "Verified the common file dialog for command $Item ($Caption). Expected completion: $Status."
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
function Read-PreviewMark([bool]$Expected, [string]$Phase) {
    $preview = Control $window 121
    $null = Wait-For 'native RichEdit preview visible' { [PaperroomHighlightGui]::IsWindowVisible($preview) }
    if (![PaperroomHighlightGui]::Read($preview).Contains($selected)) { throw 'The RichEdit preview does not contain the selected text.' }
    $priorDpi = [IntPtr]::Zero
    $bitmap = $null
    try {
        $priorDpi = [PaperroomHighlightGui]::SetThreadDpiAwarenessContext([IntPtr](-4)) # Per-monitor-v2 coordinates.
        if ($priorDpi -eq [IntPtr]::Zero) { throw 'Could not enter per-monitor DPI awareness for capture.' }
        # Capture only the verified client rectangle once the compositor has
        # had a turn to repaint following the editor/reader transition.
        Start-Sleep -Milliseconds 100
        $rect = [PaperroomHighlightGui]::PreviewArea($window, $process.Id, $preview)
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        $bitmap = [Drawing.Bitmap]::new($width, $height)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, [Drawing.Size]::new($width, $height))
        } finally { $graphics.Dispose() }
        if (!$rect.Equals([PaperroomHighlightGui]::PreviewArea($window, $process.Id, $preview))) {
            throw 'The RichEdit area changed during the capture.'
        }
        $mark = [Drawing.Color]::FromArgb(255,232,131).ToArgb() -band 0xFFFFFF
        $colors = [Collections.Generic.HashSet[int]]::new()
        $hits = 0
        for ($y = 1; $y -lt $height; $y += 3) {
            for ($x = 1; $x -lt $width; $x += 3) {
                $value = $bitmap.GetPixel($x, $y).ToArgb() -band 0xFFFFFF
                if ($value -eq $mark) { $hits++ }
                [void]$colors.Add($value)
            }
        }
        if ($colors.Count -lt 3 -or ($Expected -and $hits -lt 5) -or (!$Expected -and $hits -ne 0)) {
            throw "Unexpected RichEdit capture (colors=$($colors.Count), highlight pixels=$hits, expected=$Expected)."
        }
        Write-Host "Foreground RichEdit samples for $Phase (${width}x${height}): $hits highlight pixels; expected visible=$Expected."
    } catch {
        $report = "$Phase - $($_.Exception.Message)"
        $previewFailures.Add($report)
        Write-Warning "RichEdit capture check failed: $report"
    } finally {
        if ($bitmap) { $bitmap.Dispose() }
        if ($priorDpi -ne [IntPtr]::Zero) { [void][PaperroomHighlightGui]::SetThreadDpiAwarenessContext($priorDpi) }
    }
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
$previewFailures = [Collections.Generic.List[string]]::new()
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
    Read-PreviewMark $true 'after mark'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Assert-Database $liveDb $true

    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note $title
    Read-PreviewMark $true 'reopened original'
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
    Read-PreviewMark $true 'restored backup'
    Native-File 30 '匯出工作區 HTML ZIP…' $html '已匯出 '
    $null = Open-Editor
    Native-Menu 113 26 '清除本篇螢光標記…'
    $cancel = Wait-For 'clear-highlight confirmation' { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '清除螢光標記', 7) }
    Command $cancel 7
    $null = Wait-For 'clear canceled' { ![PaperroomHighlightGui]::IsWindowVisible($cancel) }
    Command $window 110
    Read-PreviewMark $true 'after cancel clear'
    $null = Open-Editor
    Native-Menu 113 26 '清除本篇螢光標記…'
    $confirm = Wait-For 'clear-highlight confirmation' { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '清除螢光標記', 6) }
    Command $confirm 6
    $null = Wait-For 'clear acknowledgment' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已清除 1 個螢光標記。' }
    Command $window 110
    Read-PreviewMark $false 'after confirmed clear'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Assert-Database $restoreDb $false
    Assert-Database $backup $true $html
    Assert-Database $liveDb $true
    if ((Get-FileHash -LiteralPath $backup).Hash -cne $originalBackupHash) { throw 'Backup bytes changed.' }
    if ($previewFailures.Count -ne 0) {
        Write-Host 'Native reopen, backup, restored HTML, exact Markdown and clear persistence checks completed.'
        throw ("Foreground preview color checks failed: " + ($previewFailures -join ' | '))
    }
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
