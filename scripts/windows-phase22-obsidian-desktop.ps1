# Run with pwsh -STA only on an ephemeral, GitHub-hosted Windows runner.
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
    throw 'Requires an ephemeral GitHub-hosted Windows runner.'
}
if ((Get-Process Obsidian -ErrorAction SilentlyContinue) -or (Test-Path "$env:APPDATA/obsidian")) {
    throw 'A running application or pre-existing Obsidian profile was found.'
}
$run = Join-Path $env:RUNNER_TEMP ("paperroom-obsidian-" + [guid]::NewGuid().ToString('N'))
$vault = Join-Path $run PaperroomFixture
$null=New-Item -ItemType Directory $vault -Force
function Put([string]$Name,[string]$Contents) {
    [IO.File]::WriteAllText((Join-Path $vault $Name),$Contents,[Text.UTF8Encoding]::new($false))
}
Put 'Alpha-616.md' "# Alpha Note"
Put 'Bravo-926.md' "# Bravo Note"
Put 'Default-307.md' "# Default Note"
Put 'Shipping.base' @'
views:
  - type: table
    name: Default view
    filters: 'file.path == "Default-307.md"'
    order: [file.name]
  - type: table
    name: Pin_A_601
    filters: 'file.path == "Alpha-616.md"'
    order: [file.name]
  - type: table
    name: Pin_B_904
    filters: 'file.path == "Bravo-926.md"'
    order: [file.name]
'@
Put 'PinNative.canvas' '{"nodes":[],"edges":[]}'
Put 'PinSeeded.canvas' @'
{
  "nodes": [
    {"id":"819f000000000001","type":"file","file":"Shipping.base","x":0,"y":0,"width":380,"height":256},
    {"id":"819f000000000002","type":"file","file":"Shipping.base","x":540,"y":0,"width":380,"height":256}
  ],
  "edges": []
}
'@
$installer = Join-Path $run 'Obsidian-1.15.1.exe'
Invoke-WebRequest 'https://github.com/obsidianmd/obsidian-releases/releases/download/v1.15.1/Obsidian-1.15.1.exe' -OutFile $installer
$expected = '218c7d30acc065355ce94719dd996d5e2a2347cde3811ea5dc29380583f02b58'
if ((Get-FileHash $installer).Hash -ne $expected) { throw 'Installer digest mismatch.' }
$signature = Get-AuthenticodeSignature $installer
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.GetNameInfo('SimpleName',$false) -ne 'Dynalist Inc') { throw 'Installer signature validation failed.' }
"Installer SHA-256 $expected; Windows trust Valid; signer Dynalist Inc."
$setup = Start-Process $installer -ArgumentList '/S' -PassThru
if (!$setup.WaitForExit(120000) -or $setup.ExitCode -ne 0) { throw 'Installer failed or timed out.' }
$exe = Join-Path $env:LOCALAPPDATA 'Programs/Obsidian/Obsidian.exe'
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
    @($nodes | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' }) |
        ForEach-Object { Write-Host ("Observed signed desktop window: " + $_.Current.Name) }
    throw "Blocked at $stage. No matching visible UI in the verified Obsidian process."
}
function Exact($nodes,$name,$kind) {
    @($nodes | Where-Object { $_.Current.Name -ceq $name -and (!$kind -or $_.Current.ControlType.ProgrammaticName -eq "ControlType.$kind") })
}
function Button($node) {
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
if (!(Get-Process Obsidian -ErrorAction SilentlyContinue)) { Start-Process $exe }
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
$value=$edit[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
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
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window, uint flag);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
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
            !$_.Current.IsOffscreen -and $_.Current.Name -ceq 'PaperroomFixture' -and $_.Current.ControlType.ProgrammaticName -ne 'ControlType.Edit'
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
    @($nodes | Where-Object { $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Window' -and $_.Current.Name -match 'PaperroomFixture' -and $_.Current.Name -match '1\.15\.1(?![\d.])' }).Count -gt 0
}
function OpenFixture($file) {
    $path=[uri]::EscapeDataString($file)
    Start-Process "obsidian://open?vault=PaperroomFixture&file=$path"
}
function FixtureWindow([string]$Tab) {
    $deadline = [DateTime]::UtcNow.AddSeconds(18)
    do {
        $wanted = @(
            Get-Process Obsidian -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -ceq $exe -and $_.MainWindowTitle -like "$Tab - PaperroomFixture - Obsidian 1.15.1*" }
        )
        if ($wanted.Count -eq 1 -and [ExactPicker]::GetForegroundWindow() -eq $wanted[0].MainWindowHandle) {
            return $wanted[0]
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The expected foreground Obsidian tab was not seen: $Tab"
}

if (!$env:OBSIDIAN_RECEIPTS) { throw 'The dedicated runner evidence directory is missing.' }
$null = New-Item -ItemType Directory -Path $env:OBSIDIAN_RECEIPTS
$utf8 = [Text.UTF8Encoding]::new($false)
function Record([string]$Name,[string]$Text) {
    [IO.File]::WriteAllText((Join-Path $env:OBSIDIAN_RECEIPTS $Name),$Text,$utf8)
}
function Image([string]$Name,[IntPtr]$Handle) {
    try {
        $r=[ExactPicker]::Bounds($Handle)
        $bmp=[Drawing.Bitmap]::new($r[2],$r[3])
        try {
            $g=[Drawing.Graphics]::FromImage($bmp)
            try { $g.CopyFromScreen($r[0],$r[1],0,0,$bmp.Size) } finally { $g.Dispose() }
            $bmp.Save((Join-Path $env:OBSIDIAN_RECEIPTS $Name),[Drawing.Imaging.ImageFormat]::Png)
        } finally { $bmp.Dispose() }
    } catch { Write-Host 'Screenshot unavailable in the desktop session.' }
}

function Digest([string]$Path) {
    (Get-FileHash -LiteralPath $Path).Hash.ToLowerInvariant()
}
function Canvas-Read([string]$File) {
    Get-Content -LiteralPath (Join-Path $vault $File) -Raw | ConvertFrom-Json -AsHashtable
}
function RectCenter($Element) {
    $q=$Element.Current.BoundingRectangle
    if($q.IsEmpty -or $q.Width -lt 1 -or $q.Height -lt 1) { throw 'No visible bounds for the requested control.' }
    @([int]($q.Left+$q.Width/2),[int]($q.Top+$q.Height/2))
}
function Find-Visible([string]$Pattern,[int]$Seconds,[scriptblock]$Scope) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        try {
            $elements=@(DesktopNodes | Where-Object { $_.Current.Name -cmatch $Pattern -and (&$Scope $_) })
            if($elements.Count) {
                # Chromium often exposes the same label twice with identical bounds.
                $coords=@($elements | ForEach-Object { (RectCenter $_) -join ',' } | Sort-Object -Unique)
                if($coords.Count -eq 1) { return $elements[0] }
            }
        } catch { }
        Start-Sleep -Milliseconds 230
    } while([DateTime]::UtcNow -lt $end)
    return $null
}
function Zoom([IntPtr]$Handle) {
    $r=[ExactPicker]::Bounds($Handle)
    [ExactPicker]::ClickFixture($Handle,($r[0]+[int]($r[2]*0.63)),($r[1]+[int]($r[3]*0.62)),$false)
    [Windows.Forms.SendKeys]::SendWait('+1')
    Start-Sleep -Milliseconds 850
}
function Create-Real-Cards([IntPtr]$Handle) {
    $r=[ExactPicker]::Bounds($Handle)
    $source=Find-Visible '^Shipping(?:\.base)?$' 5 {
        param($n)
        $b=$n.Current.BoundingRectangle
        return !$b.IsEmpty -and $b.Left -ge $r[0] -and $b.Right -lt ($r[0]+$r[2]*0.29) -and $b.Top -gt ($r[1]+68)
    }
    if($source) { $pos=@(RectCenter $source) }
    else {
        # Fresh default file explorer has four alphabetically ordered root files.
        $pos=@(($r[0]+[int]($r[2]*0.090)),($r[1]+[int]($r[3]*0.253)))
    }
    $count=0
    foreach($target in @(@(0.40,0.26),@(0.78,0.61))) {
        [ExactPicker]::DragFile($Handle,$pos[0],$pos[1],($r[0]+[int]($r[2]*$target[0])),($r[1]+[int]($r[3]*$target[1])))
        $count++
        $end=[DateTime]::UtcNow.AddSeconds(5)
        do {
            $data=Canvas-Read 'PinNative.canvas'
            if(@($data.nodes).Count -ge $count) { break }
            Start-Sleep -Milliseconds 260
        } while([DateTime]::UtcNow -lt $end)
        if(@($data.nodes).Count -ne $count -or @($data.nodes|Where-Object {$_.type -cne 'file' -or $_.file -cne 'Shipping.base'}).Count) {
            throw "Native explorer drag did not create Shipping.base card $count."
        }
    }
    return $true
}
function Read-PinChange($Before,$After,[string]$View) {
    $hit=@()
    foreach($entry in $After.nodes) {
        $old=@($Before.nodes | Where-Object { $_.id -ceq $entry.id })
        if($old.Count -ne 1) { throw 'A source card disappeared.' }
        foreach($k in $entry.Keys | Where-Object {$_ -notin 'x','y','width','height'}) {
            $a=ConvertTo-Json -InputObject $entry[$k] -Depth 8 -Compress
            $b=ConvertTo-Json -InputObject $old[0][$k] -Depth 8 -Compress
            if($a -cne $b) { $hit+=@{id=$entry.id;key=$k;value=$entry[$k];selected_view=$View} }
        }
    }
    if($hit.Count -eq 1){return $hit[0]}
    return $null
}
function Pin-View([string]$File,[IntPtr]$Hwnd,[string]$View,$Positions) {
    $before=Canvas-Read $File
    $r=[ExactPicker]::Bounds($Hwnd)
    $context=$null
    foreach($pos in $Positions) {
        [ExactPicker]::RightClick($Hwnd,($r[0]+[int]($r[2]*$pos[0])),($r[1]+[int]($r[3]*$pos[1])))
        Start-Sleep -Milliseconds 220
        $context=Find-Visible '^Pin view(?:…|\.{3})?$' 2 {param($n) $true}
        if($context){break}
        Image "evidence/menu-$View.png" $Hwnd
        [Windows.Forms.SendKeys]::SendWait('{ESC}')
    }
    if(!$context) { throw "The direct Canvas context menu did not expose Pin view for $View." }
    Image "evidence/menu-$View.png" $Hwnd
    $p=@(RectCenter $context)
    [ExactPicker]::ClickFixture($Hwnd,$p[0],$p[1],$false)
    $option=Find-Visible ('^'+[regex]::Escape($View)+'$') 4 {param($n) $true}
    if(!$option) {
        Image "evidence/pin-menu-$View.png" $Hwnd
        throw "The native Pin view chooser did not expose $View."
    }
    Image "evidence/pin-menu-$View.png" $Hwnd
    $p=@(RectCenter $option)
    [ExactPicker]::ClickFixture($Hwnd,$p[0],$p[1],$false)
    $end=[DateTime]::UtcNow.AddSeconds(7)
    do {
        $after=Canvas-Read $File
        $hit=Read-PinChange $before $after $View
        if($hit) { "Native Pin view $View saved." | Write-Host;return $hit }
        Start-Sleep -Milliseconds 260
    } while([DateTime]::UtcNow -lt $end)
    Copy-Item -LiteralPath (Join-Path $vault $File) -Destination (Join-Path $env:OBSIDIAN_RECEIPTS "evidence/After-$View.canvas")
    throw "The original .canvas file has no unique changed per-card value after selecting $View."
}

$null=New-Item -ItemType Directory -Path (Join-Path $env:OBSIDIAN_RECEIPTS 'evidence')
$report=[ordered]@{version=$info.FileVersion;installer_sha256=$expected;native_created=$false;reopened=$false;pass=$false}
foreach($name in 'Shipping.base','Alpha-616.md','Bravo-926.md','Default-307.md'){
    Copy-Item -LiteralPath (Join-Path $vault $name) -Destination (Join-Path $env:OBSIDIAN_RECEIPTS $name)
}
try {
    $file='PinNative.canvas'
    OpenFixture $file
    $app=FixtureWindow 'PinNative'
    $h=$app.MainWindowHandle
    [void][ExactPicker]::ShowWindow($h,3)
    Start-Sleep -Milliseconds 800
    Image 'evidence/Canvas-before-create.png' $h
    try {
        $report.native_created=Create-Real-Cards $h
        Write-Host 'Both file nodes were created by direct native explorer drag.'
    } catch {
        $report['create_blocker']=$_.Exception.Message
        $file='PinSeeded.canvas'
        OpenFixture $file
        $app=FixtureWindow 'PinSeeded'
        $h=$app.MainWindowHandle
    }
    $report['canvas']=$file
    Copy-Item -LiteralPath (Join-Path $vault $file) -Destination (Join-Path $env:OBSIDIAN_RECEIPTS 'evidence/Before-pin.canvas')
    Zoom $h
    Image 'evidence/Before-pins.png' $h

    if($report.native_created){
        $a=@(@(0.43,0.36),@(0.47,0.42),@(0.48,0.47),@(0.44,0.33))
        $b=@(@(0.77,0.61),@(0.80,0.65),@(0.79,0.57),@(0.82,0.71))
    } else {
        $a=@(@(0.43,0.43),@(0.46,0.42),@(0.43,0.55),@(0.42,0.41))
        $b=@(@(0.83,0.43),@(0.81,0.42),@(0.83,0.55),@(0.80,0.41))
    }
    $first=Pin-View $file $h 'Pin_A_601' $a
    $second=Pin-View $file $h 'Pin_B_904' $b
    if($first.id -ceq $second.id -or $first.key -cne $second.key) { throw 'The pins did not persist on separate identities.' }
    $report['pin_A']=$first
    $report['pin_B']=$second
    $after=Canvas-Read $file
    if(@($after.nodes).Count -ne 2 -or @($after.nodes|Where-Object {$_.file -cne 'Shipping.base'}).Count) {
        throw 'Both independently pinned cards no longer share the original .base file.'
    }
    $path=Join-Path $vault $file
    $saved=Digest $path
    Copy-Item -LiteralPath $path -Destination (Join-Path $env:OBSIDIAN_RECEIPTS $file)
    $report['canvas_sha256']=$saved
    Image 'evidence/After-pins.png' $h
    if(!$app.CloseMainWindow() -or !$app.WaitForExit(9000)){throw 'Obsidian did not close its process normally.'}
    OpenFixture $file
    $fresh=FixtureWindow ([IO.Path]::GetFileNameWithoutExtension($file))
    if($fresh.Id -eq $app.Id -or (Digest $path) -cne $saved){throw 'The saved pin result did not survive a new process.'}
    $report.reopened=$true
    Zoom $fresh.MainWindowHandle
    Image 'evidence/Reopened-pins.png' $fresh.MainWindowHandle
    $report.pass=$true
    "PASS actual Pin view selection saved two different named views to nodes $($first.id) and $($second.id), field $($first.key)."
} catch {
    $report['blocker']=$_.Exception.Message
    throw
} finally {
    Record 'result.json' ($report | ConvertTo-Json -Depth 8)
    $list=@(Get-ChildItem -LiteralPath $env:OBSIDIAN_RECEIPTS -File | Sort-Object Name | ForEach-Object { "$(Digest $_.FullName)  $($_.Name)" })
    Record 'SHA256SUMS.txt' (($list -join [char]10)+[char]10)
}
