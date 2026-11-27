# Exercise only a synthetic fixture in an ephemeral runner, using the official desktop UI.
[CmdletBinding(DefaultParameterSetName='Move')]
param(
    [Parameter(Mandatory)][string]$Vault,
    [Parameter(Mandatory,ParameterSetName='Move')][string]$From,
    [Parameter(Mandatory,ParameterSetName='Move')][string]$To,
    [Parameter(Mandatory,ParameterSetName='Heading')][switch]$HeadingProbe,
    [switch]$Initialize
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
    throw 'Requires an ephemeral GitHub-hosted Windows runner.'
}
$vault = [IO.Path]::GetFullPath($Vault)
$tempTarget = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
$vaultName = Split-Path $vault -Leaf
if (!$vault.StartsWith($tempTarget,[StringComparison]::OrdinalIgnoreCase)) { throw 'Requires a synthetic Vault under this runner temporary directory.' }
if ($HeadingProbe) {
    if (!$Initialize -or $vaultName -cne 'PaperroomHeadingProbe' -or
        !(Test-Path -LiteralPath $vault -PathType Container) -or @(Get-ChildItem -LiteralPath $vault -Force).Count) {
        throw 'Heading mode requires -Initialize and an empty PaperroomHeadingProbe folder.'
    }
    for ($dir=Get-Item -LiteralPath $vault; $dir -and $dir.FullName.StartsWith($tempTarget,[StringComparison]::OrdinalIgnoreCase); $dir=$dir.Parent) {
        if ($dir.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Synthetic Vault path must not include a reparse point.' }
    }
} else {
    if ($vaultName -cne 'VaultScopeA' -or
        $From -notmatch '^(Active|Moved)/Flip\.md$' -or $To -notmatch '^(Active|Moved)/Flip\.md$' -or $From -ceq $To) {
        throw 'Only the isolated, fixed fixture may be moved.'
    }
    $fromFile = Join-Path $vault $From
    $toFile = Join-Path $vault $To
    if (!(Test-Path -LiteralPath $fromFile -PathType Leaf) -or (Test-Path -LiteralPath $toFile) -or
        !(Test-Path -LiteralPath (Split-Path $toFile -Parent) -PathType Container)) { throw 'Unexpected source or target fixture.' }
    $before = (Get-FileHash -LiteralPath $fromFile -Algorithm SHA256).Hash
}
$run = Split-Path $vault -Parent
$expected = '218c7d30acc065355ce94719dd996d5e2a2347cde3811ea5dc29380583f02b58'
$exe = Join-Path $env:LOCALAPPDATA 'Programs/Obsidian/Obsidian.exe'
if ($Initialize) {
    if ((Get-Process Obsidian -ErrorAction SilentlyContinue) -or (Test-Path "$env:APPDATA/obsidian") -or (Test-Path $exe)) {
        throw 'A running application or pre-existing Obsidian profile was found.'
    }
    $installer = Join-Path $run 'Obsidian-1.15.1.exe'
    Invoke-WebRequest 'https://github.com/obsidianmd/obsidian-releases/releases/download/v1.15.1/Obsidian-1.15.1.exe' -OutFile $installer -TimeoutSec 80
    if ((Get-FileHash $installer).Hash -ine $expected) { throw 'Installer digest mismatch.' }
    $signature = Get-AuthenticodeSignature $installer
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.GetNameInfo('SimpleName',$false) -ne 'Dynalist Inc') {
        throw 'Installer signature validation failed.'
    }
    $setup = Start-Process $installer -ArgumentList '/S' -PassThru
    if (!$setup.WaitForExit(120000) -or $setup.ExitCode -ne 0) { throw 'Per-user installer failed or timed out.' }
}
if (!(Test-Path $exe -PathType Leaf)) { throw 'Expected per-user installation was not found.' }
$info = (Get-Item $exe).VersionInfo
"Installed PE product version: $($info.ProductVersion). File version: $($info.FileVersion)."
$versionMatch='^1\.15\.1(?:\.0)?(?:$|[ +\-])'
if ($info.ProductVersion -notmatch $versionMatch -and $info.FileVersion -notmatch $versionMatch) { throw 'Installed desktop version mismatch.' }
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$root = [Windows.Automation.AutomationElement]::RootElement
$tree = [Windows.Automation.TreeScope]::Descendants
$truth = [Windows.Automation.Condition]::TrueCondition
function DesktopNodes {
    $ids = @(Get-Process Obsidian -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | ForEach-Object Id)
    foreach ($w in $root.FindAll([Windows.Automation.TreeScope]::Children,$truth)) {
        if ($w.Current.ProcessId -in $ids) {
            $w
            foreach ($child in $w.FindAll($tree,$truth)) { if (!$child.Current.IsOffscreen) { $child } }
        }
    }
}
function WaitFor($stage,[scriptblock]$check) {
    $nodes=@()
    $end=[datetime]::UtcNow.AddSeconds(35)
    do {
        try { $nodes=@(DesktopNodes) } catch {
            Start-Sleep -Milliseconds 300
            continue
        }
        try { $result = & $check $nodes }
        catch [Windows.Automation.ElementNotAvailableException] { Start-Sleep -Milliseconds 250; continue }
        if ($result) { Write-Host "Passed $stage"; return $result }
        Start-Sleep -Milliseconds 500
    } while ([datetime]::UtcNow -lt $end)
    @($nodes | Where-Object { $_.Current.Name -and $_.Current.Name.Length -le 150 }) |
        Select-Object -First 50 | ForEach-Object { Write-Host ("Visible fixture control: " + [string]$_.Current.ControlType + " " + $_.Current.Name) }
    throw "Blocked at $stage. No matching visible UI in the verified Obsidian process."
}
function Exact($nodes,$name,$kind) {
    @($nodes | Where-Object { $_.Current.Name -ceq $name -and (!$kind -or $_.Current.ControlType.ProgrammaticName -eq "ControlType.$kind") })
}
function Button($node) {
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ExactPicker {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X,Y; }
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window, uint flag);
    public static int[] Bounds(IntPtr window) {
        if (GetForegroundWindow()!=window || !GetWindowRect(window, out Rect r) ||
            r.Right-r.Left < 500 || r.Bottom-r.Top < 340)
            throw new InvalidOperationException("The fixture window is not usable.");
        return new int[]{r.Left,r.Top,r.Right-r.Left,r.Bottom-r.Top};
    }
    public static void ClickFixture(IntPtr window, int x, int y, bool twice) {
        Point p=new Point{X=x,Y=y};
        if (GetForegroundWindow()!=window || GetAncestor(WindowFromPoint(p),2)!=window ||
            !SetCursorPos(x,y) || GetAncestor(WindowFromPoint(p),2)!=window ||
            GetForegroundWindow()!=window)
            throw new InvalidOperationException("The fixture point is not exposed.");
        mouse_event(2,0,0,0,UIntPtr.Zero);
        mouse_event(4,0,0,0,UIntPtr.Zero);
        if (twice) {
            System.Threading.Thread.Sleep(90);
            if (GetAncestor(WindowFromPoint(p),2)!=window || GetForegroundWindow()!=window)
                throw new InvalidOperationException("The fixture point is no longer exposed.");
            mouse_event(2,0,0,0,UIntPtr.Zero);
            mouse_event(4,0,0,0,UIntPtr.Zero);
        }
    }
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint x, uint y, uint wheel, UIntPtr extra);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int which);
    static void CheckFixturePoint(IntPtr window, int x, int y) {
        if (GetForegroundWindow()!=window || GetAncestor(WindowFromPoint(new Point{X=x,Y=y}),2)!=window)
            throw new InvalidOperationException("Input point was not on the identified Vault window.");
    }
    public static void RightClick(IntPtr window,int x,int y) {
        CheckFixturePoint(window,x,y);
        if (!SetCursorPos(x,y)) throw new InvalidOperationException("Unable to point to the Canvas card.");
        CheckFixturePoint(window,x,y);
        mouse_event(8,0,0,0,UIntPtr.Zero);
        mouse_event(16,0,0,0,UIntPtr.Zero);
    }
    public static void DragFile(IntPtr window, int x0, int y0, int x1, int y1) {
        CheckFixturePoint(window,x0,y0);
        CheckFixturePoint(window,x1,y1);
        if (!SetCursorPos(x0,y0)) throw new InvalidOperationException("Unable to point to the file.");
        CheckFixturePoint(window,x0,y0);
        mouse_event(2,0,0,0,UIntPtr.Zero);
        try {
            System.Threading.Thread.Sleep(150);
            for (int i=1; i<=22; i++) {
                int x=x0+(x1-x0)*i/22, y=y0+(y1-y0)*i/22;
                CheckFixturePoint(window,x,y);
                uint ax=(uint)Math.Round((x-GetSystemMetrics(76))*65535.0/(GetSystemMetrics(78)-1));
                uint ay=(uint)Math.Round((y-GetSystemMetrics(77))*65535.0/(GetSystemMetrics(79)-1));
                mouse_event(0xC001,ax,ay,0,UIntPtr.Zero);
                System.Threading.Thread.Sleep(24);
            }
            System.Threading.Thread.Sleep(190);
        } finally {
            mouse_event(4,0,0,0,UIntPtr.Zero);
        }
    }
    public static void ClickDefault(IntPtr dialog,IntPtr button) {
        if (GetForegroundWindow()!=dialog || GetParent(button)!=dialog ||
            !IsWindowVisible(button) || !IsWindowEnabled(button) || !GetWindowRect(button,out Rect r))
            throw new InvalidOperationException("The identified folder confirmation is not usable.");
        Point p=new Point{ X=(r.Left+r.Right)/2,Y=(r.Top+r.Bottom)/2 };
        if (r.Right<=r.Left || r.Bottom<=r.Top || WindowFromPoint(p)!=button || !SetCursorPos(p.X,p.Y) ||
            GetForegroundWindow()!=dialog || WindowFromPoint(p)!=button)
            throw new InvalidOperationException("The folder confirmation point is no longer exposed.");
        mouse_event(2,0,0,0,UIntPtr.Zero);
        mouse_event(4,0,0,0,UIntPtr.Zero);
    }
}
'@

if ($Initialize) {
    Start-Process $exe
$open = WaitFor 'first-run Open folder as vault' {
    param($nodes)
    if (@(Exact $nodes 'Open folder as vault').Count -eq 0) { return }
    $buttons=@(Exact $nodes 'Open' 'Button')
    if ($buttons.Count -eq 1) { $buttons[0] }
}
Button $open
$picker = WaitFor 'native folder picker' {
    param($nodes)
    $found=@($nodes | Where-Object { $_.Current.ClassName -eq '#32770' })
    if ($found.Count -eq 1) { $found[0] }
}
$pickerNodes=@($picker.FindAll($tree,$truth) | Where-Object { !$_.Current.IsOffscreen })
$edits=@($pickerNodes | Where-Object {
    $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and $_.Current.IsEnabled
})
$edit=@($edits | Where-Object { $_.Current.Name.TrimEnd(':') -in 'Folder','Folder name','File name' })
if ($edit.Count -eq 0 -and $edits.Count -eq 1) { $edit=$edits }
if ($edit.Count -ne 1) {
    $edits | ForEach-Object { Write-Host "Picker edit class $($_.Current.ClassName), AutomationId $($_.Current.AutomationId), Name $($_.Current.Name)" }
    throw 'Unable to identify exactly one folder path edit.'
}

$handle=[IntPtr]$picker.Current.NativeWindowHandle
$edit[0].SetFocus()
if ([ExactPicker]::GetForegroundWindow() -ne $handle) { throw 'The synthetic picker is not foreground.' }
# Navigate to the fixture's parent with the address bar, then select the real folder item.
Set-Clipboard -Value $run
[Windows.Forms.SendKeys]::SendWait('%d')
[Windows.Forms.SendKeys]::SendWait('^v')
[Windows.Forms.SendKeys]::SendWait('{ENTER}')
$item=$null
$deadline=[datetime]::UtcNow.AddSeconds(12)
do {
    if ([ExactPicker]::GetForegroundWindow() -ne $handle) { throw 'Expected the same native picker after address entry.' }
    try {
        $fresh = [Windows.Automation.AutomationElement]::FromHandle($handle)
        $items=@($fresh.FindAll($tree,$truth) | Where-Object {
            !$_.Current.IsOffscreen -and $_.Current.Name -ceq $vaultName -and $_.Current.ControlType -ne [Windows.Automation.ControlType]::Edit
        })
    } catch {
        Start-Sleep -Milliseconds 250
        continue
    }
    foreach ($element in $items) {
        $selection=$null
        if ($element.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern,[ref]$selection)) {
            $item=$selection
            break
        }
    }
    if (!$item) { Start-Sleep -Milliseconds 250 }
} while (!$item -and [datetime]::UtcNow -lt $deadline)
if (!$item) { throw 'The exact test folder was not exposed as a selectable filesystem item.' }
$selected=$false
for ($try=0;$try -lt 16;$try++) {
    if ([ExactPicker]::GetForegroundWindow() -ne $handle) {
        [void][ExactPicker]::SetForegroundWindow($handle)
    }
    if ([ExactPicker]::GetForegroundWindow() -eq $handle) {
        if (!$item.Current.IsSelected) { $item.Select() }
        Start-Sleep -Milliseconds 200
        if ($item.Current.IsSelected -and [ExactPicker]::GetForegroundWindow() -eq $handle) {
            $selected=$true
            break
        }
    }
    Start-Sleep -Milliseconds 250
}
if (!$selected) { throw 'The exact synthetic folder never stayed selected in the native picker.' }
"Selected the exact synthetic folder item through the native filesystem chooser."
$confirm=[ExactPicker]::GetDlgItem($handle,1)
if ($confirm -eq [IntPtr]::Zero) { throw 'The selected folder has no native confirm button.' }
[ExactPicker]::ClickDefault($handle,$confirm)
$null=WaitFor 'synthetic vault and running app version' {
    param($nodes)
    @($nodes | Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::Window -and
        $_.Current.Name.Contains($vaultName) -and $_.Current.Name -match '1\.15\.1(?![\d.])' }).Count -gt 0
}

}
if ($HeadingProbe) {
    $oldHeading='目標段落731'
    $newHeading='更新段落731'
    $utf=[Text.UTF8Encoding]::new($false)
    $receiptPath=Join-Path $run 'obsidian-heading-receipt.json'
    if (Test-Path -LiteralPath $receiptPath) { throw 'An existing heading receipt would be overwritten.' }
    $first=[ordered]@{
        'Target.md'=(("## $oldHeading","正文記號 TARGET-731","[[#$oldHeading|保留內文]]","## 別的段落","SEP-731") -join [char]10)
        'Inbound.md'=(("[[Target#$oldHeading|保留顯示]]","[[Decoy#$oldHeading|相同標題的其他筆記]]","INBOUND-731") -join [char]10)
        'Decoy.md'=(("## $oldHeading","正文記號 DECOY-731") -join [char]10)
    }
    $before=[ordered]@{}
    foreach ($name in $first.Keys) {
        $path=Join-Path $vault $name
        $stream=[IO.File]::Open($path,[IO.FileMode]::CreateNew)
        try {
            $bytes=$utf.GetBytes($first[$name])
            $stream.Write($bytes,0,$bytes.Length)
        } finally { $stream.Dispose() }
        $before[$name]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    function OpenHeading([string]$Heading) {
        $uri='obsidian://open?vault='+[uri]::EscapeDataString($vaultName)+'&file='+[uri]::EscapeDataString("Target#$Heading")
        Start-Process $uri
        WaitFor 'exact synthetic Target window is foreground' {
            param($nodes)
            $handle=[ExactPicker]::GetForegroundWindow()
            @($nodes | Where-Object {
                $_.Current.ControlType -eq [Windows.Automation.ControlType]::Window -and
                [IntPtr]$_.Current.NativeWindowHandle -eq $handle -and
                $_.Current.Name -like "Target - $vaultName - Obsidian 1.15.1*"
            }) | Select-Object -First 1
        }
    }
    function InVerifiedWindow([string]$keys) {
        if ([ExactPicker]::GetForegroundWindow() -ne $window) { throw 'The verified synthetic Obsidian window lost focus.' }
        [Windows.Forms.SendKeys]::SendWait($keys)
    }
    $vendor=OpenHeading $oldHeading
    $window=[IntPtr]$vendor.Current.NativeWindowHandle
    $bounds=[ExactPicker]::Bounds($window)
    $heading=WaitFor 'exact source heading is visible in the note editor' {
        param($nodes)
        $exact=@($nodes | Where-Object {
            $rect=$_.Current.BoundingRectangle
            $_.Current.Name -ceq $oldHeading -and !$rect.IsEmpty -and $rect.Left -gt ($bounds[0]+$bounds[2]*0.30)
        })
        if ($exact.Count -eq 1) { $exact[0] }
    }
    $box=$heading.Current.BoundingRectangle
    [ExactPicker]::ClickFixture($window,[int]($box.Left+$box.Width/2),[int]($box.Top+$box.Height/2),$false)
    InVerifiedWindow '^p'
    $null=WaitFor 'visible focused Command Palette search' {
        param($nodes)
        @($nodes | Where-Object { $_.Current.HasKeyboardFocus -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and
            $_.Current.Name -match '^(Select a command|Type a command|Command palette)' }).Count -eq 1
    }
    Set-Clipboard -Value 'Rename this heading'
    InVerifiedWindow '^v'
    $command=WaitFor 'exact Rename this heading palette command' {
        param($nodes)
        $named=@($nodes | Where-Object {
            $_.Current.ControlType -ne [Windows.Automation.ControlType]::Edit -and !$_.Current.BoundingRectangle.IsEmpty
        })
        $exact=@($named | Where-Object { $_.Current.Name -match '^(Editor:\s+)?Rename (this )?heading(?:…|\.\.\.)?$' })
        if ($exact.Count -eq 1) { return $exact[0] }
        # The command palette can expose matched words as separate text elements.
        $parts=@(foreach ($word in 'Rename','this','heading') {
            $hit=@($named | Where-Object { $_.Current.Name.Trim() -ceq $word })
            if ($hit.Count -ne 1) { return }
            $hit[0]
        })
        if ($parts.Count -ne 3) { return }
        $a=$parts[0].Current.BoundingRectangle
        $b=$parts[1].Current.BoundingRectangle
        $c=$parts[2].Current.BoundingRectangle
        if ($a.Right -le $b.Right -and $b.Right -le $c.Right -and
            $c.Left-$a.Right -lt 150 -and
            [Math]::Abs($a.Top-$b.Top) -lt 8 -and [Math]::Abs($a.Top-$c.Top) -lt 8) { $parts[0] }
    }
    $box=$command.Current.BoundingRectangle
    [ExactPicker]::ClickFixture($window,[int]($box.Left+$box.Width/2),[int]($box.Top+$box.Height/2),$false)
    function FocusedHeadingValue([string]$ExpectedValue) {
        WaitFor 'focused rename input equals expected heading' {
            param($nodes)
            $hit=@($nodes | Where-Object {
                $pattern=$null
                $_.Current.HasKeyboardFocus -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and
                $_.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern,[ref]$pattern) -and
                !$pattern.Current.IsReadOnly -and $pattern.Current.Value -ceq $ExpectedValue
            })
            if ($hit.Count -eq 1) { $hit[0] }
        }
    }
    $null=FocusedHeadingValue $oldHeading
    Set-Clipboard -Value $newHeading
    InVerifiedWindow '^a'
    InVerifiedWindow '^v'
    $null=FocusedHeadingValue $newHeading
    InVerifiedWindow '{ENTER}'
    $null=WaitFor 'native heading change reached disk' {
        param($nodes)
        [IO.File]::ReadAllText((Join-Path $vault 'Target.md')).Split([char]10)[0].TrimEnd([char]13) -ceq "## $newHeading"
    }
    $vendorProcess=Get-Process -Id $vendor.Current.ProcessId
    if ($vendorProcess.Path -cne $exe -or !$vendorProcess.CloseMainWindow() -or !$vendorProcess.WaitForExit(20000)) {
        throw 'The verified fixture GUI did not close.'
    }
    $after=[ordered]@{}
    $observed=[ordered]@{}
    foreach ($name in $first.Keys) {
        $path=Join-Path $vault $name
        $observed[$name]=[IO.File]::ReadAllText($path)
        $after[$name]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($after['Decoy.md'] -cne $before['Decoy.md'] -or
        !$observed['Inbound.md'].Contains("[[Decoy#$oldHeading|相同標題的其他筆記]]") -or
        !$observed['Inbound.md'].Contains('INBOUND-731') -or
        !$observed['Target.md'].Contains('TARGET-731') -or !$observed['Target.md'].Contains('SEP-731')) {
        throw 'Unrelated synthetic content was changed.'
    }
    $same=$observed['Target.md'].Contains("[[#$newHeading|保留內文]]") -and !$observed['Target.md'].Contains("[[#$oldHeading|保留內文]]")
    $cross=$observed['Inbound.md'].Contains("[[Target#$newHeading|保留顯示]]") -and !$observed['Inbound.md'].Contains("[[Target#$oldHeading|保留顯示]]")
    $reopened=OpenHeading $newHeading
    $null=WaitFor 'new heading visible after genuine process restart' {
        param($nodes)
        @($nodes | Where-Object { $_.Current.ProcessId -eq $reopened.Current.ProcessId -and $_.Current.Name -ceq $newHeading }).Count -gt 0
    }
    $process=Get-Process -Id $reopened.Current.ProcessId
    if ($process.Path -cne $exe -or !$process.CloseMainWindow() -or !$process.WaitForExit(20000)) { throw 'Reopened fixture GUI did not close.' }
    foreach ($name in $first.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $vault $name) -Algorithm SHA256).Hash -ine $after[$name]) { throw 'A fixture unexpectedly changed on reopen.' }
    }
    [ordered]@{
        product='Obsidian desktop'; product_version=$info.ProductVersion; file_version=$info.FileVersion
        installer_sha256=$expected; os=(Get-CimInstance Win32_OperatingSystem).Caption
        old_heading=$oldHeading; new_heading=$newHeading; native_command='Rename heading'
        same_note_wikilink_alias_preserved=$same; cross_note_wikilink_alias_preserved=$cross
        decoy_unchanged=$true; restarted=$true; before_sha256=$before; after_sha256=$after
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $receiptPath -Encoding utf8
    if (!$same -or !$cross) { throw 'The vendor command ran and persisted the heading; inspect the receipt and actual files for differing link behavior.' }
    Write-Host 'Verified native heading rename, same-note and cross-note wikilinks, unchanged decoy, and restart.'
    return
}
$uri = 'obsidian://open?vault=VaultScopeA&file=' + [uri]::EscapeDataString($From)
Start-Process $uri
$vendor = WaitFor 'native Obsidian has the exact fixture tab in foreground' {
    param($nodes)
    $handle = [ExactPicker]::GetForegroundWindow()
    @($nodes | Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::Window -and
        [IntPtr]$_.Current.NativeWindowHandle -eq $handle -and $_.Current.Name -like 'Flip - VaultScopeA - Obsidian 1.15.1*' }) | Select-Object -First 1
}
$window = [IntPtr]$vendor.Current.NativeWindowHandle
$bounds=[ExactPicker]::Bounds($window)
$sourceFolder=($From -split '/')[0]
$targetFolder=($To -split '/')[0]
$sourceEnd=if ($sourceFolder -ceq 'Active') { 'Archive' } else { 'PinNative' }

function Find-ExplorerLabel($nodes,[string]$name,[int]$top=-1,[int]$bottom=[int]::MaxValue) {
    $visible=@($nodes | Where-Object {
        $box=$_.Current.BoundingRectangle
        $_.Current.Name -ceq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Text -and
        !$box.IsEmpty -and $box.Left -ge $bounds[0] -and $box.Right -lt ($bounds[0]+[int]($bounds[2]*0.30)) -and
        $box.Top -gt $top -and $box.Bottom -lt $bottom
    })
    $ordered=@($visible | Sort-Object { $_.Current.BoundingRectangle.Left })
    if ($ordered.Count -eq 1 -or ($ordered.Count -gt 1 -and
        $ordered[0].Current.BoundingRectangle.Left -lt $ordered[1].Current.BoundingRectangle.Left)) {
        $ordered[0]
    }
}
function ExplorerCenter($node) {
    $box=$node.Current.BoundingRectangle
    @([int]($box.Left+$box.Width/2),[int]($box.Top+$box.Height/2))
}
$sourceLabel=WaitFor 'source folder visible in the real Files sidebar' {
    param($nodes)
    Find-ExplorerLabel $nodes $sourceFolder
}
$nextLabel=WaitFor 'next root-level folder or file constrains the source subtree' {
    param($nodes)
    Find-ExplorerLabel $nodes $sourceEnd
}
$top=[int]$sourceLabel.Current.BoundingRectangle.Bottom
$bottom=[int]$nextLabel.Current.BoundingRectangle.Top
$file=Find-ExplorerLabel @(DesktopNodes) 'Flip' $top $bottom
if (!$file) {
    $at=@(ExplorerCenter $sourceLabel)
    [ExactPicker]::ClickFixture($window,$at[0],$at[1],$false)
    $file=WaitFor 'exact Flip file visible inside the verified source subtree' {
        param($nodes)
        $fromLabel=Find-ExplorerLabel $nodes $sourceFolder
        $fromEnd=Find-ExplorerLabel $nodes $sourceEnd
        if ($fromLabel -and $fromEnd) {
            Find-ExplorerLabel $nodes 'Flip' ([int]$fromLabel.Current.BoundingRectangle.Bottom-1) ([int]$fromEnd.Current.BoundingRectangle.Top+1)
        }
    }
}
$target=WaitFor 'exact destination folder visible in the same Files sidebar' {
    param($nodes)
    Find-ExplorerLabel $nodes $targetFolder
}
$start=@(ExplorerCenter $file)
$end=@(ExplorerCenter $target)
[ExactPicker]::DragFile($window,$start[0],$start[1],$end[0],$end[1])
$null = WaitFor 'vendor physically moved the original Markdown bytes' {
    param($nodes)
    if (Test-Path -LiteralPath $fromFile) { return $false }
    if (!(Test-Path -LiteralPath $toFile -PathType Leaf)) { return $false }
    (Get-FileHash -LiteralPath $toFile -Algorithm SHA256).Hash -ceq $before
}
Write-Host "Official Obsidian 1.15.1 Files sidebar moved $From to $To. SHA-256 $before. Installer SHA-256 $expected."
$vendorProcess=Get-Process -Id $vendor.Current.ProcessId
if ($vendorProcess.Path -cne $exe -or !$vendorProcess.CloseMainWindow() -or !$vendorProcess.WaitForExit(20000)) {
    throw 'The isolated verified vendor GUI did not close.'
}


