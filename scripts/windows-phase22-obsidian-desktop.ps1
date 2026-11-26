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
$null = New-Item -Type Directory "$vault/A","$vault/B" -Force
function Put($name,$contents) { [IO.File]::WriteAllText((Join-Path $vault $name),$contents,[Text.UTF8Encoding]::new($false)) }
Put 'A/Twin.md' '# Synthetic note A'
Put 'B/Twin.md' '# Synthetic note B'
Put 'Board.canvas' '{"nodes":[{"id":"1234567890abcdef","type":"text","text":"CanvasProof7B42","x":0,"y":0,"width":320,"height":120}],"edges":[]}'
foreach ($folder in 'A','B') {
    Put "$folder.base" @"
filters: 'file.folder == "$folder"'
formulas:
  matching: 'file.path.startsWith("A/")'
  witness: 'if(formula.matching, "PaperroomPredicateTrue", "PaperroomPredicateFalse")'
properties:
  formula.matching: {displayName: "ResultFor$folder"}
  formula.witness: {displayName: "ExactPredicateText"}
views:
  - type: table
    name: "Predicate$folder"
    order: [file.path, formula.matching, formula.witness]
"@
}
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

function CopyBase([string]$Folder,[string]$Literal) {
    OpenFixture "$Folder.base"
    $app=FixtureWindow $Folder
    "Observed desktop tab: $($app.MainWindowTitle)"
    $hwnd=$app.MainWindowHandle
    [void][ExactPicker]::ShowWindow($hwnd,3)
    Start-Sleep -Milliseconds 850
    Image "Base-$Folder.png" $hwnd
    $r=[ExactPicker]::Bounds($hwnd)
    $expectedPath = "$Folder/Twin.md"
    $otherPath = if ($Folder -eq 'A') { 'B/Twin.md' } else { 'A/Twin.md' }
    $otherLiteral = if ($Folder -eq 'A') { 'PaperroomPredicateFalse' } else { 'PaperroomPredicateTrue' }
    # Use only the standard table keyboard commands and fresh clipboard values.
    foreach ($top in 115,143,170) {
        foreach ($side in 0.40,0.52) {
            $nonce=[guid]::NewGuid().ToString('N')
            Set-Clipboard -Value $nonce
            [ExactPicker]::ClickFixture($hwnd,($r[0]+[int]($r[2]*$side)),($r[1]+$top),$false)
            if ([ExactPicker]::GetForegroundWindow() -ne $hwnd) { throw 'The Base view lost foreground.' }
            [Windows.Forms.SendKeys]::SendWait('^a')
            [Windows.Forms.SendKeys]::SendWait('^c')
            Start-Sleep -Milliseconds 260
            $copied=Get-Clipboard -Raw
            $lines=@($copied -split '\r?\n')
            $witness=@($lines | Where-Object { $_.Contains($expectedPath) -and $_.Contains($Literal) })
            if ($copied -ne $nonce -and $witness.Count -eq 1 -and
                !$copied.Contains($otherLiteral) -and !$copied.Contains($otherPath)) {
                Record "Base-$Folder.tsv" $witness[0]
                "PASS Native $Folder.base rendered $expectedPath and copied computed value $Literal from the same row."
                return
            }
        }
    }
    throw "Native $Folder.base opened, but the exact computed cell was not verified by table copy."
}

function ReadBoard {
    Get-Content -LiteralPath (Join-Path $vault 'Board.canvas') -Raw | ConvertFrom-Json
}
function VerifyCanvas {
    $boardPath=Join-Path $vault 'Board.canvas'
    $before=(Get-FileHash -LiteralPath $boardPath).Hash
    OpenFixture 'Board.canvas'
    $app=FixtureWindow 'Board'
    "Observed desktop tab: $($app.MainWindowTitle)"
    $hwnd=$app.MainWindowHandle
    [void][ExactPicker]::ShowWindow($hwnd,3)
    Start-Sleep -Milliseconds 850
    Image 'Canvas-opened.png' $hwnd
    $r=[ExactPicker]::Bounds($hwnd)
    $x=$r[0]+[int]($r[2]*0.59)
    $y=$r[1]+[int]($r[3]*0.52)
    [ExactPicker]::ClickFixture($hwnd,$x,$y,$false)
    [Windows.Forms.SendKeys]::SendWait('+1')
    Start-Sleep -Milliseconds 650
    [ExactPicker]::ClickFixture($hwnd,$x,$y,$true)
    Start-Sleep -Milliseconds 325
    if ([ExactPicker]::GetForegroundWindow() -ne $hwnd) { throw 'The Canvas view lost foreground.' }
    [Windows.Forms.SendKeys]::SendWait('^{END}')
    Set-Clipboard -Value ' CanvasSaved93C1'
    [Windows.Forms.SendKeys]::SendWait('^v')
    [Windows.Forms.SendKeys]::SendWait('{ESC}')
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        try { $data=ReadBoard } catch { $data=$null }
        if ($data -and @($data.nodes).Count -eq 1 -and $data.nodes[0].id -ceq '1234567890abcdef' -and
            $data.nodes[0].text -ceq 'CanvasProof7B42 CanvasSaved93C1') { break }
        Start-Sleep -Milliseconds 300
    } while ([DateTime]::UtcNow -lt $until)
    if (!$data -or @($data.nodes).Count -ne 1 -or $data.nodes[0].id -cne '1234567890abcdef' -or
        $data.nodes[0].text -cne 'CanvasProof7B42 CanvasSaved93C1' -or
        (Get-FileHash -LiteralPath $boardPath).Hash -ceq $before) {
        throw 'The identified text card did not save the expected edit to the original Canvas file.'
    }
    $pidBefore = $app.Id
    if (!$app.CloseMainWindow()) { throw 'The actual Canvas window could not close normally.' }
    if (!$app.WaitForExit(9000)) { throw 'The Obsidian application did not exit after the normal window close.' }
    OpenFixture 'Board.canvas'
    $reopened=FixtureWindow 'Board'
    $after=ReadBoard
    if ($reopened.Id -eq $pidBefore -or @($after.nodes).Count -ne 1 -or
        $after.nodes[0].id -cne '1234567890abcdef' -or $after.nodes[0].text -cne 'CanvasProof7B42 CanvasSaved93C1' -or
        @($after.edges).Count -ne 0) {
        throw 'The stored Canvas edit did not survive the application restart.'
    }
    Start-Sleep -Milliseconds 700
    Image 'Canvas-reopened.png' $reopened.MainWindowHandle
    Record 'Canvas-saved.canvas' ([IO.File]::ReadAllText($boardPath))
    "PASS Native official Canvas edit was saved to the exact node and reopened after normal full process exit. Source hash $before and saved hash $((Get-FileHash -LiteralPath $boardPath).Hash)."
}

$outcome=[ordered]@{version=$info.FileVersion; installer_sha256=$expected; base_A=$false; base_B=$false; canvas=$false}
foreach ($name in 'A','B') {
    $literal=if($name -eq 'A'){'PaperroomPredicateTrue'}else{'PaperroomPredicateFalse'}
    try { CopyBase $name $literal; $outcome["base_$name"]=$true }
    catch { Write-Host "BASE $name BLOCKED: $($_.Exception.Message)" }
}
try { VerifyCanvas; $outcome.canvas=$true }
catch { Write-Host "CANVAS BLOCKED: $($_.Exception.Message)" }
Record 'results.json' ($outcome | ConvertTo-Json)
if (!$outcome.base_A -or !$outcome.base_B -or !$outcome.canvas) {
    throw 'A requested official desktop operation lacks a passing receipt.'
}
