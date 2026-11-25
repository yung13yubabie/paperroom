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
    private struct Input { public uint Kind; public InputData Data; }
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
    [DllImport("user32.dll")] private static extern bool IsChild(IntPtr parent, IntPtr candidate);
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
    public static Point ControlPoint(IntPtr owner, uint pid, IntPtr button, string expectedClass) {
        if (GetParent(button) != owner ||
            GetWindowThreadProcessId(button, out uint buttonPid) == 0 || buttonPid != pid ||
            !IsWindowVisible(button) || !String.Equals(Kind(button), expectedClass, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Expected an owned native " + expectedClass + " control.");
        var rect = VisibleRect(button);
        Point point = Center(rect);
        if (!Contains(rect, point) || WindowFromPoint(point) != button)
            throw new InvalidOperationException("The toolbar button is not exposed at its center.");
        return point;
    }
    public static Point ButtonPoint(IntPtr owner, uint pid, IntPtr button) {
        return ControlPoint(owner, pid, button, "Button");
    }
    public static IntPtr FolderEdit(IntPtr dialog, uint pid) {
        var found = new List<IntPtr>();
        EnumChildWindows(dialog, (hwnd, ignored) => {
            if (GetWindowThreadProcessId(hwnd, out uint actual) != 0 && actual == pid &&
                IsWindowVisible(hwnd) && String.Equals(Kind(hwnd), "Edit", StringComparison.OrdinalIgnoreCase))
                found.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        if (found.Count > 1) throw new InvalidOperationException("Expected a unique native folder path field.");
        return found.Count == 1 ? found[0] : IntPtr.Zero;
    }
    public static bool DialogContains(IntPtr dialog, uint pid, string text) {
        bool found = false;
        EnumChildWindows(dialog, (hwnd, ignored) => {
            if (GetWindowThreadProcessId(hwnd, out uint actual) != 0 && actual == pid &&
                IsWindowVisible(hwnd) && Caption(hwnd).Contains(text)) found = true;
            return !found;
        }, IntPtr.Zero);
        return found;
    }
    public static MenuSelection PlanMenu(IntPtr owner, uint pid, IntPtr popup, uint itemId, string caption, bool prefix) {
        if (popup == IntPtr.Zero || OwnedWindow(owner, pid, "#32768", null, 0) != popup)
            throw new InvalidOperationException("The verified native menu is no longer open.");
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        uint thread = GetWindowThreadProcessId(owner, out uint ignoredPid);
        if (!GetGUIThreadInfo(thread, ref gui) || (gui.Flags & 0x14) == 0 || gui.MenuOwner != owner)
            throw new InvalidOperationException("The isolated app is not the current native menu owner.");
        IntPtr menu = Send(popup, 0x01E1, IntPtr.Zero, IntPtr.Zero); // MN_GETHMENU for inspected #32768.
        int position = MenuPosition(menu, itemId, caption, prefix);
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
            ? new[] { Mouse(0x0002), Mouse(0x0004) }
            : new[] { Mouse(0x0800, unchecked((uint)(scroll * 120))) };
        SendPhysical(input);
    }
    private static Input Mouse(uint flags, uint delta = 0) {
        return new Input { Data = new InputData { Mouse = new MouseInput { Flags = flags, Data = delta } } };
    }
    private static Input Key(ushort key, ushort scan, uint flags) {
        return new Input { Kind = 1, Data = new InputData { Key = new KeyInput {
            VirtualKey = key, Scan = scan, Flags = flags
        } } };
    }
    public static void TapKey(IntPtr owner, uint pid, ushort key) {
        if (!ForegroundIsOwned(owner, pid)) throw new InvalidOperationException("Expected the owned window before typing.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        SendPhysical(new[] { Key(key, 0, 0), Key(key, 0, 2) });
    }
    public static void TypeEditorText(IntPtr owner, uint pid, IntPtr edit, string prefix) {
        if (!ForegroundIsOwned(owner, pid) || DialogFocus(owner) != edit ||
            GetWindowThreadProcessId(edit, out uint editPid) == 0 || editPid != pid ||
            prefix.Length == 0 || prefix.Length > 256 || prefix.Contains("\r"))
            throw new InvalidOperationException("Expected the owned editor and bounded LF-only input.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        var keys = new List<Input>();
        foreach (char unit in prefix) {
            if (unit == '\n') {
                keys.Add(Key(0x0D, 0, 0));
                keys.Add(Key(0x0D, 0, 2));
            } else {
                keys.Add(Key(0, unit, 4));
                keys.Add(Key(0, unit, 4 | 2));
            }
        }
        SendPhysical(keys.ToArray());
    }
    private static void SendPhysical(Input[] input) {
        int size = Marshal.SizeOf<Input>();
        if (size != (IntPtr.Size == 8 ? 40 : 28)) throw new InvalidOperationException("Unexpected Windows INPUT size.");
        uint actual = SendInput((uint)input.Length, input, size);
        if (actual != input.Length)
            throw new InvalidOperationException("SendInput delivered " + actual + "/" + input.Length +
                " physical changes; Win32 error " + Marshal.GetLastWin32Error() + ".");
    }
    private static int MenuPosition(IntPtr menu, uint itemId, string caption, bool prefix) {
        int count = GetMenuItemCount(menu);
        if (count < 1 || count > 64) throw new InvalidOperationException("Unexpected native menu.");
        for (int index = 0; index < count; index++) {
            if (GetMenuItemID(menu, index) != itemId) continue;
            var title = new StringBuilder(256);
            GetMenuStringW(menu, (uint)index, title, title.Capacity, 0x400);
            string actual = title.ToString();
            if (prefix ? !actual.StartsWith(caption, StringComparison.Ordinal) : actual != caption)
                throw new InvalidOperationException("Unexpected menu label: " + title);
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
    public static bool FilenameHasFocus(IntPtr owner, uint pid, IntPtr dialog, IntPtr edit) {
        return ForegroundIsOwned(owner, pid) && GetForegroundWindow() == dialog &&
            IsChild(dialog, edit) && IsWindowVisible(edit) &&
            GetWindowThreadProcessId(edit, out uint editPid) != 0 && editPid == pid &&
            DialogFocus(dialog) == edit;
    }
    public static Point FilenamePoint(IntPtr dialog, uint pid, IntPtr edit) {
        if (!IsChild(dialog, edit) || !IsWindowVisible(edit) ||
            GetWindowThreadProcessId(edit, out uint editPid) == 0 || editPid != pid)
            throw new InvalidOperationException("Expected an owned, visible file name edit.");
        Rect rect = VisibleRect(edit);
        Point point = Center(rect);
        if (!Contains(rect, point) || WindowFromPoint(point) != edit)
            throw new InvalidOperationException("The file name edit is not exposed at its center.");
        return point;
    }
    public static void TypeFilename(IntPtr owner, uint pid, IntPtr dialog, IntPtr edit, string value) {
        if (value.Length == 0 || value.Length > 512)
            throw new InvalidOperationException("Unexpected test filename length.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        var keys = new List<Input> { Key(0x11, 0, 0), Key(0x41, 0, 0), Key(0x41, 0, 2), Key(0x11, 0, 2) };
        foreach (char unit in value) {
            if (Char.IsControl(unit)) throw new InvalidOperationException("Unexpected control character in test filename.");
            keys.Add(Key(0, unit, 4)); // KEYEVENTF_UNICODE.
            keys.Add(Key(0, unit, 4 | 2)); // KEYEVENTF_KEYUP.
        }
        if (!FilenameHasFocus(owner, pid, dialog, edit))
            throw new InvalidOperationException("The exact native file name field lost focus.");
        SendPhysical(keys.ToArray());
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
function Native-Menu([int]$OwnerControl, [uint32]$Item, [string]$Caption, [uint16]$Shortcut = 0,
                     [switch]$MenuOpen, [switch]$CaptionPrefix) {
    if (![PaperroomHighlightGui]::ForegroundIsOwned($window, $process.Id)) {
        [void][PaperroomHighlightGui]::SetForegroundWindow($window)
        $null = Wait-For 'isolated app foreground' { [PaperroomHighlightGui]::ForegroundIsOwned($window, $process.Id) }
    }
    $original = [PaperroomHighlightGui]::Cursor()
    $point = $original
    try {
        if ($Shortcut) {
            [PaperroomHighlightGui]::TapKey($window, $process.Id, $Shortcut)
        } elseif (!$MenuOpen) {
            $button = Control $window $OwnerControl
            $point = [PaperroomHighlightGui]::ButtonPoint($window, $process.Id, $button)
            [PaperroomHighlightGui]::MouseAction($window, $process.Id, $button, $point, 0)
        }
        $menu = Wait-For "menu containing $Caption" {
            [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32768', $null, 0)
        }
        for ($attempt = 0; $attempt -lt 18; $attempt++) {
            $selection = [PaperroomHighlightGui]::PlanMenu($window, $process.Id, $menu, $Item, $Caption, $CaptionPrefix.IsPresent)
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
function Replace-EditorText([IntPtr]$Field, [string]$Text) {
    $point = [PaperroomHighlightGui]::ControlPoint($window, $process.Id, $Field, 'Edit')
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $Field, $point, 0)
    [void][PaperroomHighlightGui]::Send($Field, 0x00B1, [IntPtr]::Zero, [IntPtr](-1))
    [PaperroomHighlightGui]::TapKey($window, $process.Id, 0x08)
    if ($Text.Length -gt 0) { [PaperroomHighlightGui]::TypeEditorText($window, $process.Id, $Field, $Text) }
    $expected = $Text.Replace("`n", "`r`n")
    $null = Wait-For 'physically entered replacement text' { [PaperroomHighlightGui]::Read($Field) -ceq $expected }
}
function Open-Revision([int]$Index) {
    Native-Menu 113 22 '檢視與還原版本…'
    Native-Menu 113 $Index '版本 ' -MenuOpen -CaptionPrefix
    Wait-For 'native history difference preview' { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '筆記版本差異', 1) }
}
function Finish-Revision([IntPtr]$Dialog, [int]$Button) {
    $control = Control $Dialog $Button
    $point = [PaperroomHighlightGui]::ButtonPoint($Dialog, $process.Id, $control)
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $control, $point, 0)
    $null = Wait-For 'native revision dialog closed' { ![PaperroomHighlightGui]::IsWindowVisible($Dialog) }
}
function Read-HistoryStamp([string]$Database, [string]$Title) {
    $result = @'
from pathlib import Path
from contextlib import closing
import sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri() + '?mode=ro', uri=True, timeout=2)) as db:
    row = db.execute('SELECT id, version FROM notes WHERE title=? AND trash_root IS NULL', (sys.argv[2],)).fetchone()
    assert row is not None
    print(f'{row[1]}:{db.execute("SELECT count(*) FROM note_revisions WHERE note_id=?", (row[0],)).fetchone()[0]}')
'@ | python - $Database $Title
    if ($LASTEXITCODE -ne 0 -or $result -notmatch '^\d+:\d+$') { throw 'Could not read the draft history stamp.' }
    return $result
}
function Native-File([uint32]$Item, [string]$Caption, [string]$Path, [string]$Status,
                     [switch]$AlreadyOpened, [string]$ExpectedError = '') {
    if (!$AlreadyOpened) { Native-Menu 109 $Item $Caption }
    $dialog = Wait-For "common file dialog for $Caption" { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $null, 1) }
    $defaultName = switch ($Item) {
        3 { 'note.md' }
        4 { 'paperroom-backup.sqlite3' }
        30 { 'paperroom-html.zip' }
        34 { 'vault.zip' }
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
    $original = [PaperroomHighlightGui]::Cursor()
    $point = $original
    $saving = $Item -ne 1
    # Quoting an explicit save filename prevents a file filter from adding an extension.
    $typed = if ($saving) { '"' + $Path + '"' } else { $Path }
    try {
        $point = [PaperroomHighlightGui]::FilenamePoint($dialog, $process.Id, $edit)
        [PaperroomHighlightGui]::MouseAction($window, $process.Id, $edit, $point, 0)
        $null = Wait-For "native file name focus for $Caption" {
            [PaperroomHighlightGui]::FilenameHasFocus($window, $process.Id, $dialog, $edit)
        }
        [PaperroomHighlightGui]::TypeFilename($window, $process.Id, $dialog, $edit, $typed)
        $null = Wait-For "exact physical path entry for $Caption" {
            [PaperroomHighlightGui]::Read($edit) -ceq $typed
        }
        $submit = Control $dialog 1
        $point = [PaperroomHighlightGui]::ButtonPoint($dialog, $process.Id, $submit)
        [PaperroomHighlightGui]::MouseAction($window, $process.Id, $submit, $point, 0)
        if ($ExpectedError) {
            $errorDialog = Wait-For 'native export refusal' {
                [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '紙間', 1)
            }
            if (![PaperroomHighlightGui]::DialogContains($errorDialog, $process.Id, $ExpectedError) -or
                (Test-Path -LiteralPath $Path)) { throw 'The export refusal or its file state differs from the expected result.' }
            Finish-Revision $errorDialog 1
            return
        }
        $null = Wait-For "$Caption completed" {
            ![PaperroomHighlightGui]::IsWindowVisible($dialog) -and
                [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith($Status, [StringComparison]::Ordinal)
        }
        if ($saving -and !(Test-Path -LiteralPath $Path -PathType Leaf)) {
            throw 'The application reported success without the exact requested output file.'
        }
    } catch {
        Write-Host "Synthetic files after command $Item (relative to the isolated root):"
        Get-ChildItem -LiteralPath $root -File -Recurse -Force | Select-Object -First 36 | ForEach-Object {
            Write-Host ('  ' + [IO.Path]::GetRelativePath($root, $_.FullName))
        }
        if ([PaperroomHighlightGui]::IsWindowVisible($dialog)) {
            Write-Host ([PaperroomHighlightGui]::DescribeFilenameControls($dialog, $process.Id))
        }
        throw
    } finally {
        [PaperroomHighlightGui]::RestoreCursor($original, $point)
    }
}
function Native-VaultFolder([string]$Folder, [string]$ReportLine) {
    Native-Menu 109 6 '匯入 Obsidian Markdown 資料夾…'
    $picker = Wait-For 'native Vault folder picker' {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $null, 1)
    }
    $edit = Wait-For 'Vault folder path field' { [PaperroomHighlightGui]::FolderEdit($picker, $process.Id) }
    $point = [PaperroomHighlightGui]::FilenamePoint($picker, $process.Id, $edit)
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $edit, $point, 0)
    $null = Wait-For 'folder path focus' { [PaperroomHighlightGui]::FilenameHasFocus($window, $process.Id, $picker, $edit) }
    [PaperroomHighlightGui]::TypeFilename($window, $process.Id, $picker, $edit, $Folder)
    $null = Wait-For 'exact entered folder path' { [PaperroomHighlightGui]::Read($edit) -ceq $Folder }
    $ok = Control $picker 1
    $point = [PaperroomHighlightGui]::ButtonPoint($picker, $process.Id, $ok)
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $ok, $point, 0)
    $report = Wait-For 'Vault import report' {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '匯入報告', 1)
    }
    if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains($ReportLine)) {
        throw 'The native folder import report has an unexpected attachment count.'
    }
    Finish-Revision $report 1
    Write-Host "Native folder picker and import report verified: $ReportLine"
}
function Vault-ChoiceIndex([string]$Database, [string]$Title) {
    $number = @'
from pathlib import Path
from contextlib import closing
import sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    roots=db.execute("SELECT n.title FROM notes n WHERE n.trash_root IS NULL AND EXISTS "
                     "(SELECT 1 FROM vault_paths p WHERE p.import_root=n.id) ORDER BY n.title,n.id").fetchall()
    print(roots.index((sys.argv[2],))+1)
'@ | python - $Database $Title
    if ($LASTEXITCODE -ne 0 -or $number -notmatch '^[1-9]\d*$') { throw 'Could not identify the exact Vault menu choice.' }
    return [int]$number
}
function Native-VaultExport([string]$Title, [string]$Path, [string]$ExpectedError = '') {
    $choice = Vault-ChoiceIndex $liveDb $Title
    Native-Menu 109 34 '匯出 Vault ZIP（含來源附件）…'
    Native-Menu 109 $choice "$Title  (#" -MenuOpen -CaptionPrefix
    Native-File 34 '匯出 Vault ZIP（含來源附件）…' $Path '已匯出 2 篇 Markdown 與 2 個來源附件。' -AlreadyOpened -ExpectedError $ExpectedError
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
function Assert-OutlineSelection([uint32]$Item, [string]$MenuLabel, [string]$ExpectedLine, [string]$After) {
    Native-Menu 109 $Item $MenuLabel 0x77 # F8
    $field = Control $window 104
    $null = Wait-For 'outline opened the editor' { [PaperroomHighlightGui]::IsWindowVisible($field) }
    $source = [PaperroomHighlightGui]::Read($field)
    $expectedStart = $source.IndexOf($ExpectedLine + "`r`n" + $After, [StringComparison]::Ordinal)
    if ($expectedStart -lt 0) { throw "The expected source heading and following paragraph were absent." }
    $null = Wait-For "outline selected the UTF-16 range for $MenuLabel" {
        $selection = [PaperroomHighlightGui]::Send($field, 0x00B0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        ($selection -band 0xffff) -eq $expectedStart -and ($selection -shr 16) -eq $expectedStart + $ExpectedLine.Length
    }
    Write-Host "F8 selected expected source heading: $MenuLabel."
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
        for section, text_after in ((2, "\u7b2c\u4e00\u8655"), (3, "\u7b2c\u4e8c\u8655")):
            assert f'href="#section-{section}">\u540c\u540d \U0001f3ac</a>'.encode() in html
            assert f'<h2 id="section-{section}">\u540c\u540d \U0001f3ac</h2><p>{text_after}</p>'.encode() in html
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
    $manuscript = "---`nqa: highlight`n---`n`n前方 📘選取📝 後方`n`n==保留==`n`n# 開場`n`n## 同名 🎬`n第一處`n`n## 同名 🎬`n第二處`n"
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
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note $title
    Assert-OutlineSelection 3 'H2　同名 🎬　（第 14 行）' '## 同名 🎬' '第二處'
    $field = Control $window 104
    $existing = [PaperroomHighlightGui]::Read($field)
    $insertAt = $existing.IndexOf('# 開場', [StringComparison]::Ordinal)
    if ($insertAt -lt 0) { throw 'The prefix insertion point was missing.' }
    $prefix = "# 新段落 📽`n`n"
    $updated = $existing.Insert($insertAt, $prefix.Replace("`n", "`r`n"))
    [void][PaperroomHighlightGui]::Send($field, 0x00B1, [IntPtr]$insertAt, [IntPtr]$insertAt)
    [PaperroomHighlightGui]::TypeEditorText($window, $process.Id, $field, $prefix)
    $null = Wait-For 'real typing changed the source editor' { [PaperroomHighlightGui]::Read($field) -ceq $updated }
    Assert-OutlineSelection 4 'H2　同名 🎬　（第 16 行）' '## 同名 🎬' '第二處'
    Command $window 110
    $null = Wait-For 'saved outline manuscript in reading mode' { [PaperroomHighlightGui]::IsWindowVisible((Control $window 121)) }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note $title
    Assert-OutlineSelection 4 'H2　同名 🎬　（第 16 行）' '## 同名 🎬' '第二處'
    if ([PaperroomHighlightGui]::Read((Control $window 104)) -cne $updated) {
        throw 'Saved manuscript differs from the draft after reopening.'
    }
    Write-Host 'PASS: F8 outline selected the second of two same-name Unicode headings in the saved source, live draft, and resumed note. Offline HTML links separately identified both sections.'
    $historyTitle = Control $window 103
    $tooLong = '稿' * 181
    $stamp = Read-HistoryStamp $liveDb $title
    Replace-EditorText $historyTitle $tooLong
    $null = Wait-For 'invalid unsaved draft stayed in the editor' {
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('儲存失敗')
    }
    $preview = Open-Revision 1
    $report = [PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$report.Contains('目前未存草稿') -or !$report.Contains('先儲存草稿') -or
        !$report.Contains('− 目前標題') -or
        [PaperroomHighlightGui]::Caption((Control $preview 1)) -cne '關閉') {
        throw 'Expected a read-only comparison with the unsaved title.'
    }
    Finish-Revision $preview 1
    if ([PaperroomHighlightGui]::Read($historyTitle) -cne $tooLong -or (Read-HistoryStamp $liveDb $title) -cne $stamp) {
        throw 'Previewing the unsaved draft changed either the draft or the saved history.'
    }
    Replace-EditorText $historyTitle $title
    $null = Wait-For 'valid title saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦' }
    $field = Control $window 104
    foreach ($value in '', '保留這份正文📝', $updated.Replace("`r`n", "`n")) {
        Replace-EditorText $field $value
        $null = Wait-For 'history fixture saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦' }
    }
    $stamp = Read-HistoryStamp $liveDb $title
    $preview = Open-Revision 3
    $report = [PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$report.Contains('本版與目前保存內容完全相同') -or
        [PaperroomHighlightGui]::Caption((Control $preview 1)) -cne '關閉') {
        throw 'An identical historical version must be a read-only preview.'
    }
    Finish-Revision $preview 1
    if ((Read-HistoryStamp $liveDb $title) -cne $stamp) { throw 'Viewing an equal version changed history.' }
    $preview = Open-Revision 2
    $report = [PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$report.Contains('選取的舊版沒有正文') -or !$report.Contains('−     9  # 新段落 📽') -or
        [PaperroomHighlightGui]::Caption((Control $preview 1)) -cne '還原此版本') {
        throw 'Expected a restorable blank version and the exact removed line.'
    }
    Finish-Revision $preview 2
    if ((Read-HistoryStamp $liveDb $title) -cne $stamp) { throw 'Canceling restore changed history.' }
    $preview = Open-Revision 2
    Finish-Revision $preview 1
    $null = Wait-For 'blank historical version restored' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已還原選取的筆記版本。' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note $title
    $field = Open-Editor
    if ([PaperroomHighlightGui]::Read($field) -cne '') { throw 'Restored empty body did not survive reopening.' }
    $preview = Open-Revision 1
    $report = [PaperroomHighlightGui]::Read((Control $preview 101))
    if (!$report.Contains('＋     9  # 新段落 📽')) { throw 'The replaced version is missing from history.' }
    Finish-Revision $preview 1
    if ([PaperroomHighlightGui]::Read($field) -cne $updated) { throw 'Recovering the replaced version did not return the full original body.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: native history preview protected the unsaved title, made repeated content a no-op, canceled and restored an empty version, reopened, and recovered the replaced version.'
    $vault = Join-Path $root 'VaultFixture'
    $vaultOutput = Join-Path $root 'vault-portable.zip'
    $readback = Join-Path $root 'VaultReadback'
    $badVault = Join-Path $root 'VaultAmbiguous'
    $badOutput = Join-Path $root 'vault-ambiguous-must-not-exist.zip'
    @'
from pathlib import Path
import struct,sys,zlib
v,b=map(Path,sys.argv[1:])
(v/'\u6bb5\u843d').mkdir(parents=True)
(v/'\u9644\u4ef6').mkdir()
(v/'\u6bb5\u843d'/'\u93e1\u982d.md').write_bytes(('\ufeff# \u93e1\u982d\r\n\r\n'
    '![\u5c01\u9762](../\u9644\u4ef6/\u5c01\u9762.png)\r\n[\u5f15\u7528](../\u9644\u4ef6/\u5f15\u7528.txt)\r\n').encode())
(v/'\u7d22\u5f15.md').write_bytes('[\u5f15\u7528](\u9644\u4ef6/\u5f15\u7528.txt)\n'.encode())
(v/'\u9644\u4ef6'/'\u5f15\u7528.txt').write_bytes('\u539f\u6587\r\n'.encode())
def chunk(kind,data):
    return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
(v/'\u9644\u4ef6'/'\u5c01\u9762.png').write_bytes(
    b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,2,0,0,0))+
    chunk(b'IDAT',zlib.compress(b'\x00\xaa\x50\x28'))+chunk(b'IEND',b''))
(b/'assets').mkdir(parents=True)
(b/'one.md').write_bytes(b'![[only.txt]]\n')
(b/'assets'/'only.txt').write_bytes(b'an archived local reference\n')
'@ | python - $vault $badVault
    if ($LASTEXITCODE -ne 0) { throw 'Synthetic Vault setup failed.' }

    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-VaultFolder $vault '附件綁定 3 筆，未解析 0 筆'
    Native-VaultFolder $vault '附件綁定 3 筆，未解析 0 筆'
    Native-VaultExport '匯入：VaultFixture' $vaultOutput
    @'
from pathlib import Path
from zipfile import ZipFile
import sys
original, archive, extracted=map(Path,sys.argv[1:])
expected={p.relative_to(original).as_posix():p.read_bytes() for p in original.rglob('*') if p.is_file()}
with ZipFile(archive) as z:
    assert z.testzip() is None and len(z.namelist())==len(expected)
    assert {name:z.read(name) for name in z.namelist()}==expected
    z.extractall(extracted)
print('Vault ZIP preserved every synthetic source path and byte, including PNG and BOM/CRLF.')
'@ | python - $vault $vaultOutput $readback
    if ($LASTEXITCODE -ne 0) { throw 'Vault ZIP verification or extraction failed.' }
    Native-VaultFolder $readback '附件綁定 3 筆，未解析 0 筆'
    Native-VaultFolder $badVault '附件綁定 1 筆，未解析 0 筆'
    Native-VaultExport '匯入：VaultAmbiguous' $badOutput '以檔名搜尋'
    if ((Test-Path -LiteralPath $badOutput) -or @(Get-ChildItem $root -Force -Filter '.paperroom-*.partial').Count) {
        throw 'The export refusal left an output or staging file.'
    }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Select-Note '匯入：VaultReadback'
    @'
from pathlib import Path
from contextlib import closing
import sqlite3,sys
dbfile,v=map(Path,sys.argv[1:])
notes={p.relative_to(v).as_posix():p.read_bytes() for p in v.rglob('*.md')}
attachment_paths={p.relative_to(v).as_posix():p.read_bytes() for p in v.rglob('*')
                  if p.is_file() and p.suffix in ('.png','.txt')}
expected={(note,path,data) for note in notes for path,data in attachment_paths.items()
          if path.endswith('.txt') or '/' in note}
with closing(sqlite3.connect(dbfile.as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute('PRAGMA integrity_check').fetchone()==('ok',)
    roots=db.execute("SELECT id FROM notes WHERE title IN ('\u532f\u5165\uff1aVaultFixture','\u532f\u5165\uff1aVaultReadback')"
                     " AND trash_root IS NULL ORDER BY id").fetchall()
    assert len(set(roots))==3
    for root, in roots:
        assert dict(db.execute("SELECT relative_path,CAST(body AS BLOB) FROM vault_paths p JOIN notes n "
                               "ON n.id=p.note_id WHERE p.import_root=?", (root,)))==notes
        actual=db.execute("SELECT p.relative_path,o.relative_path,a.content FROM vault_paths p "
                          "JOIN attachments a ON a.note_id=p.note_id JOIN attachment_origins o "
                          "ON o.attachment_id=a.id WHERE p.import_root=?", (root,)).fetchall()
        assert len(actual)==len(expected) and set(actual)==expected
print('Reopened SQLite preserves the two original imports and the independent readback, with 3 exact attachment bindings apiece.')
'@ | python - $liveDb $vault
    if ($LASTEXITCODE -ne 0) { throw 'The actual reopened workspace did not preserve the three Vault snapshots.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: native Vault folder import, duplicate snapshots, portable image and text attachment export, extraction, reimport and reopen, and safe refusal.'
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
