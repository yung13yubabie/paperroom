# Requires PowerShell 7, a Windows desktop and Python 3 with sqlite3.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('\A[0-9a-fA-F]{64}\z')][string]$ExeSha256,
    [switch]$ProvePortableLinkGap,
    [switch]$LegacyLinkOnly,
    [switch]$UnifiedSourcesOnly
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
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
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
    public static IntPtr ForegroundAlert(IntPtr owner, uint pid, string caption) {
        if (!ForegroundIsOwned(owner, pid)) return IntPtr.Zero;
        IntPtr alert = GetForegroundWindow();
        return Kind(alert) == "#32770" && Caption(alert) == caption ? alert : IntPtr.Zero;
    }
    public static Rect CanvasArea(IntPtr owner, uint pid) {
        if (!ForegroundIsOwned(owner, pid) || !GetClientRect(owner, out Rect client))
            throw new InvalidOperationException("Expected the foreground Canvas client.");
        int dpi = (int)GetDpiForWindow(owner);
        if (dpi < 96 || dpi > 288) throw new InvalidOperationException("Unexpected Canvas DPI.");
        int S(int x) => (x*dpi + 48)/96;
        var top = new Point { X = S(client.Right >= S(1030) ? 330 : 303), Y = S(100) };
        var end = new Point { X = client.Right-S(27)-1, Y = client.Bottom-S(49)-1 };
        if (!ClientToScreen(owner, ref top) || !ClientToScreen(owner, ref end) ||
            WindowFromPoint(top) != owner || WindowFromPoint(end) != owner)
            throw new InvalidOperationException("The synthetic Canvas is obscured.");
        return new Rect { Left=top.X, Top=top.Y, Right=end.X+1, Bottom=end.Y+1 };
    }
    public static Point CanvasCursor(IntPtr owner, uint pid, int x, int y) {
        var point = new Point { X=x, Y=y };
        if (!ForegroundIsOwned(owner, pid) || WindowFromPoint(point) != owner ||
            !SetCursorPos(x,y)) throw new InvalidOperationException("Expected an exposed Canvas pointer position.");
        return point;
    }
    public static void MiddleButton(IntPtr owner, uint pid, bool down) {
        if (!ForegroundIsOwned(owner, pid) || (down && WindowFromPoint(Cursor()) != owner))
            throw new InvalidOperationException("The middle button target changed.");
        SendPhysical(new[] { Mouse(down ? 0x20u : 0x40u) });
    }
    public static void LeftButton(IntPtr owner, uint pid, bool down) {
        if (!ForegroundIsOwned(owner, pid) || (down && WindowFromPoint(Cursor()) != owner))
            throw new InvalidOperationException("The left button target changed.");
        SendPhysical(new[] { Mouse(down ? 0x02u : 0x04u) });
    }
    public static bool CanvasCaptured(IntPtr owner, uint pid) {
        uint thread = GetWindowThreadProcessId(owner, out uint actual);
        var gui = new GuiThread { Size = (uint)Marshal.SizeOf<GuiThread>() };
        return thread != 0 && actual == pid && GetGUIThreadInfo(thread, ref gui) && gui.Capture == owner;
    }
    public static int[] InspectMenuSelection(IntPtr owner, uint pid, IntPtr popup, uint itemId, string caption) {
        if (popup == IntPtr.Zero || OwnedWindow(owner, pid, "#32768", null, 0) != popup)
            throw new InvalidOperationException("The verified native menu is no longer open.");
        IntPtr menu = Send(popup, 0x01E1, IntPtr.Zero, IntPtr.Zero);
        int target = MenuPosition(menu, itemId, caption, false);
        int count = GetMenuItemCount(menu);
        int highlighted = -1;
        for (int i = 0; i < count; ++i) {
            uint state = GetMenuState(menu, (uint)i, 0x400); // MF_BYPOSITION
            if (state == uint.MaxValue) throw new InvalidOperationException("The menu item state is unavailable.");
            if ((state & 0x80) == 0) continue; // MF_HILITE
            if (highlighted >= 0) throw new InvalidOperationException("More than one native menu item is highlighted.");
            highlighted = i;
        }
        return new[] {target, highlighted, count};
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
    public static void BoardCardClick(IntPtr owner, uint pid, int slot, bool remove) {
        BoardCardClick(owner,pid,slot,remove,false,false);
    }
    public static void BoardCardClick(IntPtr owner, uint pid, int slot, bool right, bool centered, bool select) {
        if (slot != 0 && slot != 1) throw new ArgumentOutOfRangeException(nameof(slot));
        var area = CanvasArea(owner,pid);
        int dpi = (int)GetDpiForWindow(owner);
        // The first two ordinary cards occupy the first two grid slots.
        int x = centered ? (area.Left + area.Right) / 2 : area.Left + ((46 + 260 * slot) * dpi + 48) / 96;
        int y = centered ? (area.Top + area.Bottom) / 2 : area.Top + (46 * dpi + 48) / 96;
        var point = CanvasCursor(owner, pid, x, y);
        if (WindowFromPoint(point) != owner || !WorkspaceInForeground(owner,pid))
            throw new InvalidOperationException("Expected the unoccluded, foreground native board.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        if (select) SendPhysical(new[] { Key(0x11, 0, 0), Mouse(2), Mouse(4), Key(0x11, 0, 2) });
        else if (right) SendPhysical(new[] { Mouse(8), Mouse(16) });
        else SendPhysical(new[] { Mouse(2), Mouse(4), Mouse(2), Mouse(4) });
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
    public static void CanvasUndoKey(IntPtr owner, uint pid) {
        if (!ForegroundIsOwned(owner, pid) || DialogFocus(owner) != owner)
            throw new InvalidOperationException("Expected the drawing surface to have focus before Ctrl+Z.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        SendPhysical(new[] { Key(0x11, 0, 0), Key(0x5A, 0, 0), Key(0x5A, 0, 2), Key(0x11, 0, 2) });
    }
    public static bool WorkspaceInForeground(IntPtr owner, uint pid) {
        return ForegroundIsOwned(owner, pid) && GetForegroundWindow() == owner;
    }
    public static void WorkspaceSearchKey(IntPtr owner, uint pid) {
        if (!WorkspaceInForeground(owner, pid))
            throw new InvalidOperationException("Expected the isolated workspace before Ctrl+Shift+F.");
        foreach (int modifier in new[] { 0x10, 0x11, 0x12, 0x5B, 0x5C })
            if (GetAsyncKeyState(modifier) < 0) throw new InvalidOperationException("An input modifier is held.");
        SendPhysical(new[] { Key(0x11, 0, 0), Key(0x10, 0, 0), Key(0x46, 0, 0),
                             Key(0x46, 0, 2), Key(0x10, 0, 2), Key(0x11, 0, 2) });
    }
    public static bool EditorHasFocus(IntPtr owner, uint pid, IntPtr edit) {
        return ForegroundIsOwned(owner, pid) && DialogFocus(owner) == edit &&
            GetWindowThreadProcessId(edit, out uint editPid) != 0 && editPid == pid;
    }
    public static void TypeEditorText(IntPtr owner, uint pid, IntPtr edit, string prefix) {
        if (!EditorHasFocus(owner, pid, edit) ||
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
    public static string[] CurrentMenuLabels(IntPtr owner, uint pid, IntPtr popup) {
        return CurrentMenuLabels(owner, pid, popup, true);
    }
    public static string[] CurrentMenuLabels(IntPtr owner, uint pid, IntPtr popup, bool requireSequentialIds) {
        if (popup == IntPtr.Zero || OwnedWindow(owner, pid, "#32768", null, 0) != popup)
            throw new InvalidOperationException("The verified native menu is no longer open.");
        IntPtr menu = Send(popup, 0x01E1, IntPtr.Zero, IntPtr.Zero);
        int count = GetMenuItemCount(menu);
        if (count < 1 || count > 64) throw new InvalidOperationException("Unexpected native menu.");
        var labels = new string[count];
        for (int i = 0; i < count; ++i) {
            var label = new StringBuilder(256);
            if (requireSequentialIds && GetMenuItemID(menu, i) != (uint)(i + 1))
                throw new InvalidOperationException("Expected a sequential Paperroom list.");
            if (GetMenuStringW(menu, (uint)i, label, label.Capacity, 0x400) == 0)
                throw new InvalidOperationException("Expected a labeled Paperroom command.");
            labels[i] = label.ToString();
        }
        return labels;
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
function Native-MenuFromKeyboard([int]$OwnerControl, [uint32]$Item, [string]$Caption) {
    Click-WorkspaceButton $OwnerControl
    $menu = Wait-For "keyboard navigation of $Caption" {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32768', $null, 0)
    }
    for ($attempt = 0; $attempt -lt 64; $attempt++) {
        $target, $highlighted, $count = [PaperroomHighlightGui]::InspectMenuSelection($window, $process.Id, $menu, $Item, $Caption)
        if ($highlighted -eq $target) {
            [PaperroomHighlightGui]::TapKey($window, $process.Id, 0x0D)
            $null = Wait-For "native menu closed after $Caption" { ![PaperroomHighlightGui]::IsWindowVisible($menu) }
            Write-Host "Verified and selected native menu command $Item by keyboard: $Caption."
            return
        }
        $forward = ($target - $highlighted + $count) % $count
        $left = if ($highlighted -lt 0) { $count - $target } else { $count - $forward }
        $key = if ($left -lt $forward) { 0x26 } else { 0x28 }
        [PaperroomHighlightGui]::TapKey($window, $process.Id, $key)
        $null = Wait-For 'menu highlight changed after a physical arrow key' {
            $state = [PaperroomHighlightGui]::InspectMenuSelection($window, $process.Id, $menu, $Item, $Caption)
            $state[1] -ne $highlighted
        }
    }
    throw "Did not reach the verified keyboard menu command $Caption."
}
function Replace-EditorText([IntPtr]$Field, [string]$Text) {
    $point = [PaperroomHighlightGui]::ControlPoint($window, $process.Id, $Field, 'Edit')
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $Field, $point, 0)
    $null = Wait-For 'clicked editor received focus before replacement' {
        [PaperroomHighlightGui]::EditorHasFocus($window, $process.Id, $Field)
    }
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
                     [switch]$AlreadyOpened, [string]$ExpectedError = '', [switch]$CanvasSave,
                     [string]$NextDialog = '', [string]$SaveName = '', [int]$NextDialogButton = 1) {
    if (!$AlreadyOpened) { Native-Menu 109 $Item $Caption }
    $dialog = Wait-For "common file dialog for $Caption" { [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $null, 1) }
    $defaultName = if ($SaveName) { $SaveName } elseif ($CanvasSave) { 'canvas.canvas' } else { switch ($Item) {
        3 { 'note.md' }
        4 { 'paperroom-backup.sqlite3' }
        30 { 'paperroom-html.zip' }
        34 { 'vault.zip' }
        36 { 'vault.zip' }
        default { '' }
    } }
    $original = [PaperroomHighlightGui]::Cursor()
    $point = $original
    $saving = $SaveName -or $CanvasSave -or $Item -notin 1, 2, 9, 11, 20, 31, 37, 38, 39, 40
    # Quoting an explicit save filename prevents a file filter from adding an extension.
    $typed = if ($saving) { '"' + $Path + '"' } else { $Path }
    try {
        # Explorer can replace its edit while populating the save dialog.
        $target = Wait-For "stable native file name edit for $Caption" {
            $edit = [PaperroomHighlightGui]::FilenameField($dialog, $process.Id, $defaultName)
            if ($edit -eq [IntPtr]::Zero) { return $null }
            try {
                $point = [PaperroomHighlightGui]::FilenamePoint($dialog,$process.Id,$edit)
                [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
            } catch {
                if ($_.Exception.Message -match 'owned, visible file name edit|edit is not exposed|point is no longer|pointer target changed') {
                    return $null
                }
                throw
            }
            if ([PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$dialog,$edit)) {
                [pscustomobject]@{ Edit=$edit; Point=$point }
            }
        }
        $edit = $target.Edit
        $point = $target.Point
        Write-Host "Verified the common file dialog for command $Item ($Caption). Expected completion: $Status."
        [PaperroomHighlightGui]::TypeFilename($window, $process.Id, $dialog, $edit, $typed)
        $null = Wait-For "exact physical path entry for $Caption" {
            [PaperroomHighlightGui]::Read($edit) -ceq $typed
        }
        $submit = Control $dialog 1
        $point = [PaperroomHighlightGui]::ButtonPoint($dialog, $process.Id, $submit)
        [PaperroomHighlightGui]::MouseAction($window, $process.Id, $submit, $point, 0)
        if ($ExpectedError) {
            $errorDialog = Wait-For 'native export refusal' {
                [PaperroomHighlightGui]::ForegroundAlert($window, $process.Id, '紙間')
            }
            if (![PaperroomHighlightGui]::DialogContains($errorDialog, $process.Id, $ExpectedError) -or
                ($saving -and (Test-Path -LiteralPath $Path))) { throw 'The refusal or its file state differs from the expected result.' }
            [PaperroomHighlightGui]::TapKey($window, $process.Id, 0x0D)
            $null = Wait-For 'native refusal dialog closed' { ![PaperroomHighlightGui]::IsWindowVisible($errorDialog) }
            return
        }
        if ($NextDialog) {
            return (Wait-For "native dialog after $Caption" {
                [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', $NextDialog, $NextDialogButton)
            })
        }
        $null = Wait-For "$Caption completed" {
            if ([PaperroomHighlightGui]::IsWindowVisible($dialog)) { return $false }
            try {
                [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith($Status, [StringComparison]::Ordinal)
            } catch {
                if (!$_.Exception.Message.Contains('Native text missing or truncated.')) { throw }
                return $false
            }
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
function Pick-NativeVaultFolder([string]$Folder) {
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
    $null = Wait-For 'native Vault folder picker closed' { ![PaperroomHighlightGui]::IsWindowVisible($picker) }
}
function Open-VaultFolderReview([string]$Folder) {
    Native-Menu 109 6 '匯入 Obsidian Markdown 資料夾…'
    Pick-NativeVaultFolder $Folder
    Wait-For 'read-only Vault folder import preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','預覽 Vault 資料夾匯入',1)
    }
}
function Native-VaultFolder([string]$Folder, [string]$ReportLine) {
    $review = Open-VaultFolderReview $Folder
    if (![PaperroomHighlightGui]::Read((Control $review 101)).Contains($ReportLine)) {
        throw 'The read-only Vault folder preview has an unexpected attachment count.'
    }
    Finish-Revision $review 1
    $report = Wait-For 'Vault import report' {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '匯入報告', 1)
    }
    if (![PaperroomHighlightGui]::Read((Control $report 101)).Contains($ReportLine)) {
        throw 'The native folder import report has an unexpected attachment count.'
    }
    Finish-Revision $report 1
    Write-Host "Native folder picker and import report verified: $ReportLine"
}
function Get-VaultReviewStamp([string]$Database) {
    $state = @'
from contextlib import closing
from pathlib import Path
import hashlib,sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    print(hashlib.sha256("\n".join(db.iterdump()).encode()).hexdigest())
'@ | python - $Database
    if ($LASTEXITCODE -ne 0 -or $state -notmatch '^[0-9a-f]{64}$') { throw 'Failed to read logical workspace state.' }
    return $state
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
function Read-CanvasInk([int]$Red = 237, [int]$Green = 33, [int]$Blue = 125) {
    try { $area = [PaperroomHighlightGui]::CanvasArea($window, $process.Id) }
    catch {
        if ($_.Exception.Message.Contains('The synthetic Canvas is obscured.')) { return $null }
        throw
    }
    $w = $area.Right - $area.Left
    $h = $area.Bottom - $area.Top
    $bitmap = [Drawing.Bitmap]::new($w,$h,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.CopyFromScreen($area.Left,$area.Top,0,0,[Drawing.Size]::new($w,$h)) }
        finally { $graphics.Dispose() }
        $pixels = $bitmap.LockBits([Drawing.Rectangle]::new(0,0,$w,$h),[Drawing.Imaging.ImageLockMode]::ReadOnly,
                                   [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $data = [byte[]]::new($pixels.Stride * $h)
            [Runtime.InteropServices.Marshal]::Copy($pixels.Scan0,$data,0,$data.Length)
            $left=$w; $top=$h; $right=-1; $bottom=-1
            for ($y=0; $y -lt $h; $y++) {
                $offset=$y*$pixels.Stride
                for ($x=0; $x -lt $w; $x++) {
                    $i=$offset+4*$x
                    if ($data[$i] -eq $Blue -and $data[$i+1] -eq $Green -and $data[$i+2] -eq $Red) {
                        if ($x -lt $left) { $left=$x }; if ($x -gt $right) { $right=$x }
                        if ($y -lt $top) { $top=$y }; if ($y -gt $bottom) { $bottom=$y }
                    }
                }
            }
            if ($right -lt $left) { return $null }
            [pscustomobject]@{ L=$left; T=$top; R=$right; B=$bottom; W=$right-$left+1; H=$bottom-$top+1;
                               ScreenX=$area.Left; ScreenY=$area.Top }
        } finally { $bitmap.UnlockBits($pixels) }
    } finally { $bitmap.Dispose() }
}
function Assert-NamedCanvasGroup([string]$Database, [string]$Title, [string]$Source, [string]$Grouped, [string]$Exported, [string]$Phase) {
    @'
from contextlib import closing
from pathlib import Path
import json, sqlite3, sys

database,title,source,grouped,exported,phase=sys.argv[1:]
before=json.loads(Path(source).read_bytes())
grouped_bytes=Path(grouped).read_bytes()
moved_bytes=Path(exported).read_bytes()
doc=json.loads(grouped_bytes)
assert len(doc["nodes"])==5
group=doc["nodes"].pop(0)
assert group=={"id":"paperroom-node-1","type":"group","label":"\u7814\u7a76\u5206\u93e1 \U0001f4cc",
               "x":20,"y":26,"width":329,"height":176}
assert doc==before, "Original group metadata, card order, edges or extensions were lost"
moved=json.loads(moved_bytes)
assert len(moved["nodes"])==5
delta=[moved["nodes"][0][key]-group[key] for key in ("x","y")]
assert all(12 < shift < 58 for shift in delta), delta
doc["nodes"].insert(0,group)
for node in doc["nodes"][:4]:
    for key,shift in zip(("x","y"),delta):
        node[key]+=shift
assert moved==doc, "Moved nodes differ beyond one translation or the partly overlapping card moved"
with closing(sqlite3.connect(Path(database).as_uri()+"?mode=ro",uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
    rows=db.execute("SELECT c.source,u.source FROM saved_canvases c LEFT JOIN canvas_edit_undo u "
                    "ON u.canvas_id=c.id WHERE c.title=?", (title,)).fetchall()
assert len(rows)==1
raw,undo=rows[0]
assert raw.encode("utf-8")== (grouped_bytes if phase=="undone" else moved_bytes)
assert undo== (grouped_bytes.decode("utf-8") if phase=="moved" else None)
print(f"Named group {phase}: standard fields, exact bytes, preserved nested group and edges, consistent group translation.")
'@ | python - $Database $Title $Source $Grouped $Exported $Phase
    if ($LASTEXITCODE -ne 0) { throw "Named Canvas group failed its independent document and workspace checks ($Phase)." }
}

function Locate-NoteOnCanvas([int]$CanvasIndex, [string]$CanvasName, [int]$CanvasId, [int]$Count,
                            [int]$NodeIndex, [string]$NodeId, [int[]]$Rgb, [switch]$FromLinks) {
    if ($FromLinks) {
        Native-Menu 120 60002 '查看此筆記的 Canvas 位置…'
    } else {
        Native-Menu 113 27 '查看此筆記的 Canvas 位置…'
    }
    Native-Menu 113 $CanvasIndex ("$CanvasName  (#${CanvasId}，$Count 處)") -MenuOpen
    Native-Menu 113 $NodeIndex "${NodeIndex}　同一張卡 〔$NodeId〕" -MenuOpen -CaptionPrefix
    $null = Wait-For 'exact Canvas and occurrence are active' {
        [PaperroomHighlightGui]::Read((Control $window 115)) -ceq $CanvasName -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq "已定位 Canvas 節點「$NodeId」。"
    }
    $ink = Wait-For 'painted the chosen occurrence using its unique native border' { Read-CanvasInk $Rgb[0] $Rgb[1] $Rgb[2] }
    $area = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
    if ([Math]::Abs($ink.L+$ink.R-($area.Right-$area.Left-1)) -gt 5 -or
        [Math]::Abs($ink.T+$ink.B-($area.Bottom-$area.Top-1)) -gt 5) {
        throw "The painted node '$NodeId' was not centered."
    }
    Write-Host "Verified painted centered occurrence by border bounds: $CanvasName / $NodeId."
}
function Open-LocationSource([string]$Title = '鏡頭', [string]$ExactBody = '場景原本的獨特引文') {
    Native-Menu 120 13 '開啟已選節點的來源筆記'
    $null = Wait-For 'the selected Canvas occurrence opens the exact note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $Title -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains($ExactBody)
    }
}
function Return-FromCanvasLocation {
    $button = Control $window 119
    $point = [PaperroomHighlightGui]::ButtonPoint($window,$process.Id,$button)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$button,$point,0)
    $null = Wait-For 'returned to the original note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
}
function Read-CanvasStamp([string]$Database, [string]$Source) {
    $stamp = @'
from pathlib import Path
from contextlib import closing
import json,sqlite3,sys
database,source=map(Path,sys.argv[1:])
with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True)) as db:
    rows=db.execute("SELECT c.source,v.x,v.y,v.scale FROM saved_canvases c LEFT JOIN canvas_cameras v "
                    "ON v.canvas_id=c.id WHERE c.title='CanvasViewFixture'").fetchall()
    assert len(rows)==1 and rows[0][0].encode('utf-8')==source.read_bytes(), 'Canvas source changed during navigation'
    print(json.dumps(rows[0][1:]))
'@ | python - $Database $Source
    if ($LASTEXITCODE -ne 0 -or !$stamp) { throw 'Could not verify the saved Canvas source and view.' }
    return [string]$stamp
}
function Pan-Canvas($Bounds, [int]$Dx, [int]$Dy, [switch]$Cancel) {
    $x = $Bounds.ScreenX + $Bounds.L + [int]($Bounds.W/2)
    $y = $Bounds.ScreenY + $Bounds.T + [int]($Bounds.H/2)
    $null = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,$x,$y)
    [PaperroomHighlightGui]::MiddleButton($window,$process.Id,$true)
    try {
        $null = Wait-For 'Canvas middle button capture' { [PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
        $null = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,$x+$Dx,$y+$Dy)
        if ($Cancel) {
            [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
            $null = Wait-For 'Canvas Escape released capture' { ![PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
        }
    } finally { [PaperroomHighlightGui]::MiddleButton($window,$process.Id,$false) }
    $null = Wait-For 'Canvas middle button released capture' { ![PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
}
function Drag-Canvas([int]$X, [int]$Y, [int]$EndX, [int]$EndY, [switch]$Cancel) {
    $null = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,$X,$Y)
    [PaperroomHighlightGui]::LeftButton($window,$process.Id,$true)
    try {
        $null = Wait-For 'Canvas left button capture' { [PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
        $null = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,$EndX,$EndY)
        if ($Cancel) {
            [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
            $null = Wait-For 'Canvas Escape released left capture' { ![PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
        }
    } finally { [PaperroomHighlightGui]::LeftButton($window,$process.Id,$false) }
    $null = Wait-For 'Canvas left button released capture' { ![PaperroomHighlightGui]::CanvasCaptured($window,$process.Id) }
}
function Assert-CanvasEdit([string]$Database, [string]$Title, [string]$Source, [string]$State, [string]$Export = '-') {
    @'
from pathlib import Path
from contextlib import closing
import json,sqlite3,sys
database,title,source,state,export=sys.argv[1:]
original=Path(source).read_bytes()
before=json.loads(original)
with closing(sqlite3.connect(Path(database).as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
    rows=db.execute("SELECT c.source,u.source FROM saved_canvases c LEFT JOIN canvas_edit_undo u "
                    "ON u.canvas_id=c.id WHERE c.title=?", (title,)).fetchall()
assert len(rows)==1
raw,undo=rows[0]
canvas=json.loads(raw)
if state in ("undone","untouched"):
    assert raw.encode("utf-8")==original and undo is None
elif state in ("moved","imported"):
    delta=[canvas["nodes"][0][axis]-before["nodes"][0][axis] for axis in ("x","y")]
    assert all(20 < change < 65 for change in delta), delta
    for node in before["nodes"][:2]:
        for axis,shift in zip(("x","y"),delta):
            node[axis]+=shift
    assert canvas==before, "Nonselected node, connection, extension, dimensions or group spacing changed"
    if state=="moved":
        assert undo.encode("utf-8")==original
    else:
        assert undo is None, "The imported Canvas inherited source workspace history"
elif state in ("connected","disconnected","edge-restored"):
    assert canvas["nodes"]==before["nodes"] and canvas["extra"]==before["extra"]
    extra={"id":"paperroom-edge-1","fromNode":"b","toNode":"a"}
    if state=="disconnected":
        assert canvas["edges"]==[extra] and json.loads(undo)["edges"]==before["edges"]+[extra]
    else:
        assert canvas["edges"]==before["edges"]+[extra]
        assert (undo is None if state=="edge-restored" else undo.encode("utf-8")==original)
else:
    raise AssertionError(state)
if export != "-":
    assert Path(export).read_bytes()==raw.encode("utf-8")
print(f"SQLite Canvas {state}: {len(canvas['nodes'])} nodes, {len(canvas['edges'])} edges.")
'@ | python - $Database $Title $Source $State $Export
    if ($LASTEXITCODE -ne 0) { throw "Canvas persistence or export check failed at $State." }
}
function Start-NotebookProcess([string]$Folder) {
    $binary = Join-Path $Folder 'Paperroom.exe'
    Assert-Hash $binary
    $info = [Diagnostics.ProcessStartInfo]::new($binary, '--portable')
    $info.UseShellExecute = $false
    $info.WorkingDirectory = $Folder
    foreach ($key in 'LOCALAPPDATA','TEMP','TMP') {
        $info.Environment[$key] = Join-Path $Folder $key
        $null = [IO.Directory]::CreateDirectory($info.Environment[$key])
    }
    [Diagnostics.Process]::Start($info)
}
function Start-Notebook([string]$Folder) {
    $started = Start-NotebookProcess $Folder
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
function Open-WorkspaceSearch([switch]$Menu) {
    if ($Menu) { Native-Menu 109 35 '全域搜尋筆記…' }
    else {
        $null = Wait-For 'main workspace owns foreground before Ctrl+Shift+F' {
            [PaperroomHighlightGui]::WorkspaceInForeground($window,$process.Id)
        }
        [PaperroomHighlightGui]::WorkspaceSearchKey($window,$process.Id)
    }
    Wait-For 'native workspace search dialog' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','全域搜尋筆記',1)
    }
}
function Type-SearchFilter([IntPtr]$Dialog, [int]$FieldId, [string]$ExactText) {
    $edit = Control $Dialog $FieldId
    $point = [PaperroomHighlightGui]::FilenamePoint($Dialog,$process.Id,$edit)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
    $null = Wait-For "workspace search field $FieldId focus" {
        [PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$Dialog,$edit)
    }
    [void][PaperroomHighlightGui]::Send($edit,0x00B1,[IntPtr]::Zero,[IntPtr](-1))
    [PaperroomHighlightGui]::TypeEditorText($window,$process.Id,$edit,$ExactText)
    $null = Wait-For "exact workspace search filter $FieldId" {
        [PaperroomHighlightGui]::Read($edit) -ceq $ExactText
    }
}
function Collection-Prompt([string]$Caption, [string]$Value = '') {
    $prompt = Wait-For "collection prompt $Caption" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$Caption,1)
    }
    if ($Value) { Type-SearchFilter $prompt 101 $Value }
    Finish-Revision $prompt 1
}
function Collection-MenuLabels {
    $menu = Wait-For 'actual collection row picker' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0)
    }
    [PaperroomHighlightGui]::CurrentMenuLabels($window,$process.Id,$menu)
}
function Choose-CollectionRow([string]$CaptionPrefix) {
    $options = @(Collection-MenuLabels)
    $matches = @(for ($i=0; $i -lt $options.Count; $i++) {
        if ($options[$i].StartsWith($CaptionPrefix,[StringComparison]::Ordinal)) { $i+1 }
    })
    if ($matches.Count -ne 1) { throw "The exact synthetic row '$CaptionPrefix' was not unique in $($options -join ' | ')." }
    Native-Menu 120 $matches[0] $CaptionPrefix -MenuOpen -CaptionPrefix
}
function Open-SortCollection([string]$Name = 'SortAudition') {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Native-Menu 109 1 $Name -MenuOpen
    $null = Wait-For 'collection layout opened' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
}
function Open-SortNamedView([int]$Index, [string]$Name) {
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 2 '開啟已儲存的檢視…' -MenuOpen
    Native-Menu 120 $Index "${Name}（表格）" -MenuOpen
}
function Assert-RankedRows([string[]]$Expected) {
    Native-Menu 120 3 '開啟資料列…'
    $labels = @(Collection-MenuLabels)
    if ($labels.Count -ne $Expected.Count) { throw "Wrong result count in the actual row picker: $($labels -join ' | ')." }
    for ($i=0; $i -lt $Expected.Count; $i++) {
        if ($labels[$i] -cnotmatch ('^待處理　'+[regex]::Escape($Expected[$i])+'  \(\d+\)$')) {
            throw "Wrong order in the actual row picker: $($labels -join ' | ')."
        }
    }
    Native-Menu 120 1 "待處理　$($Expected[0])  (" -MenuOpen -CaptionPrefix
    $null = Wait-For 'the first ranked row opens its original note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $Expected[0] -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟集合資料列'
    }
    Write-Host "Observed ordered collection rows through the actual menu: $($Expected -join ', ')."
}
function Set-SortColumn([string]$Column) {
    Native-Menu 120 2 '新增排序…' -MenuOpen
    if ($Column -eq 'formula') {
        Native-Menu 120 3 '公式結果…' -MenuOpen
        Native-Menu 120 1 '審閱順位' -MenuOpen
        Native-Menu 120 1 '數字結果' -MenuOpen
        Native-Menu 120 2 '降冪' -MenuOpen
    } else {
        Native-Menu 120 5 '彙總加總…' -MenuOpen
        Collection-Prompt '關聯或彙總欄位' '明細'
        Collection-Prompt '彙總加總欄位' '工時'
        Native-Menu 120 1 '升冪' -MenuOpen
    }
}

function Rule-Prompt([string]$Caption, [string]$Expected, [string]$Value, [switch]$Cancel) {
    $dialog = Wait-For "native collection prompt $Caption" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$Caption,1)
    }
    $edit = Control $dialog 101
    if ([PaperroomHighlightGui]::Read($edit) -cne $Expected) {
        throw "Unexpected initial text in collection prompt '$Caption', expected '$Expected'."
    }
    if ($Value -cne $Expected) {
        try {
            if ($Value.Length -gt 0) {
                Type-SearchFilter $dialog 101 $Value
            } else {
                $point = [PaperroomHighlightGui]::FilenamePoint($dialog,$process.Id,$edit)
                [PaperroomHighlightGui]::MouseAction($window,$process.Id,$edit,$point,0)
                $null = Wait-For "focused field $Caption" {
                    [PaperroomHighlightGui]::FilenameHasFocus($window,$process.Id,$dialog,$edit)
                }
                [void][PaperroomHighlightGui]::Send($edit,0x00B1,[IntPtr]::Zero,[IntPtr](-1))
                [PaperroomHighlightGui]::TapKey($window,$process.Id,0x2E)
                $null = Wait-For "cleared field $Caption" { [PaperroomHighlightGui]::Read($edit) -ceq '' }
            }
        } catch {
            throw "Native test field '$Caption' contained '$([PaperroomHighlightGui]::Read($edit))' after entering '$Value'. $($_.Exception.Message)"
        }
    }
    if ($Cancel) { Finish-Revision $dialog 2 } else { Finish-Revision $dialog 1 }
}
function Rule-Error([string]$Caption, [string]$Contains) {
    $dialog = Wait-For "native rule rejection $Caption" {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770',$Caption,1)
    }
    $reason = [PaperroomHighlightGui]::Read((Control $dialog 101))
    if (!$reason.Contains($Contains)) { throw "Unexpected rule rejection: $reason" }
    Finish-Revision $dialog 1
}
function Open-RuleBoard {
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 1 '開啟集合看板…' -MenuOpen
    Native-Menu 109 1 'Rule Fixture' -MenuOpen
    $null = Wait-For 'visible collection operations' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
}
function Set-Rule([string]$Key, [string]$Default) {
    Native-Menu 120 23 '集合欄位規則…'
    Choose-CollectionRow "${Key}　"
    Native-Menu 120 2 '必填' -MenuOpen
    Rule-Prompt '欄位預設值' '' $Default
    $null = Wait-For "rule saved for $Key" { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存集合欄位規則。' }
}
function Start-RuleRow([string]$Name) {
    Native-Menu 120 22 '新增資料列…'
    Rule-Prompt '新增資料列' '' $Name
    Rule-Prompt '加入集合' '待處理' '待處理'
}
function Check-RuleRows([string]$Database, [string]$Phase) {
    @'
import sqlite3, sys
from pathlib import Path
db = sqlite3.connect(Path(sys.argv[1]).resolve().as_uri() + "?mode=ro", uri=True)
phase = sys.argv[2]
def note(name):
    return db.execute("SELECT id, body FROM notes WHERE title=?", (name,)).fetchall()
def props(name):
    return dict(db.execute("SELECT key,value FROM note_properties JOIN notes ON notes.id=note_id WHERE title=?", (name,)))
members = dict(db.execute("SELECT title,status FROM collection_notes JOIN notes ON notes.id=note_id JOIN collections ON collections.id=collection_id WHERE name='Rule Fixture'"))
assert len(note('Legacy')) == 1 and note('Legacy')[0][1] == 'preserved body' and props('Legacy') == {}
assert members.pop('Legacy') == '\u5f85\u8655\u7406'
assert not note('Cancelled')
expected = {}
if phase not in ('before', 'reject-new'):
    expected['New row'] = {'A-check':'true', 'B-hours':('9' if phase in ('edited','restored','new-restored') else '7')}
if phase in ('edited','restored','new-restored'):
    expected['External'] = {'A-check':'true', 'B-hours':'0'}
else:
    assert props('External') == {'A-check':'true'}
if phase == 'new-restored':
    expected['Restored row'] = {'A-check':'false', 'B-hours':'0'}
for name in ('New row', 'Restored row'):
    assert len(note(name)) == int(name in expected), (phase,name)
assert members == {name:'\u5f85\u8655\u7406' for name in expected}, (phase, members)
for name, values in expected.items():
    assert props(name) == values, (phase,name,props(name),values)
assert note('External')[0][1] == 'keep this prior body'
assert db.execute('PRAGMA integrity_check').fetchall() == [('ok',)]
assert not db.execute('PRAGMA foreign_key_check').fetchall()
print("Read-only collection checks passed:", phase)
'@ | python - $Database $Phase
    if ($LASTEXITCODE -ne 0) { throw "Collection state differs at $Phase." }
}

function Search-Workspace([IntPtr]$Dialog, [string]$Status, [int]$Count, [string]$Context = '') {
    $go = Control $Dialog 1
    $point = [PaperroomHighlightGui]::ButtonPoint($Dialog,$process.Id,$go)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$go,$point,0)
    $null = Wait-For "workspace search: $Status" { [PaperroomHighlightGui]::Read((Control $Dialog 106)) -ceq $Status }
    $list = Control $Dialog 104
    $items = [PaperroomHighlightGui]::Send($list,0x018B,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64() # LB_GETCOUNT
    $preview = [PaperroomHighlightGui]::Read((Control $Dialog 105))
    if ($items -ne $Count -or ($Context -and !$preview.Contains($Context)) -or ($Count -eq 0 -and $preview)) {
        throw "Workspace search returned unexpected results or context. Count: $items; context: $preview"
    }
    Write-Host "Verified search UI: $Status"
}
function Assert-SearchSelection([int]$FieldId, [string]$Text, [string]$Status) {
    $field = Control $window $FieldId
    $null = Wait-For 'workspace search jumped to its source' {
        [PaperroomHighlightGui]::IsWindowVisible($field) -and
        [PaperroomHighlightGui]::EditorHasFocus($window,$process.Id,$field) -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq $Status
    }
    $body = [PaperroomHighlightGui]::Read($field)
    $start = $body.IndexOf($Text,[StringComparison]::Ordinal)
    if ($start -lt 0) { throw 'The search target did not contain the expected text.' }
    $null = Wait-For 'native EDIT selection matches exact UTF-16 search span' {
        $span = [PaperroomHighlightGui]::Send($field,0x00B0,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64()
        ($span -band 0xffff) -eq $start -and ($span -shr 16) -eq $start + $Text.Length
    }
    Write-Host "Verified exact native UTF-16 range $start..$($start+$Text.Length): $Text"
}
function Open-LinkedFixture([string]$Title, [string]$Folder) {
    $dialog = Open-WorkspaceSearch
    Type-SearchFilter $dialog 101 $Title
    Type-SearchFilter $dialog 103 $Folder
    Search-Workspace $dialog '找到 1 筆筆記。' 1
    Finish-Revision $dialog 111
    $null = Wait-For "linked fixture $Folder/$Title opened" {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $Title -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
}
function Find-LinkTarget([string]$Target) {
    Native-Menu 120 1 '插入筆記連結…'
    $dialog = Wait-For 'workspace link target prompt' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','插入筆記連結',1)
    }
    Type-SearchFilter $dialog 101 $Target
    Finish-Revision $dialog 1
}
function Click-WorkspaceButton([int]$Id) {
    $control = Control $window $Id
    $point = [PaperroomHighlightGui]::ButtonPoint($window,$process.Id,$control)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$control,$point,0)
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
    if ($UnifiedSourcesOnly) {
        . (Join-Path $PSScriptRoot 'windows-unified-sources.ps1')
        return
    }
    if ($LegacyLinkOnly) {
        . (Join-Path $PSScriptRoot 'windows-imported-legacy-link.ps1')
        return
    }
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
    # The Windows CI desktop can clip this row under the popup's scroll arrow.
    Native-MenuFromKeyboard 109 30 '匯出工作區 HTML ZIP…'
    Native-File 30 '匯出工作區 HTML ZIP…' $html '已匯出 ' -AlreadyOpened
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
    $null = Wait-For 'clear confirmation closed' { ![PaperroomHighlightGui]::IsWindowVisible($confirm) }
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
        try { [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('儲存失敗') }
        catch {
            if (!$_.Exception.Message.Contains('Native text missing or truncated.')) { throw }
            return $false
        }
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
    $canvasFile = Join-Path $root 'CanvasViewFixture.canvas'
    $canvasJson = '{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":100,"height":70,"text":"First","color":"#ed217d"},{"id":"b","type":"text","x":230,"y":110,"width":95,"height":65,"text":"Second","color":"#047bea"},{"id":"d","type":"text","x":65,"y":155,"width":58,"height":69,"text":"Unselected"}],"edges":[{"id":"c","fromNode":"a","toNode":"b","label":"kept","extension":{"flag":true}}],"extra":{"marker":true}}'
    [IO.File]::WriteAllText($canvasFile,$canvasJson,[Text.UTF8Encoding]::new($false))
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-File 11 '匯入 JSON Canvas…' $canvasFile '中鍵拖曳平移'
    $before = Wait-For 'painted exact Canvas node accent' { Read-CanvasInk }
    if ((Read-CanvasStamp $liveDb $canvasFile) -cne '[null, null, null]') { throw 'A new Canvas should start in fit view.' }
    Pan-Canvas $before 54 28
    $moved = Wait-For 'painted Canvas pan' {
        $ink = Read-CanvasInk
        if ($ink -and [Math]::Abs($ink.L-$before.L-54) -le 2 -and [Math]::Abs($ink.T-$before.T-28) -le 2) { $ink }
    }
    if ($moved.W -ne $before.W) { throw 'Panning changed the rendered node size.' }
    $pointer = [PaperroomHighlightGui]::CanvasCursor($window,$process.Id,$moved.ScreenX+$moved.L+[int]($moved.W/2),
                                                        $moved.ScreenY+$moved.T+[int]($moved.H/2))
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$window,$pointer,1)
    $zoom = Wait-For 'painted Canvas zoom' { $ink=Read-CanvasInk; if ($ink -and $ink.W -gt $moved.W) { $ink } }
    if ([Math]::Abs(($zoom.L+$zoom.R)-($moved.L+$moved.R)) -gt 4 -or
        [Math]::Abs(($zoom.T+$zoom.B)-($moved.T+$moved.B)) -gt 4) {
        throw 'Zoom lost its pointer anchor.'
    }
    $savedView = Read-CanvasStamp $liveDb $canvasFile
    if ($savedView -ceq '[null, null, null]') { throw 'Pan and zoom did not store a view.' }
    Write-Host "Actual Canvas accent checks: pan +$($moved.L-$before.L), +$($moved.T-$before.T) pixels; width $($moved.W) → $($zoom.W) at the mouse anchor."
    Pan-Canvas $zoom 29 23 -Cancel
    if ((Read-CanvasStamp $liveDb $canvasFile) -cne $savedView) { throw 'Canceled pan changed persisted state.' }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'CanvasViewFixture' -MenuOpen
    $null = Wait-For 'reopened Canvas controls became ready' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('中鍵拖曳平移')
    }
    $reopened = Wait-For 'reopened painted Canvas view' { Read-CanvasInk }
    if ($reopened.L -ne $zoom.L -or $reopened.T -ne $zoom.T -or $reopened.W -ne $zoom.W -or
        (Read-CanvasStamp $liveDb $canvasFile) -cne $savedView) { throw 'Reopening changed the saved view.' }
    Native-Menu 120 8 '將畫布調整至視窗'
    $null = Wait-For 'native fit command' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布調整至視窗。' }
    $null = Wait-For 'painted fit view' {
        $ink=Read-CanvasInk
        $ink -and $ink.L -eq $before.L -and $ink.T -eq $before.T -and $ink.W -eq $before.W
    }
    if ((Read-CanvasStamp $liveDb $canvasFile) -cne '[null, null, null]') { throw 'Fit command did not clear the prior view.' }
    Write-Host 'PASS: physical Canvas middle-button pan and wheel zoom, pointer anchoring, Escape cancel, native reopen and fit, with byte-identical Canvas JSON.'

    $a = Read-CanvasInk
    $b = Wait-For 'painted second Canvas node' { Read-CanvasInk 4 123 234 }
    $nearX = $a.ScreenX+$a.L-5
    $nearY = $a.ScreenY+$a.T-5
    Drag-Canvas $nearX $nearY ($b.ScreenX+$b.R+5) ($b.ScreenY+$b.B+5)
    $null = Wait-For 'exactly two enclosed nodes selected' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已選取 2 個畫布節點。' }
    $dragX = $a.ScreenX+$a.L+[int]($a.W/2)
    $dragY = $a.ScreenY+$a.T+[int]($a.H/2)
    Drag-Canvas $dragX $dragY ($dragX+54) ($dragY+48) -Cancel
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'untouched'
    Drag-Canvas $dragX $dragY ($dragX+54) ($dragY+48)
    $null = Wait-For 'group move stored' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存畫布修改。' }
    $null = Wait-For 'both painted nodes moved' {
        $ma = Read-CanvasInk
        $mb = Read-CanvasInk 4 123 234
        if ($ma -and $mb -and $ma.L-$a.L -eq $mb.L-$b.L -and $ma.T-$a.T -eq $mb.T-$b.T -and
            [Math]::Abs($ma.L-$a.L-54) -le 2 -and [Math]::Abs($ma.T-$a.T-48) -le 2) { $ma }
    }
    $portableCanvas = Join-Path $root 'CanvasReadback.canvas'
    Native-Menu 120 1 '匯出原始 JSON Canvas…'
    Native-File 1 '匯出原始 JSON Canvas…' $portableCanvas '已匯出原始 JSON Canvas。' -AlreadyOpened -CanvasSave
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'moved' $portableCanvas

    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 1 'CanvasViewFixture' -MenuOpen
    $null = Wait-For 'reopened Canvas edit view' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq 'Canvas 操作' -and
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('中鍵拖曳平移')
    }
    $null = Wait-For 'reopened Canvas paint' { Read-CanvasInk }
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'moved' $portableCanvas
    Drag-Canvas $nearX $nearY $nearX $nearY
    $null = Wait-For 'real click focused the Canvas' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已選取 0 個畫布節點。' }
    [PaperroomHighlightGui]::CanvasUndoKey($window,$process.Id)
    $null = Wait-For 'physical Ctrl+Z restored the move' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已復原上一次 Canvas 修改。' }
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'undone'

    Native-Menu 120 7 '編輯 Canvas…'
    Native-Menu 120 11 '新增連線…' -MenuOpen
    Native-Menu 120 2 '2　Second' -MenuOpen -CaptionPrefix
    Native-Menu 120 1 '1　First' -MenuOpen -CaptionPrefix
    $prompt = Wait-For 'native connection label prompt' {
        [PaperroomHighlightGui]::OwnedWindow($window, $process.Id, '#32770', '新增連線', 1)
    }
    $submit = Control $prompt 1
    $point = [PaperroomHighlightGui]::ButtonPoint($prompt, $process.Id, $submit)
    [PaperroomHighlightGui]::MouseAction($window, $process.Id, $submit, $point, 0)
    $null = Wait-For 'native connection created' {
        ![PaperroomHighlightGui]::IsWindowVisible($prompt) -and [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存畫布修改。'
    }
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'connected'
    Native-Menu 120 8 '將畫布調整至視窗'
    $null = Wait-For 'fit view before deleting a connection' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布調整至視窗。'
    }
    Native-Menu 120 7 '編輯 Canvas…'
    Native-Menu 120 13 '刪除連線…' -MenuOpen
    Native-Menu 120 1 '1　a → b' -MenuOpen -CaptionPrefix
    $confirmation = Wait-For 'synthetic connection deletion confirmation' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'刪除連線')
    }
    $yes = Control $confirmation 6
    $point = [PaperroomHighlightGui]::ButtonPoint($confirmation,$process.Id,$yes)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$yes,$point,0)
    $null = Wait-For 'native connection deleted' {
        ![PaperroomHighlightGui]::IsWindowVisible($confirmation) -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存畫布修改。'
    }
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'disconnected'
    Native-Menu 120 12 '復原上一次 Canvas 修改'
    $null = Wait-For 'native Undo restored the deleted connection' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已復原上一次 Canvas 修改。' }
    Assert-CanvasEdit $liveDb 'CanvasViewFixture' $canvasFile 'edge-restored'

    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $restored
    $window = $process.MainWindowHandle
    Native-File 11 '匯入 JSON Canvas…' $portableCanvas '中鍵拖曳平移'
    $null = Wait-For 'imported Canvas painted in second workspace' { Read-CanvasInk }
    Assert-CanvasEdit $restoreDb 'CanvasReadback' $canvasFile 'imported' $portableCanvas
    Native-Menu 120 12 '復原上一次 Canvas 修改'
    $null = Wait-For 'second workspace has no unrelated Undo record' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這份畫布目前沒有可復原的修改。' }
    if ([IO.File]::ReadAllText($canvasFile) -cne $canvasJson) { throw 'The imported original was modified on disk.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: physical Canvas marquee, canceled drag, equal group movement, reopen and Ctrl+Z, connect/disconnect and Undo, and exact export to a second workspace.'

    $groupFile = Join-Path $root 'CanvasGroupFixture.canvas'
    $groupBefore = Join-Path $root 'CanvasGroupBefore.canvas'
    $groupExport = Join-Path $root 'CanvasGroupReadback.canvas'
    $groupJson = '{"nodes":[{"id":"inner","type":"group","x":44,"y":50,"width":133,"height":96,"label":"內層","color":"#12a474","background":"assets/frame.png","backgroundStyle":"ratio","vendor":true},{"id":"a","type":"text","x":60,"y":75,"width":100,"height":55,"text":"鏡頭 A","color":"#ed217d"},{"id":"b","type":"text","x":235,"y":118,"width":90,"height":60,"text":"鏡頭 B","color":"#047bea"},{"id":"outside","type":"text","x":310,"y":158,"width":90,"height":60,"text":"部分相交","color":"#6f5092"}],"edges":[{"id":"e","fromNode":"a","toNode":"outside","label":"保留連線","color":"#a2572e","vendor":123}],"vendor":{"script":"合成測資"}}'
    [IO.File]::WriteAllText($groupFile,$groupJson,[Text.UTF8Encoding]::new($false))
    $groupInputHash=(Get-FileHash -LiteralPath $groupFile).Hash
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-File 11 '匯入 JSON Canvas…' $groupFile '中鍵拖曳平移'
    $inner = Wait-For 'original standard group is actually painted' { Read-CanvasInk 18 164 116 }
    $second = Wait-For 'second card is actually painted' { Read-CanvasInk 4 123 234 }
    Native-Menu 120 7 '編輯 Canvas…'
    Native-Menu 120 15 '從選取建立群組…' -MenuOpen
    $errorDialog = Wait-For 'empty selection refusal' { [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間') }
    if (![PaperroomHighlightGui]::DialogContains($errorDialog,$process.Id,'請先框選要放入群組的節點')) {
        throw 'The empty selection refusal was missing.'
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null = Wait-For 'closed the selection refusal' { ![PaperroomHighlightGui]::IsWindowVisible($errorDialog) }
    Drag-Canvas ($inner.ScreenX+$inner.L-5) ($inner.ScreenY+$inner.T-5) ($second.ScreenX+$second.R+5) ($second.ScreenY+$second.B+5)
    $null = Wait-For 'marquee selected the nested frame and two cards' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已選取 3 個畫布節點。'
    }
    Native-Menu 120 7 '編輯 Canvas…'
    Native-Menu 120 15 '從選取建立群組…' -MenuOpen
    $prompt = Wait-For 'native group name prompt' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','建立 Canvas 群組',1)
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null = Wait-For 'canceled group name prompt' { ![PaperroomHighlightGui]::IsWindowVisible($prompt) }
    Native-Menu 120 7 '編輯 Canvas…'
    Native-Menu 120 15 '從選取建立群組…' -MenuOpen
    $prompt = Wait-For 'name prompt after cancellation' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','建立 Canvas 群組',1)
    }
    Type-SearchFilter $prompt 101 '研究分鏡 📌'
    Finish-Revision $prompt 1
    $null = Wait-For 'named group saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存畫布修改。' }
    Native-Menu 120 8 '將畫布調整至視窗'
    $outer = Wait-For 'painted named group border' { Read-CanvasInk 23 98 78 }
    Native-Menu 120 1 '匯出原始 JSON Canvas…'
    Native-File 1 '匯出原始 JSON Canvas…' $groupBefore '已匯出原始 JSON Canvas。' -AlreadyOpened -CanvasSave
    $groupX = $outer.ScreenX+$outer.L+20
    $groupY = $outer.ScreenY+$outer.T+6
    Drag-Canvas $groupX $groupY ($groupX+50) ($groupY+32) -Cancel
    $null = Wait-For 'canceled physical drag restored the frame position' {
        $ink = Read-CanvasInk 23 98 78
        $ink -and $ink.L -eq $outer.L -and $ink.T -eq $outer.T
    }
    Drag-Canvas $groupX $groupY ($groupX+50) ($groupY+32)
    $null = Wait-For 'dragging the outer frame saved a single Canvas edit' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存畫布修改。'
    }
    Native-Menu 120 1 '匯出原始 JSON Canvas…'
    Native-File 1 '匯出原始 JSON Canvas…' $groupExport '已匯出原始 JSON Canvas。' -AlreadyOpened -CanvasSave
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-Menu 109 12 '開啟已匯入的 Canvas…'
    Native-Menu 109 2 'CanvasGroupFixture' -MenuOpen
    $null = Wait-For 'reopened Canvas group painted' { Read-CanvasInk 23 98 78 }
    Assert-NamedCanvasGroup $liveDb 'CanvasGroupFixture' $groupFile $groupBefore $groupExport 'moved'
    $surface = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
    $x=$surface.Right-10
    $y=$surface.Bottom-10
    Drag-Canvas $x $y $x $y
    $null = Wait-For 'reopened Canvas has focus' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已選取 0 個畫布節點。'
    }
    [PaperroomHighlightGui]::CanvasUndoKey($window,$process.Id)
    $null = Wait-For 'Ctrl+Z reverted the group move after reopen' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已復原上一次 Canvas 修改。'
    }
    Assert-NamedCanvasGroup $liveDb 'CanvasGroupFixture' $groupFile $groupBefore $groupExport 'undone'
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $restored
    $window = $process.MainWindowHandle
    Native-File 11 '匯入 JSON Canvas…' $groupExport '中鍵拖曳平移'
    $null = Wait-For 'standard moved group painted in a separate workspace' { Read-CanvasInk 23 98 78 }
    Assert-NamedCanvasGroup $restoreDb 'CanvasGroupReadback' $groupFile $groupBefore $groupExport 'imported'
    if ((Get-FileHash -LiteralPath $groupFile).Hash -cne $groupInputHash) { throw 'Source .canvas was changed.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: native named Canvas group from a physical marquee, nested frame and fully enclosed cards translate together, overlap is left in place, empty selection and Escape preserve data, Ctrl+Z after reopening, and standard .canvas export and reimport into a separate workspace.'

    $searchVault = Join-Path $root 'SearchFixture'
    $folder = Join-Path $searchVault '製片'
    $other = Join-Path $searchVault '製片舊'
    [void][IO.Directory]::CreateDirectory($folder)
    [void][IO.Directory]::CreateDirectory($other)
    $searchTitle = 'SearchSameTake'
    $keyword = '訊號🧪'
    $searchBody = "---`r`ntags: [製片/早班]`r`n---`r`n📝 左側 $keyword 右側`r`n"
    $chosen = Join-Path $folder "$searchTitle.md"
    [IO.File]::WriteAllText($chosen,$searchBody,[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $other "$searchTitle.md"),"同名另一版 $keyword #製片/夜班",[Text.UTF8Encoding]::new($false))
    $originalSearchHash = (Get-FileHash -LiteralPath $chosen).Hash
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-VaultFolder $searchVault '附件綁定 0 筆，未解析 0 筆'
    $dialog = Open-WorkspaceSearch
    Type-SearchFilter $dialog 101 $keyword
    Search-Workspace $dialog '找到 2 筆筆記。' 2
    Type-SearchFilter $dialog 102 '=#製片'
    Search-Workspace $dialog '沒有符合條件的筆記。' 0
    Type-SearchFilter $dialog 102 '#製片'
    Type-SearchFilter $dialog 103 '../製片'
    Search-Workspace $dialog '請使用 Vault 相對資料夾路徑。' 0
    Type-SearchFilter $dialog 103 '製片'
    Search-Workspace $dialog '找到 1 筆筆記。' 1 "左側 〔$keyword〕 右側"
    if (![PaperroomHighlightGui]::Read((Control $dialog 105)).Contains("製片/$searchTitle.md")) {
        throw 'The identical titles were not disambiguated by the exact source folder.'
    }
    Finish-Revision $dialog 111
    Assert-SearchSelection 104 $keyword '全域搜尋：正文第 4 行'
    Native-Menu 113 3 '移到垃圾桶'
    $dialog = Wait-For 'native trash confirmation for the selected search hit' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'移到垃圾桶')
    }
    Finish-Revision $dialog 1
    $null = Wait-For 'trashed result left the active note' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -cne $searchTitle
    }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    $dialog = Open-WorkspaceSearch -Menu
    Type-SearchFilter $dialog 101 $keyword
    Type-SearchFilter $dialog 103 '製片'
    Search-Workspace $dialog '沒有符合條件的筆記。' 0
    Type-SearchFilter $dialog 101 '訊號'
    Type-SearchFilter $dialog 103 '製片舊'
    Search-Workspace $dialog '找到 1 筆筆記。' 1 '同名另一版 〔訊號〕'
    Finish-Revision $dialog 2
    $trash = Control $window 107
    $point = [PaperroomHighlightGui]::ButtonPoint($window,$process.Id,$trash)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$trash,$point,0)
    $null = Wait-For 'the trashed search hit can be restored' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq $searchTitle -and
        [PaperroomHighlightGui]::Read((Control $window 113)) -ceq '還原'
    }
    $restore = Control $window 113
    $point = [PaperroomHighlightGui]::ButtonPoint($window,$process.Id,$restore)
    [PaperroomHighlightGui]::MouseAction($window,$process.Id,$restore,$point,0)
    $null = Wait-For 'restored search hit reentered the workspace' {
        [PaperroomHighlightGui]::Read((Control $window 113)) -ceq '更多'
    }
    $dialog = Open-WorkspaceSearch
    Type-SearchFilter $dialog 101 'sameTake'
    Type-SearchFilter $dialog 103 '製片'
    Search-Workspace $dialog '找到 1 筆筆記。' 1 'Search〔SameTake〕'
    Finish-Revision $dialog 111
    Assert-SearchSelection 103 'SameTake' '全域搜尋：標題命中'
    if ((Get-FileHash -LiteralPath $chosen).Hash -cne $originalSearchHash) {
        throw 'Search, trash or restore changed the original Vault Markdown bytes.'
    }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: native workspace search shortcut and menu, indexed CJK and emoji, short terms, exact folder and tag filters, context, precise source selection, invalid filter, trash exclusion after reopen, and restore.'

    $linkVault = Join-Path $root 'LinkFixture'
    foreach ($folder in '現場','甲','乙') {
        [void][IO.Directory]::CreateDirectory((Join-Path $linkVault $folder))
    }
    $linkBody = "# 入口`r`n`r`n[到乙](../乙/LinkTwin.md)`r`n[到標題](../乙/LinkTwin.md#細節)`r`n[[LinkTwin|有歧義]]`r`n[外部](https://example.invalid/LinkTwin.md)`r`n"
    $senderFile = Join-Path $linkVault '現場/LinkSender.md'
    [IO.File]::WriteAllText($senderFile,$linkBody,[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $linkVault '甲/LinkTwin.md'),"甲的原文",[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $linkVault '乙/LinkTwin.md'),"# 細節`r`n乙的原文",[Text.UTF8Encoding]::new($false))
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Native-VaultFolder $linkVault '附件綁定 0 筆，未解析 0 筆'
    Open-LinkedFixture 'LinkSender' '現場'
    Native-Menu 120 2 '前往　到乙　｜ LinkTwin　｜ 乙/LinkTwin.md' -CaptionPrefix
    $null = Wait-For 'plain Markdown opened the correct same-name target' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkTwin' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('乙的原文')
    }
    Native-Menu 120 2 '反向連結　LinkSender　｜ 現場/LinkSender.md' -CaptionPrefix
    $null = Wait-For 'clicked Markdown backlink returned to the source' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkSender'
    }
    Native-Menu 120 3 '前往　到標題 #細節' -CaptionPrefix
    $dialog = Wait-For 'native Markdown heading excerpt' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','#細節',1)
    }
    if (![PaperroomHighlightGui]::Read((Control $dialog 101)).Contains('乙的原文')) {
        throw 'The Markdown heading preview did not show the expected same-name source.'
    }
    Finish-Revision $dialog 1
    $headingEditor = Control $window 104
    $null = Wait-For 'Markdown heading source selection after closing preview' {
        $span = [PaperroomHighlightGui]::Send($headingEditor,0x00B0,[IntPtr]::Zero,[IntPtr]::Zero).ToInt64()
        [PaperroomHighlightGui]::IsWindowVisible($headingEditor) -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '標題連結：第 1 行，細節' -and
        ($span -band 0xffff) -eq 0 -and ($span -shr 16) -eq '# 細節'.Length
    }
    Write-Host 'Verified the Markdown heading excerpt and exact source selection.'
    Native-Menu 120 2 '反向連結　LinkSender　｜ 現場/LinkSender.md' -CaptionPrefix
    $null = Wait-For 'returned from heading to backlink source' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkSender'
    }
    Find-LinkTarget 'MissingLinkTarget'
    $null = Wait-For 'unknown link target produced an empty picker message' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '沒有符合條件的連結目標。'
    }
    Find-LinkTarget '甲/LinkTwin.md'
    $pick = @'
from pathlib import Path
from contextlib import closing
import json,sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    rows=db.execute("SELECT p.import_root,p.note_id,i.token FROM vault_paths p JOIN notes n ON n.id=p.import_root "
                    "JOIN note_link_identities i ON i.note_id=p.note_id "
                    "WHERE n.title='\u532f\u5165\uff1aLinkFixture' AND p.relative_path='\u7532/LinkTwin.md'").fetchall()
    assert len(rows)==1
    print(json.dumps(rows[0]))
'@ | python - $liveDb
    if ($LASTEXITCODE -ne 0 -or !$pick) { throw 'Could not validate the exact same-name link target.' }
    $ids = $pick | ConvertFrom-Json
    Native-Menu 120 1 "LinkTwin　匯入：LinkFixture (#$($ids[0])) / 甲/LinkTwin.md　[#$($ids[1])]" -MenuOpen
    $markup = "[[paperroom:note:$($ids[1]):$($ids[2])|LinkTwin]]"
    $null = Wait-For 'new link appended to the original text and saved' {
        [PaperroomHighlightGui]::Read((Control $window 104)) -ceq ($linkBody+$markup) -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
    }
    Open-LinkedFixture 'LinkTwin' '甲'
    Click-WorkspaceButton 110
    $null = Wait-For 'rename editor opened for the selected exact target' {
        [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) -and
        [PaperroomHighlightGui]::Read((Control $window 104)) -ceq '甲的原文'
    }
    Replace-EditorText (Control $window 103) 'LinkRenamed'
    $null = Wait-For 'renamed target saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦' }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $live
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'LinkSender' '現場'
    Native-Menu 120 4 '前往　LinkTwin　｜ LinkRenamed　｜ 甲/LinkTwin.md' -CaptionPrefix
    $null = Wait-For 'renamed stable link opened the original note after restart' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkRenamed' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲的原文')
    }
    Native-Menu 113 3 '移到垃圾桶'
    $dialog = Wait-For 'trash confirmation for exact link target' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'移到垃圾桶')
    }
    Finish-Revision $dialog 1
    $null = Wait-For 'trashed link target is no longer active' { [PaperroomHighlightGui]::Read((Control $window 103)) -cne 'LinkRenamed' }
    Open-LinkedFixture 'LinkSender' '現場'
    Native-Menu 120 4 '開啟筆記關聯圖…'
    $null = Wait-For 'no undeleted namesake took over a missing or ambiguous link' {
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '關聯圖操作'
    }
    Click-WorkspaceButton 107
    $null = Wait-For 'exact title in trash can be restored' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkRenamed' -and
        [PaperroomHighlightGui]::Read((Control $window 113)) -ceq '還原'
    }
    Click-WorkspaceButton 113
    $null = Wait-For 'original link target restored' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkRenamed' -and
        [PaperroomHighlightGui]::Read((Control $window 113)) -ceq '更多'
    }
    Native-Menu 120 2 '反向連結　LinkSender　｜ 現場/LinkSender.md' -CaptionPrefix
    $null = Wait-For 'restored stable link has the original backlink' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'LinkSender'
    }
    if ([IO.File]::ReadAllText($senderFile) -cne $linkBody) { throw 'Link insertion changed the original imported Vault file.' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Write-Host 'PASS: native Markdown note and heading links, precise backlinks with source path, no result, stable same-name insertion, rename and reopen, trash without retargeting, restoration and preserved source file.'

    $portableSource = Join-Path $root 'PortableFixture'
    $portableFiles = Join-Path $portableSource '場'
    $portableLive = Join-Path $root 'portable-origin'
    $portableNew = Join-Path $root 'portable-destination'
    foreach ($folder in $portableFiles,$portableLive,$portableNew) {
        [void][IO.Directory]::CreateDirectory($folder)
    }
    foreach ($folder in $portableLive,$portableNew) { Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe') }
    $plain = "[相對引用](./PortableB.md)`r`n"
    $originalA = Join-Path $portableFiles 'PortableA.md'
    [IO.File]::WriteAllText($originalA,$plain,[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $portableFiles 'PortableB.md'),'精確的 B 內容',[Text.UTF8Encoding]::new($false))
    $portableZip = Join-Path $root 'PortableFixture.zip'
    $process = Start-Notebook $portableLive
    $window = $process.MainWindowHandle
    Native-VaultFolder $portableSource '附件綁定 0 筆，未解析 0 筆'
    Open-LinkedFixture 'PortableA' '場'
    Find-LinkTarget '場/PortableB.md'
    Native-Menu 120 1 'PortableB　匯入：PortableFixture' -MenuOpen -CaptionPrefix
    $null = Wait-For 'selector created the scoped note link' {
        [PaperroomHighlightGui]::Read((Control $window 104)) -cmatch '\[\[paperroom:note:\d+:[0-9a-f]{32}\|PortableB\]\]' -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
    }
    Native-Menu 120 3 '前往　PortableB　｜ PortableB' -CaptionPrefix
    $null = Wait-For 'scope link opens B before rename' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableB' }
    Click-WorkspaceButton 110
    $null = Wait-For 'B editor visible' { [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) }
    Replace-EditorText (Control $window 103) 'PortableRenamed'
    $null = Wait-For 'B rename persisted' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦' }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $portableLive
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'PortableA' '場'
    Native-Menu 120 2 '前往　相對引用　｜ PortableRenamed' -CaptionPrefix
    $null = Wait-For 'relative link opens B after a title-only change and restart' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableRenamed'
    }
    Native-Menu 120 2 '反向連結　PortableA' -CaptionPrefix
    $null = Wait-For 'returned to A' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableA' }
    Native-Menu 120 3 '前往　PortableB　｜ PortableRenamed' -CaptionPrefix
    $null = Wait-For 'scoped link opens the same B after rename and restart' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableRenamed'
    }
    if ($ProvePortableLinkGap) {
        Native-Menu 109 34 '匯出 Vault ZIP（含來源附件）…'
        Native-Menu 109 1 '匯入：PortableFixture  (#' -MenuOpen -CaptionPrefix
        Native-File 34 '匯出 Vault ZIP（含來源附件）…' $portableZip '已匯出 2 篇 Markdown 與 0 個來源附件。' -AlreadyOpened
    } else {
        Native-Menu 109 36 'Vault ZIP 可攜連結…'
        Native-Menu 109 1 '匯出可攜 Vault ZIP…' -MenuOpen
        Native-Menu 109 1 '匯入：PortableFixture  (#' -MenuOpen -CaptionPrefix
        Native-File 36 '匯出可攜 Vault ZIP…' $portableZip '已匯出 2 篇 Markdown 與 0 個來源附件。已加入可攜連結清單。' -AlreadyOpened
    }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    if ($ProvePortableLinkGap) {
        $portableUnpacked = Join-Path $root 'PortableReadback'
        @'
from pathlib import Path
from zipfile import ZipFile
import re,sys
parent="\u5834"
with ZipFile(sys.argv[1]) as z:
    assert set(z.namelist()) == {f"{parent}/PortableA.md",f"{parent}/PortableB.md"}
    assert z.testzip() is None
    a=z.read(f"{parent}/PortableA.md")
    assert b"./PortableB.md" in a and re.search(rb'paperroom:note:\d+:[0-9a-f]{32}\|PortableB',a)
    z.extractall(sys.argv[2])
print("Baseline export contains both reference types and exactly two Markdown members.")
'@ | python - $portableZip $portableUnpacked
        if ($LASTEXITCODE -ne 0) { throw 'Actual baseline Vault ZIP failed its two-note check.' }
        $process = Start-Notebook $portableNew
        $window = $process.MainWindowHandle
        Native-VaultFolder $portableUnpacked '附件綁定 0 筆，未解析 0 筆'
        Open-LinkedFixture 'PortableA' '場'
        Native-Menu 120 2 '前往　相對引用　｜ PortableB' -CaptionPrefix
        $null = Wait-For 'relative link works in the fresh workspace' {
            [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableB' -and
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains('精確的 B 內容')
        }
        Native-Menu 120 2 '反向連結　PortableA' -CaptionPrefix
        $null = Wait-For 'returned to A in the fresh workspace' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableA' }
        Native-Menu 120 3 '開啟筆記關聯圖…'
        $null = Wait-For 'scoped link absent in the fresh workspace' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '關聯圖操作' }
        if ([IO.File]::ReadAllText($originalA) -cne $plain) { throw 'The baseline altered original A.' }
        Close-Notebook $process
        $process.Dispose()
        $process = $null
        Write-Host 'REPRODUCED on the pinned EXE: both links survive a same-workspace rename and restart; after actual Vault ZIP export, extraction and fresh workspace import, only the relative Markdown link opens B.'
    } else {
        $badZip = Join-Path $root 'PortableChanged.zip'
        $initialZipHash = (Get-FileHash -LiteralPath $portableZip).Hash
        @'
from hashlib import sha256
from zipfile import ZipFile
import json,re,sys
parent="\u5834"
a,b=(f"{parent}/Portable{name}.md" for name in ("A","B"))
manifest="Paperroom-Vault-Links.json"
with ZipFile(sys.argv[1]) as z:
    assert set(z.namelist()) == {a,b,manifest}
    notes={n["path"]:n for n in json.loads(z.read(manifest))["notes"]}
    assert all(sha256(z.read(name)).hexdigest()==notes[name]["sha256"] for name in (a,b))
    assert notes[b]["title"]=="PortableRenamed"
    assert b"./PortableB.md" in z.read(a) and notes[b]["identity"].encode() in z.read(a)
    with ZipFile(sys.argv[2],"w") as changed:
        for member in z.namelist():
            payload=z.read(member)
            changed.writestr(member, payload+b" changed" if member==b else payload)
print("Confirmed exact source and target archive members. Prepared a synthetic target checksum mismatch.")
'@ | python - $portableZip $badZip
        if ($LASTEXITCODE -ne 0) { throw 'Actual portable ZIP failed its two-note and manifest check.' }
        $process = Start-Notebook $portableNew
        $window = $process.MainWindowHandle
        Native-Menu 109 36 'Vault ZIP 可攜連結…'
        Native-Menu 109 2 '匯入可攜 Vault ZIP…' -MenuOpen
        Native-File 37 '匯入可攜 Vault ZIP…' $badZip '拒絕錯誤校驗' -AlreadyOpened -ExpectedError '筆記路徑、身分、標題或校驗不符'
        Native-Menu 109 36 'Vault ZIP 可攜連結…'
        Native-Menu 109 2 '匯入可攜 Vault ZIP…' -MenuOpen
        Native-File 37 '匯入可攜 Vault ZIP…' $portableZip '已匯入 2 篇筆記，重綁 1 個紙間連結。' -AlreadyOpened
        $dialog = Wait-For 'native portable Vault import report' {
            [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','可攜 Vault 匯入報告',1)
        }
        if (![PaperroomHighlightGui]::Read((Control $dialog 101)).Contains('重綁 1 個紙間連結。')) {
            throw 'The import report did not account for the one verified link.'
        }
        Finish-Revision $dialog 1
        Close-Notebook $process
        $process.Dispose()
        $process = Start-Notebook $portableNew
        $window = $process.MainWindowHandle
        Open-LinkedFixture 'PortableA' '場'
        Native-Menu 120 2 '前往　相對引用　｜ PortableRenamed' -CaptionPrefix
        $null = Wait-For 'relative link opens the exact imported B after restart' {
            [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableRenamed' -and
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains('精確的 B 內容')
        }
        Native-Menu 120 2 '反向連結　PortableA' -CaptionPrefix
        $null = Wait-For 'relative backlink returned to imported A' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableA' }
        Native-Menu 120 3 '前往　PortableB　｜ PortableRenamed' -CaptionPrefix
        $null = Wait-For 'rebound identity link opens the exact imported B after restart' {
            [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'PortableRenamed' -and
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains('精確的 B 內容')
        }
        Close-Notebook $process
        $process.Dispose()
        $process = $null
        $destinationDb = Join-Path $portableNew 'paperroom-data/notes.sqlite3'
        @'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import json,sqlite3,sys
parent="\u5834"
a,b=(f"{parent}/Portable{name}.md" for name in ("A","B"))
with ZipFile(sys.argv[1]) as z, closing(sqlite3.connect(Path(sys.argv[2]).as_uri()+"?mode=ro",uri=True)) as db:
    notes={n["path"]:n for n in json.loads(z.read("Paperroom-Vault-Links.json"))["notes"]}
    raw=z.read(a)
    assert not db.execute("SELECT 1 FROM notes WHERE title=?", ("\u532f\u5165\uff1aPortableChanged",)).fetchone()
    rows=db.execute("SELECT p.relative_path,n.id,n.title,n.body,t.token FROM vault_paths p "
                    "JOIN notes n ON n.id=p.note_id JOIN note_link_identities t ON t.note_id=n.id").fetchall()
    assert len(rows)==2
    saved={row[0]:row[1:] for row in rows}
    assert saved[b][1]=="PortableRenamed" and saved[b][2].encode()==z.read(b)
    prior=notes[b]["identity"].encode()
    after=f"paperroom:note:{saved[b][0]}:{saved[b][3]}".encode()
    assert after!=prior and saved[a][2].encode()==raw.replace(prior,after,1)
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
print("Verified exact B content and identity separately from its display title; A differs from the archived bytes only at the scoped identity.")
'@ | python - $portableZip $destinationDb
        if ($LASTEXITCODE -ne 0) { throw 'Actual reopened workspace failed its archived link and raw byte checks.' }
        if ((Get-FileHash -LiteralPath $portableZip).Hash -cne $initialZipHash -or
            [IO.File]::ReadAllText($originalA) -cne $plain) { throw 'Import changed the original ZIP or Vault Markdown.' }
        Write-Host 'PASS: native two-note portable Vault ZIP, verified rebinding, both link types after restart, independent title and content checks, checksum refusal without partial import, and untouched sources.'
    }

    $sortFolder = Join-Path $root 'derived-sort-workspace'
    [void][IO.Directory]::CreateDirectory($sortFolder)
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $sortFolder 'Paperroom.exe')
    $sortDb = Join-Path $sortFolder 'paperroom-data/notes.sqlite3'
    $sortCsv = Join-Path $root 'SortAudition.csv'
    $sortFixture = @'
標題,工時
Rank-A,7
Rank-B,7
Rank-Bad,壞值
Rank-Empty,
Rank-Zero,0
Lookup-High,8
Lookup-Low,2
'@
    [IO.File]::WriteAllText($sortCsv,$sortFixture,[Text.UTF8Encoding]::new($false))
    $csvHash = (Get-FileHash -LiteralPath $sortCsv).Hash
    $process = Start-Notebook $sortFolder
    $window = $process.MainWindowHandle
    Native-File 9 '匯入 CSV 表格…' $sortCsv '已從 CSV 匯入 7 列'
    Native-Menu 109 14 '屬性型別…'
    Native-Menu 109 1 '設定屬性型別…' -MenuOpen
    Collection-Prompt '設定工作區屬性型別' '工時'
    Native-Menu 109 1 '數字' -MenuOpen
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 2 '新增集合…' -MenuOpen
    Collection-Prompt '新增集合' 'SortAudition'
    $null = Wait-For 'new native collection' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
    foreach ($title in @('Rank-A','Rank-B','Rank-Bad','Rank-Empty','Rank-Zero','Lookup-High','Lookup-Low')) {
        Native-Menu 120 1 '加入筆記…'
        Choose-CollectionRow "$title  ("
        Collection-Prompt '加入集合'
    }
    foreach ($pair in @(@('Rank-A','Lookup-High'),@('Rank-B','Lookup-Low'))) {
        Native-Menu 120 19 '編輯資料列關聯…'
        Choose-CollectionRow "待處理　$($pair[0])  (#"
        Native-Menu 120 1 '新增同集合關聯…' -MenuOpen
        Choose-CollectionRow "同集合　待處理　$($pair[1])  (#"
        Collection-Prompt '新增集合關聯' '明細'
    }
    Native-Menu 120 4 '新增公式…'
    Collection-Prompt '新增公式' '審閱順位'
    Collection-Prompt '新增公式' 'typed("工時")'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 1 '儲存新檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' 'Ranked'
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '集合檢視'
    Collection-Prompt '集合檢視' 'Rank-'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
    Set-SortColumn 'formula'
    Set-SortColumn 'rollup'
    Native-Menu 120 5 '儲存變更' -MenuOpen
    Assert-RankedRows @('Rank-B','Rank-A','Rank-Zero','Rank-Bad','Rank-Empty')
    Open-SortCollection
    Open-SortNamedView 1 'Ranked'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
    Native-Menu 120 9 '查看衍生欄位的篩選或排序錯誤…' -MenuOpen
    $problems = Wait-For 'native rule problem report' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','衍生欄位的篩選與排序錯誤',1)
    }
    $details = [PaperroomHighlightGui]::Read((Control $problems 101))
    if ([regex]::Matches($details,'排序　公式「審閱順位」').Count -ne 2 -or !$details.Contains('工時')) {
        throw "Expected two legacy invalid fields in the sorting diagnostics."
    }
    Finish-Revision $problems 1
    Native-Menu 120 6 '取消所有未儲存變更' -MenuOpen
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 3 '顯示全部資料列（看板）' -MenuOpen
    Native-Menu 120 18 '編輯資料列的型別欄位…'
    Choose-CollectionRow '待處理　Lookup-High  ('
    Native-Menu 120 1 '工時　8' -MenuOpen -CaptionPrefix
    Collection-Prompt '工時' '1'
    Open-SortNamedView 1 'Ranked'
    Assert-RankedRows @('Rank-A','Rank-B','Rank-Zero','Rank-Bad','Rank-Empty')
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $sortFolder
    $window = $process.MainWindowHandle
    Open-SortCollection
    Open-SortNamedView 1 'Ranked'
    Assert-RankedRows @('Rank-A','Rank-B','Rank-Zero','Rank-Bad','Rank-Empty')
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    @'
from pathlib import Path
from contextlib import closing
import json,sqlite3,sys
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
    spec,keyword=db.execute("SELECT rules_json,keyword FROM collection_views WHERE name='Ranked'").fetchone()
    assert keyword=="Rank-" and json.loads(spec)==[False,[],[["\u5be9\u95b1\u9806\u4f4d","formula-number",True],["\u660e\u7d30","rollup-sum",False,"\u5de5\u6642"]]]
    vals=dict(db.execute("SELECT n.title,p.value FROM notes n JOIN note_properties p ON n.id=p.note_id WHERE p.key=?",
                         ("\u5de5\u6642",)))
    assert vals["Rank-Bad"]=="\u58de\u503c" and vals["Rank-Empty"]=="" and vals["Rank-Zero"]=="0" and vals["Lookup-High"]=="1"
    assert db.execute("SELECT count(*) FROM note_relations").fetchone()==(2,)
print("Verified exact saved view rules, invalid and empty values, live target edit, stable links and SQLite integrity.")
'@ | python - $sortDb
    if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath $sortCsv).Hash -cne $csvHash) {
        throw 'Saved ordering proof or original CSV bytes changed.'
    }
    Write-Host 'PASS: native named collection view uses live numeric formula and rollup sorts with deterministic ties, zero before invalid and empty, native diagnostics, an edited related value, and identical order after restart.'

    $reviewHome = Join-Path $root 'folder-review-workspace'
    $reviewFolder = Join-Path $root 'ReviewFixture'
    $reviewNotes = Join-Path $reviewFolder '場'
    $reviewMedia = Join-Path $reviewFolder '素材'
    foreach ($dir in @($reviewHome,$reviewNotes,$reviewMedia)) { [void][IO.Directory]::CreateDirectory($dir) }
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $reviewHome 'Paperroom.exe')
    $reviewA = Join-Path $reviewNotes 'ReviewA.md'
    $reviewB = Join-Path $reviewNotes 'ReviewB.md'
    $otherB = Join-Path $reviewFolder 'ReviewB.md'
    $reviewAsset = Join-Path $reviewMedia '卡.txt'
    $enc = [Text.UTF8Encoding]::new($false)
    $reviewMarkdown = "[相對頁](./ReviewB.md#review)" + [Environment]::NewLine + "[雙義頁](ReviewB.md#review)" +
        [Environment]::NewLine + "[附件](../素材/卡.txt)" + [Environment]::NewLine + "[缺檔](../素材/缺.txt)"
    [IO.File]::WriteAllText($reviewA,$reviewMarkdown,$enc)
    [IO.File]::WriteAllText($reviewB,"# Review" + [Environment]::NewLine + "場內的精確內容",$enc)
    [IO.File]::WriteAllText($otherB,"# Review" + [Environment]::NewLine + "根目錄同名頁",$enc)
    [IO.File]::WriteAllText($reviewAsset,'asset one',$enc)
    $checks = @{}
    foreach ($file in @($reviewA,$reviewB,$otherB,$reviewAsset)) { $checks[$file]=(Get-FileHash -LiteralPath $file).Hash }
    $process = Start-Notebook $reviewHome
    $window = $process.MainWindowHandle
    $reviewDb = Join-Path $reviewHome 'paperroom-data/notes.sqlite3'
    $before = Get-VaultReviewStamp $reviewDb
    $review = Open-VaultFolderReview $reviewFolder
    $preview = [PaperroomHighlightGui]::Read((Control $review 101))
    foreach ($expected in @('附件綁定 1 筆，未解析 1 筆','相符 1 筆，未解析 1 筆',
                            '相對於筆記與相對於 Vault 根目錄時指向不同筆記','找不到可匯入的來源檔案')) {
        if (!$preview.Contains($expected)) { throw "Expected warning '$expected' in the real preview." }
    }
    if ((Get-VaultReviewStamp $reviewDb) -cne $before) { throw 'The preview changed persisted tables.' }
    Finish-Revision $review 2
    if ((Get-VaultReviewStamp $reviewDb) -cne $before) { throw 'Cancel wrote to the workspace.' }
    $review = Open-VaultFolderReview $reviewFolder
    [IO.File]::WriteAllText($reviewAsset,'asset two',$enc)
    Finish-Revision $review 1
    $errorDialog = Wait-For 'changed attachment refused after consent' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
    }
    if (![PaperroomHighlightGui]::DialogContains($errorDialog,$process.Id,'預覽後 Vault 來源已有變更')) {
        throw 'The changed-file error did not explain how to re-open the preview.'
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null = Wait-For 'changed-file alert closed' { ![PaperroomHighlightGui]::IsWindowVisible($errorDialog) }
    if ((Get-VaultReviewStamp $reviewDb) -cne $before) { throw 'A stale preview changed the workspace.' }
    [IO.File]::WriteAllText($reviewAsset,'asset one',$enc)
    Native-VaultFolder $reviewFolder '附件綁定 1 筆，未解析 1 筆'
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $reviewHome
    $window = $process.MainWindowHandle
    $dialog = Open-WorkspaceSearch -Menu
    Type-SearchFilter $dialog 101 'ReviewA'
    Type-SearchFilter $dialog 103 '場'
    Search-Workspace $dialog '找到 1 筆筆記。' 1
    Finish-Revision $dialog 111
    $null = Wait-For 'opened the imported source note after restart' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewA'
    }
    Native-Menu 120 2 '前往　相對頁' -CaptionPrefix
    $heading = Wait-For 'native relative heading preview' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','#review',1)
    }
    if (![PaperroomHighlightGui]::Read((Control $heading 101)).Contains('場內的精確內容')) {
        throw 'The relative heading preview opened the wrong same-name note.'
    }
    Finish-Revision $heading 1
    $null = Wait-For 'relative link opened the exact target by body, not title alone' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewB' -and
        ([PaperroomHighlightGui]::Read((Control $window 104))).Contains('場內的精確內容') -and
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('標題連結：第 1 行，Review',[StringComparison]::Ordinal)
    }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    @'
from contextlib import closing
from pathlib import Path
import sqlite3,sys
folder=Path(sys.argv[2])
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
    rows=db.execute("SELECT n.body,p.relative_path,p.import_root FROM notes n JOIN vault_paths p ON p.note_id=n.id").fetchall()
    assert len(rows)==3 and len({root for _,_,root in rows})==1
    for body,path,_ in rows:
        assert body.encode("utf-8")== (folder/path).read_bytes(), path
    path,data,source=db.execute("SELECT o.relative_path,a.content,p.relative_path FROM attachments a "
                                "JOIN attachment_origins o ON o.attachment_id=a.id JOIN vault_paths p ON p.note_id=a.note_id").fetchone()
    assert path=="\u7d20\u6750/\u5361.txt" and source=="\u5834/ReviewA.md"
    assert data== (folder/path).read_bytes()
    assert db.execute("SELECT count(*) FROM attachments").fetchone()==(1,)
print("Verified original Markdown paths and bytes, one fresh import root, exact attachment binding and integrity after restart.")
'@ | python - $reviewDb $reviewFolder
    if ($LASTEXITCODE -ne 0) { throw 'The real import failed its original-byte and SQLite checks.' }
    foreach ($file in $checks.Keys) {
        if ((Get-FileHash -LiteralPath $file).Hash -cne $checks[$file]) { throw 'A Vault source file was changed by the application.' }
    }
    Write-Host 'PASS: native read-only Vault import preview, missing attachment and ambiguous path report, cancellation and changed-file refusal without writes, successful fresh batch, exact attachment and relative-note navigation after reopening.'


    # Continue the same real Vault sample from preview and refusal through every feature.
    $jointCanvas = Join-Path $root 'IntegratedResearch.canvas'
    [IO.File]::WriteAllText($jointCanvas,'{"nodes":[{"id":"first","type":"file","file":"場/ReviewA.md","label":"同一張卡","x":-28000,"y":800,"width":120,"height":75,"color":"#ed217d"},{"id":"second","type":"file","file":"場/ReviewA.md","label":"同一張卡","x":28000,"y":-800,"width":120,"height":75,"color":"#12a474"}],"edges":[{"id":"follow","fromNode":"first","toNode":"second"}]}',$enc)
    $process = Start-Notebook $reviewHome
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'ReviewA' '場'
    Find-LinkTarget '場/ReviewB.md'
    Native-Menu 120 1 'ReviewB　匯入：ReviewFixture' -MenuOpen -CaptionPrefix
    $null = Wait-For 'ReviewA saved an identity link alongside its relative heading link' {
        [PaperroomHighlightGui]::Read((Control $window 104)) -cmatch '\[\[paperroom:note:\d+:[0-9a-f]{32}\|ReviewB\]\]' -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
    }
    Native-Menu 120 3 '前往　ReviewB　｜ ReviewB' -CaptionPrefix
    $null = Wait-For 'the stable link selected the nested source rather than its namesake' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewB' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('場內的精確內容')
    }
    Click-WorkspaceButton 110
    $null = Wait-For 'target note editor visible for title-only rename' { [PaperroomHighlightGui]::IsWindowVisible((Control $window 104)) }
    Replace-EditorText (Control $window 103) 'ReviewedB'
    $null = Wait-For 'same note saved with new display title' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewedB' -and
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已儲存至此電腦'
    }
    Native-File 11 '匯入 JSON Canvas…' $jointCanvas '中鍵拖曳平移'
    Native-Menu 120 4 '綁定匯入的 Vault…'
    Native-Menu 120 1 '匯入：ReviewFixture  (' -MenuOpen -CaptionPrefix
    $null = Wait-For 'integrated Canvas bound to the exact previewed batch' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'
    }
    Open-LinkedFixture 'ReviewA' '場'
    Locate-NoteOnCanvas 1 'IntegratedResearch' 1 2 1 'first' @(237,33,125)
    Open-LocationSource 'ReviewA' '相對頁'
    Locate-NoteOnCanvas 1 'IntegratedResearch' 1 2 2 'second' @(18,164,116)
    Open-LocationSource 'ReviewA' '相對頁'

    Native-Menu 109 14 '屬性型別…'
    Native-Menu 109 1 '設定屬性型別…' -MenuOpen
    Collection-Prompt '設定工作區屬性型別' '工時'
    Native-Menu 109 1 '數字' -MenuOpen
    Native-Menu 109 10 '資料集合與看板…'
    Native-Menu 109 2 '新增集合…' -MenuOpen
    Collection-Prompt '新增集合' 'IntegratedReview'
    $null = Wait-For 'integrated collection opened' { [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '看板操作' }
    Set-Rule '工時' '0'
    foreach ($pair in @(@('ReviewA','3'),@('ReviewedB','7'))) {
        $beforeAdmission = Get-VaultReviewStamp $reviewDb
        Native-Menu 120 1 '加入筆記…'
        Choose-CollectionRow "$($pair[0])  ("
        Collection-Prompt '加入集合'
        if ($pair[0] -ceq 'ReviewA') {
            Rule-Prompt '工時' '0' ''
            Rule-Error '欄位值無效' '必填'
            if ((Get-VaultReviewStamp $reviewDb) -cne $beforeAdmission) {
                throw 'Rejected admission changed the source workspace.'
            }
            Rule-Prompt '工時' '' '3'
        } else {
            Rule-Prompt '工時' '0' $pair[1]
        }
        $null = Wait-For 'collection admission completed' {
            [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將筆記加入集合。'
        }
    }
    Native-Menu 120 4 '新增公式…'
    Collection-Prompt '新增公式' '審閱順位'
    Collection-Prompt '新增公式' 'typed("工時")'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 1 '儲存新檢視…' -MenuOpen
    Collection-Prompt '儲存新檢視' 'IntegratedOrder'
    Native-Menu 120 2 '表格' -MenuOpen
    Collection-Prompt '集合檢視'
    Collection-Prompt '集合檢視'
    Native-Menu 120 11 '集合檢視…'
    Native-Menu 120 11 '編輯篩選與多欄排序…' -MenuOpen
    Set-SortColumn 'formula'
    Native-Menu 120 5 '儲存變更' -MenuOpen
    Assert-RankedRows @('ReviewedB','ReviewA')
    Open-SortCollection 'IntegratedReview'
    Open-SortNamedView 1 'IntegratedOrder'
    $beforeEdit = Get-VaultReviewStamp $reviewDb
    Native-Menu 120 18 '編輯資料列的型別欄位…'
    Choose-CollectionRow '待處理　ReviewA  ('
    Choose-CollectionRow '工時　'
    Rule-Prompt '工時' '3' ''
    Rule-Error '欄位未儲存' '必填'
    if ((Get-VaultReviewStamp $reviewDb) -cne $beforeEdit) {
        throw 'Rejected field edit changed the source workspace.'
    }
    Rule-Prompt '工時' '' '8'
    $null = Wait-For 'corrected required property changes live sorting' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新資料列欄位。'
    }
    Assert-RankedRows @('ReviewA','ReviewedB')
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $reviewHome
    $window = $process.MainWindowHandle
    Open-SortCollection 'IntegratedReview'
    Open-SortNamedView 1 'IntegratedOrder'
    Assert-RankedRows @('ReviewA','ReviewedB')
    Open-LinkedFixture 'ReviewA' '場'
    Locate-NoteOnCanvas 1 'IntegratedResearch' 1 2 2 'second' @(18,164,116)
    Open-LocationSource 'ReviewA' '相對頁'

    $jointZip = Join-Path $root 'integrated-vault.zip'
    $brokenZip = Join-Path $root 'integrated-bad-checksum.zip'
    $jointNew = Join-Path $root 'integrated-import'
    [void][IO.Directory]::CreateDirectory($jointNew)
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $jointNew 'Paperroom.exe')
    Native-Menu 109 36 'Vault ZIP 可攜連結…'
    Native-Menu 109 1 '匯出可攜 Vault ZIP…' -MenuOpen
    Native-Menu 109 1 '匯入：ReviewFixture  (#' -MenuOpen -CaptionPrefix
    Native-File 36 '匯出可攜 Vault ZIP…' $jointZip '已匯出 3 篇 Markdown 與 1 個來源附件。已加入可攜連結清單。' -AlreadyOpened
    $jointBackup = Join-Path $root '整合工作區備份.sqlite3'
    Native-File 4 '備份工作區…' $jointBackup '工作區備份已完成'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $originHash = (Get-FileHash -LiteralPath $reviewDb).Hash
    $archiveHash = (Get-FileHash -LiteralPath $jointZip).Hash
    @'
from pathlib import Path
from zipfile import ZipFile
import json, sys
archive, bad, folder = map(Path, sys.argv[1:])
a, b, other, media = "\u5834/ReviewA.md", "\u5834/ReviewB.md", "ReviewB.md", "\u7d20\u6750/\u5361.txt"
manifest = "Paperroom-Vault-Links.json"
with ZipFile(archive) as z, ZipFile(bad, "w") as changed:
    assert z.testzip() is None and set(z.namelist()) == {a, b, other, media, manifest}
    notes = {n["path"]: n for n in json.loads(z.read(manifest))["notes"]}
    assert set(notes) == {a, b, other}
    assert notes[b]["title"] == "ReviewedB" and notes[b]["identity"].encode() in z.read(a)
    assert b"./ReviewB.md#review" in z.read(a)
    for path in (b, other, media):
        assert z.read(path) == (folder / path).read_bytes(), path
    for name in z.namelist():
        changed.writestr(name, z.read(name) + (b" changed" if name == b else b""))
print("Integrated ZIP: exact source paths, unchanged nested and decoy notes and attachment, saved title, and scoped identity.")
'@ | python - $jointZip $brokenZip $reviewFolder
    if ($LASTEXITCODE -ne 0) { throw 'Integrated Vault archive did not match the reviewed sample.' }

    $process = Start-Notebook $jointNew
    $window = $process.MainWindowHandle
    $jointDb = Join-Path $jointNew 'paperroom-data/notes.sqlite3'
    $empty = Get-VaultReviewStamp $jointDb
    Native-Menu 109 36 'Vault ZIP 可攜連結…'
    Native-Menu 109 2 '匯入可攜 Vault ZIP…' -MenuOpen
    Native-File 37 '匯入可攜 Vault ZIP…' $brokenZip '拒絕錯誤校驗' -AlreadyOpened -ExpectedError '筆記路徑、身分、標題或校驗不符'
    if ((Get-VaultReviewStamp $jointDb) -cne $empty) { throw 'Rejected checksum changed the clean workspace.' }
    Native-Menu 109 36 'Vault ZIP 可攜連結…'
    Native-Menu 109 2 '匯入可攜 Vault ZIP…' -MenuOpen
    Native-File 37 '匯入可攜 Vault ZIP…' $jointZip '已匯入 3 篇筆記，重綁 1 個紙間連結。' -AlreadyOpened
    $dialog = Wait-For 'integrated portable ZIP import report' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','可攜 Vault 匯入報告',1)
    }
    Finish-Revision $dialog 1
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $jointNew
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'ReviewA' '場'
    Native-Menu 120 2 '前往　相對頁' -CaptionPrefix
    $heading = Wait-For 'imported relative heading after re-opening destination' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32770','#review',1)
    }
    if (![PaperroomHighlightGui]::Read((Control $heading 101)).Contains('場內的精確內容')) {
        throw 'The reimported relative heading reached the wrong same-name document.'
    }
    Finish-Revision $heading 1
    $null = Wait-For 'reimported relative heading selected the exact original body' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewedB' -and
        [PaperroomHighlightGui]::Read((Control $window 104)).Contains('場內的精確內容') -and
        [PaperroomHighlightGui]::Read((Control $window 116)).StartsWith('標題連結：第 1 行，Review',[StringComparison]::Ordinal)
    }
    Open-LinkedFixture 'ReviewA' '場'
    Native-Menu 120 3 '前往　ReviewB　｜ ReviewedB' -CaptionPrefix
    $null = Wait-For 'rebound scoped identity after destination restart' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewedB' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('場內的精確內容')
    }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    @'
from contextlib import closing
from pathlib import Path
from zipfile import ZipFile
import json, sqlite3, sys
archive, destination, origin, canvas = map(Path, sys.argv[1:])
a, b, other, media = "\u5834/ReviewA.md", "\u5834/ReviewB.md", "ReviewB.md", "\u7d20\u6750/\u5361.txt"
with ZipFile(archive) as z, closing(sqlite3.connect(destination.as_uri() + "?mode=ro", uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
    rows = db.execute("SELECT p.relative_path,n.id,n.title,n.body,t.token FROM vault_paths p "
                      "JOIN notes n ON n.id=p.note_id JOIN note_link_identities t ON t.note_id=n.id").fetchall()
    notes = {row[0]: row[1:] for row in rows}
    assert len(rows) == 3 and set(notes) == {a, b, other} and notes[b][1] == "ReviewedB"
    old = {n["path"]: n for n in json.loads(z.read("Paperroom-Vault-Links.json"))["notes"]}[b]["identity"].encode()
    new = f"paperroom:note:{notes[b][0]}:{notes[b][3]}".encode()
    assert old != new and notes[a][2].encode() == z.read(a).replace(old, new, 1)
    for path in (b, other):
        assert notes[path][2].encode() == z.read(path)
    assert db.execute("SELECT o.relative_path,a.content,p.relative_path FROM attachments a "
                      "JOIN attachment_origins o ON o.attachment_id=a.id JOIN vault_paths p ON p.note_id=a.note_id").fetchall() == [(media, z.read(media), a)]
    assert db.execute("SELECT COUNT(*) FROM saved_canvases").fetchone() == (0,)
    assert db.execute("SELECT COUNT(*) FROM collection_views").fetchone() == (0,)
    assert db.execute("SELECT COUNT(*) FROM collection_field_rules").fetchone() == (0,)
with closing(sqlite3.connect(origin.as_uri() + "?mode=ro", uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
    assert db.execute("SELECT source FROM saved_canvases WHERE title='IntegratedResearch'").fetchone()[0].encode() == canvas.read_bytes()
    assert db.execute("SELECT COUNT(*) FROM canvas_cameras").fetchone() == (0,)
    assert db.execute("SELECT COUNT(*) FROM collection_views WHERE name='IntegratedOrder'").fetchone() == (1,)
print("Integrated workspace: ZIP rebinds only the stable identity, includes the exact attachment, and leaves the source Canvas and collection intact.")
'@ | python - $jointZip $jointDb $reviewDb $jointCanvas
    if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath $jointZip).Hash -cne $archiveHash) {
        throw 'Integrated readback changed data or the input ZIP.'
    }
    foreach ($file in $checks.Keys) {
        if ((Get-FileHash -LiteralPath $file).Hash -cne $checks[$file]) { throw 'Integrated workflow wrote into the source Vault.' }
    }
    Write-Host 'PASS: same real Vault sample across preview refusal, note links, repeated Canvas locations, saved formula sorting, checksum refusal and portable ZIP re-import after restart.'


    # Follow the documented separate-folder restore, using only the downloaded EXE and the UI backup.
    $fullRestore = Join-Path $root 'full-workspace-restore'
    $fullData = Join-Path $fullRestore 'paperroom-data'
    [void](New-Item -ItemType Directory -Path $fullRestore)
    [void](New-Item -ItemType Directory -Path $fullData)
    $fullDb = Join-Path $fullData 'notes.sqlite3'
    $backupHash = (Get-FileHash -LiteralPath $jointBackup).Hash
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $fullRestore 'Paperroom.exe')
    Copy-Item -LiteralPath $jointBackup -Destination $fullDb
    if ((Get-FileHash -LiteralPath $fullDb).Hash -cne $backupHash -or
        @(Get-ChildItem -LiteralPath $fullData -Force).Count -ne 1) {
        throw 'A separate workspace must start with the exact backup bytes and no SQLite sidecars.'
    }
    $process = Start-Notebook $fullRestore
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'ReviewA' '場'
    Native-Menu 120 3 '前往　ReviewB　｜ ReviewedB' -CaptionPrefix
    $null = Wait-For 'first opening of restored workspace resolves the renamed identity' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewedB' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('場內的精確內容')
    }
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $fullRestore
    $window = $process.MainWindowHandle
    Open-LinkedFixture 'ReviewA' '場'
    foreach ($node in @(@(1,'first',@(237,33,125)),@(2,'second',@(18,164,116)))) {
        Locate-NoteOnCanvas 1 'IntegratedResearch' 1 2 $node[0] $node[1] $node[2]
        Open-LocationSource 'ReviewA' '相對頁'
    }
    Native-Menu 120 3 '前往　ReviewB　｜ ReviewedB' -CaptionPrefix
    $null = Wait-For 'reopened backup resolves the exact renamed note instead of the namesake' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq 'ReviewedB' -and
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('場內的精確內容')
    }
    Open-SortCollection 'IntegratedReview'
    Open-SortNamedView 1 'IntegratedOrder'
    Assert-RankedRows @('ReviewA','ReviewedB')
    Close-Notebook $process
    $process.Dispose()
    $process = $null

    $invalidRestore = Join-Path $root 'truncated-workspace-restore'
    $invalidData = Join-Path $invalidRestore 'paperroom-data'
    [void](New-Item -ItemType Directory -Path $invalidRestore)
    [void](New-Item -ItemType Directory -Path $invalidData)
    $invalidDb = Join-Path $invalidData 'notes.sqlite3'
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $invalidRestore 'Paperroom.exe')
    @'
from contextlib import closing
from pathlib import Path
import json, sqlite3, sys
origin, archive, restored, invalid = map(Path, sys.argv[1:])

def snapshot(path):
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
        assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",), path
        assert not db.execute("PRAGMA foreign_key_check").fetchall(), path
        queries = (
            "SELECT p.import_root,p.note_id,p.relative_path,n.title,n.body,t.token FROM vault_paths p "
            "JOIN notes n ON n.id=p.note_id JOIN note_link_identities t ON t.note_id=n.id ORDER BY p.relative_path",
            "SELECT c.id,c.title,c.source,b.import_root FROM saved_canvases c "
            "JOIN canvas_vault_bindings b ON b.canvas_id=c.id",
            "SELECT v.collection_id,v.name,v.rules_json,f.name,f.expression FROM collection_views v "
            "JOIN collection_formulas f ON f.collection_id=v.collection_id",
            "SELECT c.collection_id,c.note_id,c.status,p.key,p.value FROM collection_notes c "
            "JOIN note_properties p ON p.note_id=c.note_id ORDER BY c.note_id,p.key",
            "SELECT p.relative_path,o.relative_path,a.content FROM attachments a "
            "JOIN attachment_origins o ON o.attachment_id=a.id JOIN vault_paths p ON p.note_id=a.note_id",
            "SELECT collection_id,key,required,default_value FROM collection_field_rules ORDER BY collection_id,key",
        )
        return [db.execute(query).fetchall() for query in queries]

expected = snapshot(origin)
assert snapshot(archive) == snapshot(restored) == expected
notes, canvases, views, ranks, media, field_rules = expected
by_path = {row[2]: row for row in notes}
a, b, decoy = (by_path[path] for path in ("\u5834/ReviewA.md", "\u5834/ReviewB.md", "ReviewB.md"))
assert b[3] == "ReviewedB" and decoy[3] == "ReviewB"
assert f"paperroom:note:{b[1]}:{b[5]}|ReviewB" in a[4] and "./ReviewB.md#review" in a[4]
assert len(canvases) == 1 and canvases[0][1] == "IntegratedResearch" and canvases[0][3] == a[0]
assert [n["id"] for n in json.loads(canvases[0][2])["nodes"]] == ["first", "second"]
assert len(views) == 1 and views[0][1] == "IntegratedOrder" and views[0][4] == 'typed("\u5de5\u6642")'
assert json.loads(views[0][2]) == [False, [], [["\u5be9\u95b1\u9806\u4f4d", "formula-number", True]]]
assert {row[1]: row[4] for row in ranks} == {a[1]: "8", b[1]: "7"}
assert field_rules == [(views[0][0], "\u5de5\u6642", 1, "0")]
assert media == [(a[2], "\u7d20\u6750/\u5361.txt", b"asset one")]

# Drop a full page from the real backup while preserving its SQLite-format header.
raw = archive.read_bytes()
assert raw.startswith(b"SQLite format 3\0")
page_size = int.from_bytes(raw[16:18], "big")
if page_size == 1:
    page_size = 65536
assert 512 <= page_size <= 65536 and len(raw) % page_size == 0 and len(raw) > page_size * 2
with invalid.open("xb") as out:
    out.write(raw[:-page_size])
print("Full backup has the original stable note identity and attachment, exact Canvas binding and nodes, and saved formula and sort. Prepared a truncated copy.")
'@ | python - $reviewDb $jointBackup $fullDb $invalidDb
    if ($LASTEXITCODE -ne 0) { throw 'Workspace backup and restore did not preserve the integrated feature data.' }
    $invalidHash = (Get-FileHash -LiteralPath $invalidDb).Hash
    $watcher = [IO.FileSystemWatcher]::new($invalidData)
    $watcher.NotifyFilter = [IO.NotifyFilters]'FileName, DirectoryName, LastWrite, Size'
    $watchEvents = @('Created','Changed','Deleted','Renamed','Error')
    $watchPrefix = 'PaperroomRejectedBackup.' + [guid]::NewGuid().ToString('N')
    try {
        foreach ($eventName in $watchEvents) {
            $null = Register-ObjectEvent -InputObject $watcher -EventName $eventName -SourceIdentifier "$watchPrefix.$eventName"
        }
        $watcher.EnableRaisingEvents = $true
        $window = [IntPtr]::Zero
        $process = Start-NotebookProcess $invalidRestore
        $alert = Wait-For 'truncated backup startup error from the exact EXE' {
            $process.Refresh()
            if ($process.HasExited) { throw "Backup startup exited without its expected native error (code $($process.ExitCode))." }
            $handle = $process.MainWindowHandle
            if ($handle -ne [IntPtr]::Zero -and [PaperroomHighlightGui]::GetDlgItem($handle,103) -ne [IntPtr]::Zero) {
                throw 'The app opened an editor with the truncated backup.'
            }
            [PaperroomHighlightGui]::ForegroundAlert($handle,$process.Id,'紙間無法啟動')
        }
        if (![PaperroomHighlightGui]::DialogContains($alert,$process.Id,'database disk image is malformed')) {
            throw 'The startup error did not identify database corruption.'
        }
        [PaperroomHighlightGui]::TapKey($alert,$process.Id,0x0D)
        if (!$process.WaitForExit(10000) -or $process.ExitCode -ne 1) { throw 'The corrupt backup did not exit with the startup failure code.' }
        $process.Dispose()
        $process = $null
        Start-Sleep -Milliseconds 300
        $events = @(Get-Event | Where-Object { $_.SourceIdentifier.StartsWith($watchPrefix,[StringComparison]::Ordinal) })
        if ($events.Count -ne 0) {
            $details = $events | ForEach-Object { "$($_.SourceIdentifier.Split('.')[-1]): $($_.SourceEventArgs)" }
            throw "The failed startup triggered writes, sidecar activity or a watcher error: $($details -join ', ')"
        }
    } finally {
        $watcher.Dispose()
        foreach ($eventName in $watchEvents) {
            Unregister-Event -SourceIdentifier "$watchPrefix.$eventName" -ErrorAction SilentlyContinue
            Remove-Event -SourceIdentifier "$watchPrefix.$eventName" -ErrorAction SilentlyContinue
        }
    }
    if ((Get-FileHash -LiteralPath $invalidDb).Hash -cne $invalidHash -or
        @(Get-ChildItem -LiteralPath $invalidData -Force).Count -ne 1 -or
        (Get-FileHash -LiteralPath $jointBackup).Hash -cne $backupHash -or
        (Get-FileHash -LiteralPath $reviewDb).Hash -cne $originHash) {
        throw 'Restore testing modified a backup input or the original workspace, or left a sidecar.'
    }
    Write-Host 'PASS: native full-workspace backup and isolated restore retained the exact renamed identity, both Canvas occurrences and formula-based order after restart. The truncated backup showed a native corruption error, exited with code 1 and triggered no workspace writes.'
    $process = Start-Notebook $fullRestore
    $window = $process.MainWindowHandle
    Open-SortCollection 'IntegratedReview'
    Open-SortNamedView 1 'IntegratedOrder'
    $beforeRestoredAdmission = Get-VaultReviewStamp $fullDb
    Start-RuleRow 'RestoredDefault'
    Rule-Prompt '工時' '0' ''
    Rule-Error '欄位值無效' '必填'
    if ((Get-VaultReviewStamp $fullDb) -cne $beforeRestoredAdmission) {
        throw 'Rejecting a missing required value changed the restored collection.'
    }
    Rule-Prompt '工時' '' '0'
    $null = Wait-For 'restored rule admits a valid zero' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已新增集合資料列。'
    }
    Assert-RankedRows @('ReviewA','ReviewedB','RestoredDefault')
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    if ((Get-FileHash -LiteralPath $jointBackup).Hash -cne $backupHash -or
        (Get-FileHash -LiteralPath $reviewDb).Hash -cne $originHash) {
        throw 'An edit in the restored clone changed the source or backup.'
    }
    Write-Host 'PASS: integrated required fields rejected admission and clearing without writes, a retained correction changed live formula order, and backup preserved rule behavior alongside the same Vault and Canvas.'


    $locationHome = Join-Path $root 'canvas-locations-workspace'
    $vaultA = Join-Path $root 'LocationVaultA'
    $vaultB = Join-Path $root 'LocationVaultB'
    $locationFolderA = Join-Path $vaultA '資料'
    $locationFolderB = Join-Path $vaultB '資料'
    foreach ($dir in @($locationHome,$locationFolderA,$locationFolderB)) { [void][IO.Directory]::CreateDirectory($dir) }
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $locationHome 'Paperroom.exe')
    $encoding = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText((Join-Path $locationFolderA '鏡頭.md'),'場景原本的獨特引文',$encoding)
    [IO.File]::WriteAllText((Join-Path $locationFolderB '鏡頭.md'),'另一本 Vault 的同名正文',$encoding)
    $north = Join-Path $root 'CanvasNorth.canvas'
    $south = Join-Path $root 'CanvasSouth.canvas'
    $other = Join-Path $root 'CanvasOther.canvas'
    [IO.File]::WriteAllText($north,'{"nodes":[{"id":"west","type":"file","file":"資料/鏡頭.md","label":"同一張卡","x":-28000,"y":800,"width":120,"height":75,"color":"#ed217d"},{"id":"east","type":"file","file":"資料/鏡頭.md","label":"同一張卡","x":28000,"y":-800,"width":120,"height":75,"color":"#12a474"}],"edges":[{"id":"line","fromNode":"west","toNode":"east","custom":true}],"vendor":{"keep":true}}',$encoding)
    [IO.File]::WriteAllText($south,'{"nodes":[{"id":"only","type":"file","file":"資料/鏡頭.md","label":"同一張卡","x":37000,"y":800,"width":120,"height":75,"color":"#047bea"}]}',$encoding)
    [IO.File]::WriteAllText($other,'{"nodes":[{"id":"unrelated","type":"file","file":"資料/鏡頭.md","label":"同一張卡","x":1000,"y":800,"width":120,"height":75,"color":"#6f5092"}]}',$encoding)
    $locationDb = Join-Path $locationHome 'paperroom-data/notes.sqlite3'
    $process = Start-Notebook $locationHome
    $window = $process.MainWindowHandle
    Native-VaultFolder $vaultA '附件綁定 0 筆，未解析 0 筆'
    Native-VaultFolder $vaultB '附件綁定 0 筆，未解析 0 筆'
    foreach ($spec in @(@($north,1,'LocationVaultA'),@($south,1,'LocationVaultA'),@($other,2,'LocationVaultB'))) {
        Native-File 11 '匯入 JSON Canvas…' $spec[0] '中鍵拖曳平移'
        Native-Menu 120 4 '綁定匯入的 Vault…'
        Native-Menu 120 $spec[1] "匯入：$($spec[2])  (" -MenuOpen -CaptionPrefix
        $null = Wait-For 'explicit Canvas vault binding was saved' {
            [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將畫布連到這個匯入的 Vault。'
        }
    }
    $dialog = Open-WorkspaceSearch -Menu
    Type-SearchFilter $dialog 101 '場景原本的獨特引文'
    Search-Workspace $dialog '找到 1 筆筆記。' 1
    Finish-Revision $dialog 111
    $null = Wait-For 'selected the only original card by unique body' {
        [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭'
    }
    # The two notes share a title. Edit through the real picker (it omits the
    # current note), then verify each destination by its distinct saved body.
    Native-Menu 113 7 '筆記關聯…'
    Native-Menu 113 1 '新增關聯…' -MenuOpen
    Choose-CollectionRow '鏡頭  ('
    Collection-Prompt '新增關聯' '取景對照'
    function Open-TwinNamedRelation([int]$Direction, [string]$Label, [string]$Body) {
        Native-Menu 120 60001 '具名關聯的來源與目標…'
        Native-Menu 120 $Direction $Label -MenuOpen
        Native-Menu 120 1 '取景對照　鏡頭' -MenuOpen -CaptionPrefix
        $null = Wait-For "selected the exact named relation endpoint: $Body" {
            [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
            [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟具名關聯的筆記。' -and
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains($Body)
        }
    }
    Open-TwinNamedRelation 2 '此筆記指向的具名關聯…' '另一本 Vault 的同名正文'
    Open-TwinNamedRelation 1 '指向此筆記的具名關聯…' '場景原本的獨特引文'
    Locate-NoteOnCanvas 1 'CanvasNorth' 1 2 1 'west' @(237,33,125) -FromLinks
    Open-LocationSource
    Locate-NoteOnCanvas 1 'CanvasNorth' 1 2 2 'east' @(18,164,116)
    Open-LocationSource
    Locate-NoteOnCanvas 2 'CanvasSouth' 2 1 1 'only' @(4,123,234)
    Open-LocationSource
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $locationHome
    $window = $process.MainWindowHandle
    $dialog = Open-WorkspaceSearch -Menu
    Type-SearchFilter $dialog 101 '場景原本的獨特引文'
    Search-Workspace $dialog '找到 1 筆筆記。' 1
    Finish-Revision $dialog 111
    $null = Wait-For 'reopened the original note' { [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' }
    Open-TwinNamedRelation 2 '此筆記指向的具名關聯…' '另一本 Vault 的同名正文'
    Open-TwinNamedRelation 1 '指向此筆記的具名關聯…' '場景原本的獨特引文'
    Write-Host 'PASS: native Links button followed both named-relation directions between same-titled notes by distinct body after restart.'
    Locate-NoteOnCanvas 1 'CanvasNorth' 1 2 2 'east' @(18,164,116) -FromLinks
    Native-Menu 120 6 '解除 Vault 綁定'
    Native-Menu 120 13 '開啟已選節點的來源筆記'
    $refusal = Wait-For 'selected node cannot open an unbound file' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
    }
    if (![PaperroomHighlightGui]::DialogContains($refusal,$process.Id,'請先綁定含有這個精確路徑的 Vault')) {
        throw 'The unbound selected occurrence did not explain why it cannot resolve.'
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null = Wait-For 'dismissed the native stale binding alert' { ![PaperroomHighlightGui]::IsWindowVisible($refusal) }
    Native-Menu 120 4 '綁定匯入的 Vault…'
    Native-Menu 120 2 '匯入：LocationVaultB  (' -MenuOpen -CaptionPrefix
    Native-Menu 120 13 '開啟已選節點的來源筆記'
    $refusal = Wait-For 'rebinding cannot redirect this selected occurrence to a namesake' {
        [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
    }
    if (![PaperroomHighlightGui]::DialogContains($refusal,$process.Id,'此節點已不再指向原筆記')) {
        throw 'Rebinding unexpectedly redirected the current visit.'
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
    $null = Wait-For 'dismissed the native redirected-target alert' { ![PaperroomHighlightGui]::IsWindowVisible($refusal) }
    Native-Menu 120 6 '解除 Vault 綁定'
    Return-FromCanvasLocation
    Locate-NoteOnCanvas 1 'CanvasSouth' 2 1 1 'only' @(4,123,234)
    Native-Menu 120 6 '解除 Vault 綁定'
    Return-FromCanvasLocation
    Native-Menu 113 27 '查看此筆記的 Canvas 位置…'
    $null = Wait-For 'unbound and same-name other Vault occurrences are excluded' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這篇筆記未出現在已綁定的 Canvas。'
    }
    # Work with the same two imported notes, not a second set with similar names.
    $nativeBodies = @('場景原本的獨特引文','另一本 Vault 的同名正文')
    $pickRows = @'
import sqlite3, sys
from pathlib import Path
with sqlite3.connect(Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as db:
    live=db.execute("SELECT title,body FROM notes WHERE trash_root IS NULL ORDER BY id").fetchall()
    for body in sys.argv[2:]:
        hits=[i for i,row in enumerate(live,1) if row==("鏡頭",body)]
        assert len(hits)==1
        print(hits[0])
'@ | python - $locationDb $nativeBodies[0] $nativeBodies[1]
    if ($LASTEXITCODE -ne 0 -or @($pickRows).Count -ne 2) { throw 'Could not independently identify the two real note-picker rows.' }
    function Assert-TwinBody([string]$Body) {
        $null=Wait-For "native board opened the exact note: $Body" {
            [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '鏡頭' -and
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains($Body) -and
            [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
        }
    }
    function Pick-NativePlacement([string]$Board, [int]$ExpectedCount) {
        Native-Menu 120 60003 '查看此筆記的原生白板位置…'
        $options=@(Collection-MenuLabels)
        if ($options.Count -ne $ExpectedCount) { throw "Expected $ExpectedCount placements; got $($options -join ', ')." }
        Choose-CollectionRow $Board
        $null=Wait-For "native board focus: $Board" {
            [PaperroomHighlightGui]::Read((Control $window 115)).Contains($Board) -and
            [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已定位白板上的筆記卡片。'
        }
    }
    function Search-NativeTwin([string]$Body) {
        $d=Open-WorkspaceSearch -Menu
        Type-SearchFilter $d 101 $Body
        Search-Workspace $d '找到 1 筆筆記。' 1
        Finish-Revision $d 111
        Assert-TwinBody $Body
    }
    $null = Wait-For 'ready to switch from a note to the native board' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
    Click-WorkspaceButton 119
    $null = Wait-For 'native board controls ready at their board positions' {
        if ([PaperroomHighlightGui]::Read((Control $window 119)) -cne '返回筆記' -or
            [PaperroomHighlightGui]::Read((Control $window 120)) -cne '白板操作') { return $false }
        try {
            $area = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
            $button = [PaperroomHighlightGui]::ButtonPoint($window,$process.Id,(Control $window 120))
            return $button.Y -lt $area.Top
        } catch { return $false }
    }
    Native-Menu 120 3 '新增白板…'
    Collection-Prompt '新增白板' 'NativeNorth'
    $null = Wait-For 'new board NativeNorth is active' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains('NativeNorth') -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
    }
    Native-Menu 120 1 '放入筆記…'
    Native-Menu 120 ([uint32]$pickRows[0]) '鏡頭' -MenuOpen
    Native-Menu 120 4 '新增分區…'
    Collection-Prompt '新增分區' '遠端卡片'
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true)
    Native-Menu 120 4 '設定分區…' -MenuOpen
    Native-Menu 120 2 '遠端卡片' -MenuOpen
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false,$false,$true)
    Native-Menu 120 12 '移動選取的物件…'
    Collection-Prompt '移動選取的物件' '4200,3600'
    $null = Wait-For 'moved the card outside the viewport' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已移動 1 個白板物件。'
    }
    Native-Menu 120 13 '收合或展開分區…'
    Native-Menu 120 1 '收合「遠端卡片」' -MenuOpen
    Native-Menu 120 3 '新增白板…'
    Collection-Prompt '新增白板' 'NativeSouth'
    $null = Wait-For 'new board NativeSouth is active' {
        [PaperroomHighlightGui]::Read((Control $window 115)).Contains('NativeSouth') -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '白板操作'
    }
    foreach ($index in @(1,0)) {
        Native-Menu 120 1 '放入筆記…'
        Native-Menu 120 ([uint32]$pickRows[$index]) '鏡頭' -MenuOpen
    }
    Click-WorkspaceButton 119
    $null = Wait-For 'note controls ready after leaving the native board' {
        [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板' -and
        [PaperroomHighlightGui]::Read((Control $window 120)) -ceq '連結'
    }
    Search-NativeTwin $nativeBodies[0]
    Pick-NativePlacement 'NativeSouth' 2
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,1,$false)
    Assert-TwinBody $nativeBodies[0]
    Open-TwinNamedRelation 2 '此筆記指向的具名關聯…' $nativeBodies[1]
    Pick-NativePlacement 'NativeSouth' 1
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false)
    Assert-TwinBody $nativeBodies[1]
    Open-TwinNamedRelation 1 '指向此筆記的具名關聯…' $nativeBodies[0]
    $nativeBackup=Join-Path $root 'native-locations.sqlite3'
    Native-File 4 '備份工作區…' $nativeBackup '工作區備份已完成'
    $nativeBackupHash=(Get-FileHash -LiteralPath $nativeBackup).Hash
    Pick-NativePlacement 'NativeNorth' 2
    # The far card began inside a collapsed group. Opening it with a physical
    # click at the read-only viewport focus verifies both scrolling and reveal.
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false,$true,$false)
    Assert-TwinBody $nativeBodies[0]
    Pick-NativePlacement 'NativeNorth' 2
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$true,$true,$false)
    Native-Menu 120 1 '移出白板' -MenuOpen
    Close-Notebook $process
    $process.Dispose()
    $process=Start-Notebook $locationHome
    $window=$process.MainWindowHandle
    Search-NativeTwin $nativeBodies[0]
    Pick-NativePlacement 'NativeSouth' 1
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,1,$false)
    Assert-TwinBody $nativeBodies[0]
    Close-Notebook $process
    $process.Dispose()
    $clone=Join-Path $root 'native-location-restore'
    $cloneData=Join-Path $clone 'paperroom-data'
    [void][IO.Directory]::CreateDirectory($cloneData)
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $clone 'Paperroom.exe')
    $restoreCopy = Join-Path $cloneData 'notes.sqlite3'
    Copy-Item -LiteralPath $nativeBackup -Destination $restoreCopy
    if ((Get-FileHash -LiteralPath $restoreCopy).Hash -cne $nativeBackupHash) { throw 'Restored workspace copy differs from the backup before startup.' }
    $process=Start-Notebook $clone
    $window=$process.MainWindowHandle
    Search-NativeTwin $nativeBodies[0]
    Pick-NativePlacement 'NativeNorth' 2
    [PaperroomHighlightGui]::BoardCardClick($window,$process.Id,0,$false,$true,$false)
    Assert-TwinBody $nativeBodies[0]
    if ((Get-FileHash -LiteralPath $nativeBackup).Hash -cne $nativeBackupHash) { throw 'Opening the isolated restore changed its backup input.' }
    Write-Host 'PASS: native board placements distinguished same-title note bodies, crossed a named relation, revealed a collapsed off-screen card, survived restart after a placement removal, and restored both placements from an isolated workspace backup.'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    # Use two live windows on the isolated restored copy. Compare SQLite
    # read-only, but move the card exclusively through native pointer input.
    function Read-SouthCoordinates {
        $coords = @'
import json, sqlite3, sys
from contextlib import closing
from pathlib import Path
with closing(sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)) as db:
    rows = db.execute("SELECT n.body,c.x,c.y FROM board_cards c JOIN notes n ON n.id=c.note_id "
                      "JOIN boards b ON b.id=c.board_id WHERE b.title='NativeSouth' ORDER BY n.id").fetchall()
assert len(rows) == 2
target, = [row for row in rows if row[0] == sys.argv[2]]
other, = [row for row in rows if row[0] != sys.argv[2]]
print(json.dumps(dict(x=target[1], y=target[2], other=other[1:])))
'@ | python - $restoreCopy $nativeBodies[1]
        if ($LASTEXITCODE -ne 0 -or !$coords) { throw 'Could not read the two isolated board placements.' }
        return $coords | ConvertFrom-Json
    }
    function Activate-TestBoard {
        if (![PaperroomHighlightGui]::WorkspaceInForeground($window,$process.Id)) {
            [void][PaperroomHighlightGui]::SetForegroundWindow($window)
            if (![PaperroomHighlightGui]::WorkspaceInForeground($window,$process.Id)) {
                $activator = New-Object -ComObject WScript.Shell
                [void]$activator.AppActivate($process.Id)
            }
        }
        $null = Wait-For 'selected isolated board window in foreground' {
            [PaperroomHighlightGui]::WorkspaceInForeground($window,$process.Id)
        }
    }
    function Drag-NativeCard($Position, [int]$Dx, [int]$Dy) {
        $area = [PaperroomHighlightGui]::CanvasArea($window,$process.Id)
        $dpi = [PaperroomHighlightGui]::GetDpiForWindow($window)
        $x = $area.Left + [int][Math]::Round(($Position.x + 30) * $dpi / 96)
        $y = $area.Top + [int][Math]::Round(($Position.y + 30) * $dpi / 96)
        Drag-Canvas $x $y ($x+$Dx) ($y+$Dy)
    }
    $second = $null
    try {
        $process = Start-Notebook $clone
        $window = $process.MainWindowHandle
        Search-NativeTwin $nativeBodies[1]
        Pick-NativePlacement 'NativeSouth' 1
        $first = $process
        $firstWindow = $window
        $startCoords = Read-SouthCoordinates
        if ($startCoords.x -ne 16 -or $startCoords.y -ne 16) { throw 'Unexpected clean first-slot geometry.' }

        $second = Start-Notebook $clone
        $process = $second
        $window = $process.MainWindowHandle
        Search-NativeTwin $nativeBodies[1]
        Pick-NativePlacement 'NativeSouth' 1
        Activate-TestBoard
        Drag-NativeCard $startCoords 60 70
        $secondCoords = Read-SouthCoordinates
        if ($secondCoords.x -le $startCoords.x -or $secondCoords.y -le $startCoords.y) {
            throw 'The second window did not save its physical drag.'
        }

        $process = $first
        $window = $firstWindow
        Activate-TestBoard
        Drag-NativeCard $startCoords 0 0
        $afterClick = Read-SouthCoordinates
        if ($afterClick.x -ne $secondCoords.x -or $afterClick.y -ne $secondCoords.y) {
            throw 'The stationary click overwrote newer coordinates.'
        }
        Start-Sleep -Milliseconds 700 # Keep these two gestures separate from a system double-click.
        Drag-NativeCard $startCoords 35 40
        $refusal = Wait-For 'native alert for a stale board gesture' {
            [PaperroomHighlightGui]::ForegroundAlert($window,$process.Id,'紙間')
        }
        if (![PaperroomHighlightGui]::DialogContains($refusal,$process.Id,'卡片位置或狀態已變更')) {
            throw 'The first window did not identify a conflicting card move.'
        }
        [PaperroomHighlightGui]::TapKey($window,$process.Id,0x0D)
        $null = Wait-For 'stale board alert closed after the board refresh' {
            ![PaperroomHighlightGui]::IsWindowVisible($refusal)
        }
        $afterRefusal = Read-SouthCoordinates
        if ($afterRefusal.x -ne $secondCoords.x -or $afterRefusal.y -ne $secondCoords.y) {
            throw 'A stale physical drag overwrote newer coordinates.'
        }
        Drag-NativeCard $afterRefusal 35 40
        $fresh = Read-SouthCoordinates
        if ($fresh.x -le $afterRefusal.x -or $fresh.y -le $afterRefusal.y -or
            (($fresh.other -join ',') -cne ($startCoords.other -join ','))) {
            throw 'The refreshed drag did not persist or affected the neighbor card.'
        }
        Close-Notebook $second
        $second.Dispose()
        $second = $null
        Close-Notebook $process
        $process.Dispose()
        $process = Start-Notebook $clone
        $window = $process.MainWindowHandle
        Search-NativeTwin $nativeBodies[1]
        Pick-NativePlacement 'NativeSouth' 1
        $reopened = Read-SouthCoordinates
        if ($reopened.x -ne $fresh.x -or $reopened.y -ne $fresh.y) {
            throw 'The latest native drag did not survive reopen.'
        }
        Close-Notebook $process
        $process.Dispose()
        $process = $null
        Write-Host 'PASS: two native board windows kept the newer card position on a stationary click, rejected a stale drag, and persisted a refreshed move across restart.'
    } finally {
        if ($second) {
            if (!$second.HasExited) { $second.Kill(); [void]$second.WaitForExit(5000) }
            if ($process -eq $second) { $process = $first }
            $second.Dispose()
        }
    }
    @'
from contextlib import closing
from pathlib import Path
import sqlite3,sys
db_path,north,south,other=map(Path,sys.argv[1:])
with closing(sqlite3.connect(db_path.as_uri()+"?mode=ro",uri=True)) as db:
    assert db.execute("PRAGMA integrity_check").fetchone()==("ok",)
    canvas=db.execute("SELECT title, source FROM saved_canvases ORDER BY id").fetchall()
    assert len(canvas)==3
    for (title,data),path in zip(canvas,(north,south,other)):
        assert title==path.stem and data.encode("utf-8")==path.read_bytes()
    assert db.execute("SELECT count(*) FROM canvas_cameras").fetchone()==(0,)
    binding=db.execute("SELECT c.title,n.title FROM canvas_vault_bindings b "
                       "JOIN saved_canvases c ON c.id=b.canvas_id JOIN notes n ON n.id=b.import_root").fetchall()
    assert binding==[("CanvasOther","\u532f\u5165\uff1aLocationVaultB")]
    assert db.execute("SELECT count(*) FROM vault_paths").fetchone()==(2,)
print("Verified all original Canvas bytes, no saved camera edits, and only the expected second-vault binding after native navigation.")
'@ | python - $locationDb $north $south $other
    if ($LASTEXITCODE -ne 0) { throw 'The exact Canvas and workspace round-trip verification failed.' }
    Write-Host 'PASS: native note-to-Canvas locations find repeated nodes in separate canvases, center each painted occurrence, reopen the exact note and persist across restart, refuse an unbound selected node, and keep Canvas content and camera settings unchanged.'

    $ruleHome = Join-Path $root 'collection-rules'
    $ruleRestore = Join-Path $root 'restored-collection-rules'
    $ruleDb = Join-Path $ruleHome 'paperroom-data\notes.sqlite3'
    $ruleRestoreDb = Join-Path $ruleRestore 'paperroom-data\notes.sqlite3'
    $ruleBackup = Join-Path $root 'rule-backup.sqlite3'
    foreach ($folder in $ruleHome,$ruleRestore) {
        [void][IO.Directory]::CreateDirectory((Join-Path $folder 'paperroom-data'))
        Copy-Item -LiteralPath $Exe -Destination (Join-Path $folder 'Paperroom.exe')
    }
    $process = Start-Notebook $ruleHome
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    # A synthetic existing collection, a member without properties and an outside note
    # with an explicit true. All rule and row changes below go through Paperroom.exe.
    @'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute('PRAGMA foreign_keys=ON')
    cid = db.execute("INSERT INTO collections(name) VALUES('Rule Fixture')").lastrowid
    legacy = db.execute("INSERT INTO notes(title,body) VALUES('Legacy','preserved body')").lastrowid
    ext = db.execute("INSERT INTO notes(title,body) VALUES('External','keep this prior body')").lastrowid
    db.execute("INSERT INTO collection_notes VALUES(?,?,?)",(cid,legacy,'\u5f85\u8655\u7406'))
    db.executemany("INSERT INTO property_definitions VALUES(?,?)",[('A-check','checkbox'),('B-hours','number')])
    db.execute("INSERT INTO note_properties VALUES(?,'A-check','true')",(ext,))
'@ | python - $ruleDb
    if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the synthetic pre-rule workspace.' }
    $process = Start-Notebook $ruleHome
    $window = $process.MainWindowHandle
    Open-RuleBoard
    Set-Rule 'A-check' 'false'
    Set-Rule 'B-hours' '0'
    Check-RuleRows $ruleDb before

    Start-RuleRow 'Cancelled'
    Rule-Prompt 'A-check' 'false' 'true'
    Rule-Prompt 'B-hours' '0' '0' -Cancel
    Check-RuleRows $ruleDb before

    Start-RuleRow 'New row'
    Rule-Prompt 'A-check' 'false' 'true'
    Rule-Prompt 'B-hours' '0' ''
    Rule-Error '欄位值無效' '必填'
    Check-RuleRows $ruleDb reject-new
    Rule-Prompt 'B-hours' '' '7'
    $null = Wait-For 'new row saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已新增集合資料列。' }
    Check-RuleRows $ruleDb created

    Native-Menu 120 18 '編輯資料列的型別欄位…'
    Choose-CollectionRow '待處理　New row  ('
    Choose-CollectionRow 'B-hours　'
    Rule-Prompt 'B-hours' '7' ''
    Rule-Error '欄位未儲存' '必填'
    Check-RuleRows $ruleDb created
    Rule-Prompt 'B-hours' '' 'bad-number'
    Rule-Error '欄位未儲存' 'bad-number'
    Check-RuleRows $ruleDb created
    Rule-Prompt 'B-hours' 'bad-number' '9'
    $null = Wait-For 'corrected edit saved' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已更新資料列欄位。' }

    Native-Menu 120 1 '加入筆記…'
    Choose-CollectionRow 'External  ('
    Rule-Prompt '加入集合' '待處理' '待處理'
    Rule-Prompt 'B-hours' '0' '0'
    $null = Wait-For 'existing note joined the same collection' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已將筆記加入集合。'
    }
    Check-RuleRows $ruleDb edited
    Close-Notebook $process
    $process.Dispose()
    $process = Start-Notebook $ruleHome
    $window = $process.MainWindowHandle
    Open-RuleBoard
    Check-RuleRows $ruleDb edited
    Native-File 4 '備份工作區…' $ruleBackup '工作區備份已完成'
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    $sourceStamp = (Get-FileHash -LiteralPath $ruleDb).Hash
    $backupStamp = (Get-FileHash -LiteralPath $ruleBackup).Hash
    Copy-Item -LiteralPath $ruleBackup -Destination $ruleRestoreDb
    $process = Start-Notebook $ruleRestore
    $window = $process.MainWindowHandle
    Open-RuleBoard
    Check-RuleRows $ruleRestoreDb restored
    Start-RuleRow 'Restored row'
    Rule-Prompt 'A-check' 'false' 'false'
    Rule-Prompt 'B-hours' '0' ''
    Rule-Error '欄位值無效' '必填'
    Rule-Prompt 'B-hours' '' '0'
    $null = Wait-For 'restored board applied rules' { [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已新增集合資料列。' }
    Close-Notebook $process
    $process.Dispose()
    $process = $null
    Check-RuleRows $ruleRestoreDb new-restored
    if ((Get-FileHash -LiteralPath $ruleDb).Hash -cne $sourceStamp -or
        (Get-FileHash -LiteralPath $ruleBackup).Hash -cne $backupStamp) { throw 'Isolated restore changed source or backup.' }
    Write-Host "PASS: collection rules, actual native creation and edits, rejected input and cancel without writes, legacy properties, reopen and isolated backup restore. EXE SHA-256 $ExeSha256."
    . (Join-Path $PSScriptRoot 'windows-imported-legacy-link.ps1')
    . (Join-Path $PSScriptRoot 'windows-notion-csv-workflow.ps1')
    . (Join-Path $PSScriptRoot 'windows-notion-page-links.ps1')
    . (Join-Path $PSScriptRoot 'windows-canvas-local-assets.ps1')
    . (Join-Path $PSScriptRoot 'windows-joint-notion-canvas.ps1')
    . (Join-Path $PSScriptRoot 'windows-canvas-vault-root.ps1')
    . (Join-Path $PSScriptRoot 'windows-note-history-state.ps1')
    . (Join-Path $PSScriptRoot 'windows-collection-due-filters.ps1')
    . (Join-Path $PSScriptRoot 'windows-collection-display-columns.ps1')
    . (Join-Path $PSScriptRoot 'windows-collection-group-folds.ps1')
    . (Join-Path $PSScriptRoot 'windows-joint-collection-history.ps1')
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
