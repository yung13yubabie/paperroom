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
$null=New-Item -ItemType Directory (Join-Path $vault '媒體'),(Join-Path $vault '白板') -Force
$relativeCanvas='白板/Board.canvas'
$boardPath=Join-Path $vault $relativeCanvas
$seed = @'
{
  "nodes": [
    {"id":"1234567890abcde1","type":"group","label":"研究群組","background":"媒體/底圖-紫.png","backgroundStyle":"cover","x":-900,"y":-420,"width":600,"height":680},
    {"id":"1234567890abcde2","type":"file","file":"媒體/場景-青.png","x":-812,"y":-270,"width":306,"height":222},
    {"id":"1234567890abcde3","type":"text","text":"DRAG-CARD-882A","x":-230,"y":-210,"width":548,"height":414},
    {"id":"1234567890abcde4","type":"text","text":"SUMMARY-CARD-17","x":500,"y":134,"width":295,"height":220}
  ],
  "edges": [
    {"id":"2234567890abcde1","fromNode":"1234567890abcde2","fromSide":"right","fromEnd":"none","toNode":"1234567890abcde3","toSide":"left","toEnd":"arrow","color":"#f4a24a","label":"原圖解讀"},
    {"id":"2234567890abcde2","fromNode":"1234567890abcde1","fromSide":"right","fromEnd":"arrow","toNode":"1234567890abcde4","toSide":"top","toEnd":"arrow","color":"#4b68fb","label":"群組往返"},
    {"id":"2234567890abcde3","fromNode":"1234567890abcde3","fromSide":"bottom","fromEnd":"none","toNode":"1234567890abcde4","toSide":"left","toEnd":"none","color":"#59504c","label":"無箭頭參照"}
  ]
}
'@
[IO.File]::WriteAllText($boardPath,$seed,[Text.UTF8Encoding]::new($false))
Add-Type -AssemblyName System.Drawing
function Make-FixturePng([string]$Name,[int]$R,[int]$G,[int]$B,[bool]$Horizontal) {
    $img=[Drawing.Bitmap]::new(192,128)
    try {
        $gfx=[Drawing.Graphics]::FromImage($img)
        try {
            $gfx.Clear([Drawing.Color]::FromArgb($R,$G,$B))
            if ($Horizontal) {
                $gfx.FillRectangle([Drawing.Brushes]::White,13,16,150,15)
                $gfx.FillRectangle([Drawing.Brushes]::Black,29,79,93,19)
            } else {
                $gfx.FillRectangle([Drawing.Brushes]::White,17,13,18,99)
                $gfx.FillRectangle([Drawing.Brushes]::Black,113,35,22,71)
            }
        } finally { $gfx.Dispose() }
        $img.Save((Join-Path $vault $Name),[Drawing.Imaging.ImageFormat]::Png)
    } finally { $img.Dispose() }
}
Make-FixturePng '媒體/場景-青.png' 13 223 147 $false
Make-FixturePng '媒體/底圖-紫.png' 192 31 212 $true
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

function Read-Canvas {
    Get-Content -LiteralPath $boardPath -Raw | ConvertFrom-Json
}
function Digest([string]$Path) {
    (Get-FileHash -LiteralPath $Path).Hash.ToLowerInvariant()
}
function Node($Data,[string]$Id) {
    $found=@($Data.nodes | Where-Object { $_.id -ceq $Id })
    if ($found.Count -ne 1) { throw "Expected one Canvas node $Id." }
    $found[0]
}
function Zoom-Canvas([IntPtr]$Hwnd,[string]$Name) {
    $r=[ExactPicker]::Bounds($Hwnd)
    [ExactPicker]::ClickFixture($Hwnd,($r[0]+[int]($r[2]*0.60)),($r[1]+[int]($r[3]*0.52)),$false)
    [Windows.Forms.SendKeys]::SendWait('+1')
    Start-Sleep -Milliseconds 950
    Image $Name $Hwnd
}
function Media-Pixels([string]$Name) {
    $path=Join-Path $env:OBSIDIAN_RECEIPTS $Name
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) { return @{image=0;background=0} }
    $bmp=[Drawing.Bitmap]::new($path)
    try {
        $image=0
        $background=0
        for($y=132;$y -lt ($bmp.Height-45);$y+=4) {
            for($x=[int]($bmp.Width*0.27);$x -lt ($bmp.Width-45);$x+=4) {
                $p=$bmp.GetPixel($x,$y)
                if ($p.G -ge 144 -and $p.G -ge ($p.R+78) -and $p.G -ge ($p.B+34)) { $image++ }
                if ($p.B -ge 132 -and $p.R -ge 114 -and $p.B -ge ($p.G+63) -and $p.R -ge ($p.G+55) -and
                    [Math]::Abs([int]$p.B-[int]$p.R) -lt 55) { $background++ }
            }
        }
        return @{image=$image;background=$background}
    } finally { $bmp.Dispose() }
}
function Edge-Witness($First,$After) {
    $hits=@($After.edges | Where-Object { $_.id -ceq $First.id })
    if($hits.Count -ne 1) { throw "Missing exact edge $($First.id)." }
    $actual=$hits[0]
    foreach($field in 'id','fromNode','fromSide','toNode','toSide') {
        if($First.$field -cne $actual.$field) { throw "Edge $($First.id) changed $field." }
    }
    $expectedLabel = if($First.id -ceq $editedEdgeId){$newLabel}else{$First.label}
    if($expectedLabel -cne $actual.label) { throw "Edge $($First.id) changed its expected label." }
    if($First.color -ine $actual.color) { throw "Edge $($First.id) changed color." }
    $from=if([string]::IsNullOrEmpty($actual.fromEnd)){'none'}else{$actual.fromEnd}
    $to=if([string]::IsNullOrEmpty($actual.toEnd)){'arrow'}else{$actual.toEnd}
    if($First.fromEnd -cne $from -or $First.toEnd -cne $to) { throw "Edge $($First.id) changed direction." }
    [ordered]@{
        id=$actual.id; fromNode=$actual.fromNode; fromSide=$actual.fromSide; fromEnd=$from
        toNode=$actual.toNode; toSide=$actual.toSide; toEnd=$to; color=$actual.color; label=$actual.label
        serializedFromEnd=$actual.fromEnd; serializedToEnd=$actual.toEnd
    }
}

$source=Read-Canvas
$editedEdgeId='2234567890abcde3'
$oldLabel='無箭頭參照'
$newLabel='無箭頭參照｜已核對'
$media=@('媒體/場景-青.png','媒體/底圖-紫.png')
$assets=[ordered]@{}
foreach ($name in '白板','媒體','evidence') {
    $null=New-Item -ItemType Directory -Path (Join-Path $env:OBSIDIAN_RECEIPTS $name)
}
Copy-Item -LiteralPath $boardPath -Destination (Join-Path $env:OBSIDIAN_RECEIPTS 'evidence/Board-before.canvas')
foreach($name in $media) {
    $assets[$name]=Digest (Join-Path $vault $name)
    Copy-Item -LiteralPath (Join-Path $vault $name) -Destination (Join-Path $env:OBSIDIAN_RECEIPTS $name)
}
$result=[ordered]@{
    version=$info.FileVersion; installer_sha256=$expected; canvas=$relativeCanvas; before_sha256=(Digest $boardPath)
    media_sha256=$assets; reopened=$false; pass=$false
}
try {
    OpenFixture $relativeCanvas
    $app=FixtureWindow 'Board'
    "Observed native nested Canvas tab: $($app.MainWindowTitle)"
    $hwnd=$app.MainWindowHandle
    [void][ExactPicker]::ShowWindow($hwnd,3)
    Start-Sleep -Milliseconds 850
    Zoom-Canvas $hwnd 'evidence/Canvas-opened.png'

    $r=[ExactPicker]::Bounds($hwnd)
    # The existing edge chip was seen near this position in the opened fixture.
    # Prove the actual label editor before committing any desktop edit.
    $x=$r[0]+[int]($r[2]*0.744)
    $y=$r[1]+[int]($r[3]*0.718)
    $foundEdit=$false
    foreach($offset in 0,-5,6) {
        Set-Clipboard -Value ([guid]::NewGuid().ToString('N'))
        [ExactPicker]::ClickFixture($hwnd,$x,($y+$offset),$true)
        Start-Sleep -Milliseconds 350
        if ([ExactPicker]::GetForegroundWindow() -ne $hwnd) { throw 'The Canvas label edit lost foreground.' }
        [Windows.Forms.SendKeys]::SendWait('^a')
        [Windows.Forms.SendKeys]::SendWait('^c')
        Start-Sleep -Milliseconds 230
        if(([string](Get-Clipboard -Raw)).Trim() -ceq $oldLabel) { $foundEdit=$true;break }
        [Windows.Forms.SendKeys]::SendWait('{ESC}')
    }
    if(!$foundEdit) {
        Image 'evidence/Canvas-unconfirmed-edge-label.png' $hwnd
        throw 'The existing edge label did not open as an editable control.'
    }
    "Native exact edge label editor exposed $oldLabel."
    Set-Clipboard -Value $newLabel
    [Windows.Forms.SendKeys]::SendWait('^v')
    [Windows.Forms.SendKeys]::SendWait('{ESC}')
    [ExactPicker]::ClickFixture($hwnd,($r[0]+[int]($r[2]*0.58)),($r[1]+[int]($r[3]*0.24)),$false)
    $deadline=[DateTime]::UtcNow.AddSeconds(9)
    $edited=$false
    do {
        try { $saved=Read-Canvas } catch { $saved=$null }
        if($saved) {
            $labels=@($saved.edges | Where-Object { $_.id -ceq $editedEdgeId -and $_.label -ceq $newLabel })
            if(@($saved.edges).Count -eq 3 -and $labels.Count -eq 1) { $edited=$true;break }
        }
        Start-Sleep -Milliseconds 230
    } while([DateTime]::UtcNow -lt $deadline)
    if(!$edited) { throw 'The exact edge edit did not reach the Canvas file.' }
    $savedHash=Digest $boardPath
    if($savedHash -ceq $result.before_sha256) { throw 'A native file write was not observed.' }
    $result['saved_sha256']=$savedHash
    $result['edited_edge']=[ordered]@{id=$editedEdgeId;before=$oldLabel;after=$newLabel}
    Copy-Item -LiteralPath $boardPath -Destination (Join-Path $env:OBSIDIAN_RECEIPTS $relativeCanvas)
    "Native edit saved edge $editedEdgeId label $newLabel. Canvas SHA-256 $savedHash."

    if(!$app.CloseMainWindow() -or !$app.WaitForExit(9000)) { throw 'Obsidian did not exit after the normal window close.' }
    OpenFixture $relativeCanvas
    $fresh=FixtureWindow 'Board'
    if($app.Id -eq $fresh.Id) { throw 'The reopen did not create a new Obsidian process.' }
    [void][ExactPicker]::ShowWindow($fresh.MainWindowHandle,3)
    Start-Sleep -Milliseconds 850
    Zoom-Canvas $fresh.MainWindowHandle 'evidence/Canvas-reopened.png'
    $after=Read-Canvas
    if((Digest $boardPath) -cne $savedHash -or @($after.nodes).Count -ne 4 -or @($after.edges).Count -ne 3) {
        throw 'The Canvas or its inventory changed on reopen.'
    }
    foreach($originalNode in $source.nodes) {
        $observedNode=Node $after $originalNode.id
        foreach($part in 'x','y','width','height','text') {
            if($observedNode.$part -cne $originalNode.$part) { throw "Canvas node $($originalNode.id) changed $part unexpectedly." }
        }
    }
    $result['reopened']=$true
    $picture=Node $after '1234567890abcde2'
    $group=Node $after '1234567890abcde1'
    if($picture.type -cne 'file' -or $picture.file -cne $media[0] -or $group.type -cne 'group' -or
        $group.label -cne '研究群組' -or $group.background -cne $media[1] -or $group.backgroundStyle -cne 'cover') {
        throw 'The original Chinese asset paths or group metadata changed.'
    }
    $result['file']=[ordered]@{id=$picture.id;path=$picture.file}
    $result['group']=[ordered]@{id=$group.id;label=$group.label;background=$group.background;style=$group.backgroundStyle}
    foreach($name in $media) {
        if((Digest (Join-Path $vault $name)) -cne $assets[$name]) { throw "The source image changed: $name" }
    }
    $result['edges']=@(foreach($edge in $source.edges) { Edge-Witness $edge $after })
    $result['pixels_before']=Media-Pixels 'evidence/Canvas-opened.png'
    $result['pixels_reopened']=Media-Pixels 'evidence/Canvas-reopened.png'
    if($result.pixels_before.image -lt 30 -or $result.pixels_before.background -lt 30 -or
        $result.pixels_reopened.image -lt 30 -or $result.pixels_reopened.background -lt 30) {
        throw 'Both distinctive source colors were not established in the opened and reopened native Canvas.'
    }
    $result['pass']=$true
    'PASS Official Obsidian Canvas retained the native edge edit, original media/group paths and all three attributed edges across a new process.'
} catch {
    $result['blocked']=$_.Exception.Message
    throw
} finally {
    Record 'result.json' ($result | ConvertTo-Json -Depth 10)
    $manifest=@(
        foreach($name in (@($relativeCanvas)+$media+'result.json')) {
            $p=Join-Path $env:OBSIDIAN_RECEIPTS $name
            if(Test-Path -LiteralPath $p -PathType Leaf) { "$(Digest $p)  $name" }
        }
    )
    Record 'SHA256SUMS.txt' (($manifest -join [char]10)+[char]10)
}
