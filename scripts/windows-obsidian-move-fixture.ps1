# Move only a synthetic file in an ephemeral runner, using the official desktop UI.
param(
    [Parameter(Mandatory)][string]$Vault,
    [Parameter(Mandatory)][string]$From,
    [Parameter(Mandatory)][string]$To,
    [switch]$Initialize
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
    throw 'Requires an ephemeral GitHub-hosted Windows runner.'
}
$vault = [IO.Path]::GetFullPath($Vault)
$tempTarget = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
if (!$vault.StartsWith($tempTarget,[StringComparison]::OrdinalIgnoreCase) -or
    (Split-Path $vault -Leaf) -cne 'VaultScopeA' -or
    $From -notmatch '^(Active|Moved)/Flip\.md$' -or $To -notmatch '^(Active|Moved)/Flip\.md$' -or $From -ceq $To) {
    throw 'Only the isolated, fixed fixture may be moved.'
}
$fromFile = Join-Path $vault $From
$toFile = Join-Path $vault $To
if (!(Test-Path -LiteralPath $fromFile -PathType Leaf) -or (Test-Path -LiteralPath $toFile) -or
    !(Test-Path -LiteralPath (Split-Path $toFile -Parent) -PathType Container)) { throw 'Unexpected source or target fixture.' }
$before = (Get-FileHash -LiteralPath $fromFile -Algorithm SHA256).Hash
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
        $result = & $check $nodes
        if ($result) { Write-Host "Passed $stage"; return $result }
        Start-Sleep -Milliseconds 500
    } while ([datetime]::UtcNow -lt $end)
    @($nodes | Where-Object { $_.Current.Name -and $_.Current.Name.Length -le 150 }) |
        Select-Object -First 50 | ForEach-Object { Write-Host ("Visible fixture control: " + $_.Current.ControlType.ProgrammaticName + " " + $_.Current.Name) }
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
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X,Y; }
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags,uint x,uint y,uint wheel,UIntPtr extra);
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
    $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Edit' -and $_.Current.IsEnabled
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
            !$_.Current.IsOffscreen -and $_.Current.Name -ceq 'VaultScopeA' -and $_.Current.ControlType.ProgrammaticName -ne 'ControlType.Edit'
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
$item.Select()
if (!$item.Current.IsSelected -or [ExactPicker]::GetForegroundWindow() -ne $handle) { throw 'The synthetic folder selection did not stick.' }
"Selected the exact synthetic folder item through the native filesystem chooser."
$confirm=[ExactPicker]::GetDlgItem($handle,1)
if ($confirm -eq [IntPtr]::Zero) { throw 'The selected folder has no native confirm button.' }
[ExactPicker]::ClickDefault($handle,$confirm)
$null=WaitFor 'synthetic vault and running app version' {
    param($nodes)
    @($nodes | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and $_.Current.Name -match 'VaultScopeA' -and $_.Current.Name -match '1\.15\.1(?![\d.])' }).Count -gt 0
}

}
$uri = 'obsidian://open?vault=VaultScopeA&file=' + [uri]::EscapeDataString($From)
Start-Process $uri
$vendor = WaitFor 'native Obsidian has the exact fixture tab in foreground' {
    param($nodes)
    $handle = [ExactPicker]::GetForegroundWindow()
    @($nodes | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and
        [IntPtr]$_.Current.NativeWindowHandle -eq $handle -and $_.Current.Name -like 'Flip - VaultScopeA - Obsidian 1.15.1*' }) | Select-Object -First 1
}
$window = [IntPtr]$vendor.Current.NativeWindowHandle
function ExactForegroundKeys([string]$keys) {
    if ([ExactPicker]::GetForegroundWindow() -ne $window) { throw 'The verified vendor window lost focus.' }
    [Windows.Forms.SendKeys]::SendWait($keys)
}
$focusedBefore = [Windows.Automation.AutomationElement]::FocusedElement
$beforeId = if ($focusedBefore) { $focusedBefore.GetRuntimeId() -join ',' } else { '' }
function Find-NewVendorInput([string]$Prior) {
    $focus=[Windows.Automation.AutomationElement]::FocusedElement
    if ($focus -and $focus.Current.ProcessId -eq $vendor.Current.ProcessId -and
        $focus.Current.ControlType.ProgrammaticName -eq 'ControlType.Edit' -and
        ($focus.GetRuntimeId() -join ',') -cne $Prior -and
        [ExactPicker]::GetForegroundWindow() -eq $window) { $focus }
}
ExactForegroundKeys '^p'
$commandInput=WaitFor 'the command palette owns a newly focused text input' { Find-NewVendorInput $beforeId }
$commandId=$commandInput.GetRuntimeId() -join ','
ExactForegroundKeys 'move current file'
$null = WaitFor 'native command palette shows the Move current file command' {
    param($nodes)
    @($nodes | Where-Object {
        $_.Current.Name -match '(?i)\bmove\s+(the\s+)?current\s+file\s+to\s+another\s+folder\b' -and
        $_.Current.ControlType.ProgrammaticName -ne 'ControlType.Edit'
    }).Count -gt 0
}
ExactForegroundKeys '{ENTER}'
$moveInput=WaitFor 'the native Move dialog owns its own focused text input' { Find-NewVendorInput $commandId }
$moveBounds=$moveInput.Current.BoundingRectangle
$folder = ($To -split '/')[0]
ExactForegroundKeys $folder
$null = WaitFor 'destination folder visible in vendor dialog' {
    param($nodes)
    @($nodes | Where-Object {
        $box=$_.Current.BoundingRectangle
        $_.Current.Name -ceq $folder -and $_.Current.ControlType.ProgrammaticName -ne 'ControlType.Edit' -and
        !$box.IsEmpty -and $box.Top -ge ($moveBounds.Bottom-3) -and
        $box.Top -le ($moveBounds.Bottom+160) -and $box.Right -gt $moveBounds.Left -and $box.Left -lt $moveBounds.Right
    }).Count -gt 0
}
ExactForegroundKeys '{ENTER}'
$null = WaitFor 'vendor physically moved the original Markdown bytes' {
    param($nodes)
    if (Test-Path -LiteralPath $fromFile) { return $false }
    if (!(Test-Path -LiteralPath $toFile -PathType Leaf)) { return $false }
    (Get-FileHash -LiteralPath $toFile -Algorithm SHA256).Hash -ceq $before
}
Write-Host "Official Obsidian 1.15.1 command moved $From to $To. SHA-256 $before. Installer SHA-256 $expected."
$vendorProcess=Get-Process -Id $vendor.Current.ProcessId
if ($vendorProcess.Path -cne $exe -or !$vendorProcess.CloseMainWindow() -or !$vendorProcess.WaitForExit(20000)) {
    throw 'The isolated verified vendor GUI did not close.'
}

