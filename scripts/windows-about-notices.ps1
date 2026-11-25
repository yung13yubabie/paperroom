# Verify the native File menu and the single read-only About dialog on a Windows desktop.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExeSha256,
    [string]$NoticeRoot = (Join-Path $PSScriptRoot '../docs/third-party/pdf-assets')
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'The About test requires a Windows desktop.' }

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PaperroomAbout {
    public delegate bool EnumProc(IntPtr w, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr w,out int pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr w,StringBuilder b,int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr w);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr w,uint cmd);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr w,int id);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr w,uint msg,IntPtr a,IntPtr b,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool PostMessage(IntPtr w,uint msg,IntPtr a,IntPtr b);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr menu,uint item,StringBuilder text,int size,uint flags);
    [StructLayout(LayoutKind.Sequential)] struct Mouse {
        public int x,y; public uint data,flags,time; public UIntPtr extra;
    }
    [StructLayout(LayoutKind.Sequential)] struct Key {
        public ushort vk,scan; public uint flags,time; public UIntPtr extra;
    }
    [StructLayout(LayoutKind.Explicit)] struct Payload {
        [FieldOffset(0)] public Key key;
        [FieldOffset(0)] public Mouse mouse;
    }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint type; public Payload data; }
    [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,Input[] input,int size);
    [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr menu,uint item,uint byPosition);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr w);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr w);
    static int ProcessOf(IntPtr w) { int pid; GetWindowThreadProcessId(w,out pid); return pid; }
    public static string State(int pid) {
        var result = new StringBuilder("foregroundIsCandidate=").Append(ProcessOf(GetForegroundWindow())==pid);
        EnumWindows((w,p)=>{
            if(ProcessOf(w)!=pid) return true;
            var cls=new StringBuilder(100); GetClassName(w,cls,100);
            result.Append(" [").Append(cls).Append(" visible=").Append(IsWindowVisible(w))
                  .Append(" enabled=").Append(IsWindowEnabled(w)).Append(" owned=").Append(GetWindow(w,4)!=IntPtr.Zero).Append("]");
            return true;
        },IntPtr.Zero); return result.ToString();
    }
    static void Press(int pid,ushort vk) {
        if(ProcessOf(GetForegroundWindow())!=pid) throw new Exception("Candidate lost foreground: "+State(pid));
        var keys = new Input[2];
        for(int i=0;i<2;i++){
            keys[i].type=1;
            keys[i].data.key.vk=vk;
            keys[i].data.key.flags=(uint)((vk==0x23?1:0)|(i==1?2:0));
        }
        if(SendInput(2,keys,Marshal.SizeOf(typeof(Input)))!=2)
            throw new Exception("SendInput failed ("+Marshal.GetLastWin32Error()+"): "+State(pid));
    }
    public static IntPtr Find(int pid,string cls,IntPtr owner) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((w,p)=>{int id; GetWindowThreadProcessId(w,out id); var name=new StringBuilder(128);
            if(id!=pid || !IsWindowVisible(w) || GetClassName(w,name,128)==0 || name.ToString()!=cls ||
               (owner!=IntPtr.Zero && GetWindow(w,4)!=owner)) return true;
            found=w; return false;},IntPtr.Zero); return found;
    }
    static IntPtr Send(IntPtr w,uint msg,IntPtr a,IntPtr b) {
        IntPtr value;
        if(w==IntPtr.Zero || SendMessageTimeout(w,msg,a,b,2,1500,out value)==IntPtr.Zero)
            throw new Exception("Timed out reading a native control.");
        return value;
    }
    public static string Text(IntPtr w) {
        int length=Send(w,14,IntPtr.Zero,IntPtr.Zero).ToInt32();
        if(length<0 || length>131072) throw new Exception("Unexpected control text size.");
        IntPtr mem=Marshal.AllocHGlobal((length+1)*2);
        try { int n=Send(w,13,(IntPtr)(length+1),mem).ToInt32();
              return Marshal.PtrToStringUni(mem,n); } finally {Marshal.FreeHGlobal(mem);}
    }
    public static void Pick(IntPtr popup,string label) {
        IntPtr menu=Send(popup,0x1e1,IntPtr.Zero,IntPtr.Zero);
        int count=GetMenuItemCount(menu);
        for(uint i=0;i<count;i++){
            var text=new StringBuilder(256); GetMenuString(menu,i,text,256,0x400);
            if(text.ToString()!=label) continue;
            int pid=ProcessOf(popup);
            // About is the final item. Use the real menu keyboard loop; this also
            // works when the menu scrolls on a small desktop. Never activate a
            // row until Win32 reports the requested row is actually highlighted.
            if(i!=count-1) throw new Exception("About is no longer the final File item.");
            Press(pid,0x23); // VK_END
            var timer=System.Diagnostics.Stopwatch.StartNew();
            while(timer.ElapsedMilliseconds<4000) {
                uint state=GetMenuState(menu,i,0x400);
                if(state!=0xffffffff && (state&0x80)!=0 && (state&3)==0 && IsWindowVisible(popup)) {
                    Press(pid,0x0d); // VK_RETURN
                    return;
                }
                System.Threading.Thread.Sleep(40);
            }
            throw new Exception("Native menu did not highlight About: "+State(pid));
        }
        throw new Exception("Could not select the named About item from the native popup.");
    }
}
'@

function Wait-Window([string]$Class,[IntPtr]$Owner) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 20 -and -not $app.HasExited) {
        $window = [PaperroomAbout]::Find($app.Id,$Class,$Owner)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 100
    }
    throw "Did not observe $Class. Candidate state: $([PaperroomAbout]::State($app.Id))"
}

function Stop-Cleanly([IntPtr]$Window) {
    if (-not [PaperroomAbout]::PostMessage($Window,16,0,0) -or
        -not $app.WaitForExit(12000) -or $app.ExitCode -ne 0) { throw 'Application did not exit normally.' }
}

function Start-Portable {
    $info = [Diagnostics.ProcessStartInfo]::new($candidate)
    $info.WorkingDirectory = $scratch
    $info.UseShellExecute = $false
    $info.ArgumentList.Add('--portable')
    foreach ($name in 'LOCALAPPDATA','APPDATA','TEMP','TMP') { $info.Environment[$name] = $scratch }
    return [Diagnostics.Process]::Start($info)
}

function Assert-ClosedDatabase {
    if (-not [IO.File]::Exists($database)) { throw 'Synthetic portable database is missing.' }
    foreach ($suffix in '-wal','-shm','-journal') {
        if ([IO.File]::Exists($database + $suffix)) { throw 'Database has pending sidecars.' }
    }
    return (Get-FileHash -LiteralPath $database -Algorithm SHA256).Hash
}

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('paperroom-about-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($scratch)
$app = $null
try {
    $candidate = Join-Path $scratch 'Paperroom.exe'
    Copy-Item -LiteralPath $Exe -Destination $candidate
    if ((Get-FileHash -LiteralPath $candidate).Hash -ine $ExeSha256) { throw 'Candidate SHA-256 mismatch.' }
    $database = Join-Path $scratch 'paperroom-data/notes.sqlite3'
    $app = Start-Portable
    $main = Wait-Window 'PaperroomWindow' 0
    Stop-Cleanly $main
    $app.Dispose()
    $app = $null
    $before = Assert-ClosedDatabase

    $app = Start-Portable
    $main = Wait-Window 'PaperroomWindow' 0
    [void][PaperroomAbout]::SetForegroundWindow($main)
    $button = [PaperroomAbout]::GetDlgItem($main,109)
    if ([PaperroomAbout]::Text($button) -cne '檔案' -or -not [PaperroomAbout]::PostMessage($button,245,0,0)) {
        throw 'File button unavailable.'
    }
    $popup = Wait-Window '#32768' 0
    [PaperroomAbout]::Pick($popup,'關於紙間／第三方補充通知…')
    $dialog = Wait-Window '#32770' $main
    if ([PaperroomAbout]::Text($dialog) -cne '第三方補充通知') { throw 'Unexpected modal title.' }
    $actual = [PaperroomAbout]::Text([PaperroomAbout]::GetDlgItem($dialog,101)).Replace("`r`n","`n")
    foreach ($marker in '第三方補充通知（PDF 資產及其依賴）','Original code copyright 2014 Foxit Software Inc.',
        'Adobe CMaps','CGATS001、LAB、ProPhoto','alloc-stdlib 0.2.4') {
        if (-not $actual.Contains($marker,[StringComparison]::Ordinal)) { throw "Missing About section marker: $marker." }
    }
    $previous = 0
    foreach ($name in 'pdfium-foxit.txt','adobe-cmap.txt','icc-cc0.txt','alloc-stdlib.txt') {
        $expected = [IO.File]::ReadAllText((Join-Path $NoticeRoot $name)).Replace("`r`n","`n").TrimEnd([char]10)
        $at = $actual.IndexOf($expected,[StringComparison]::Ordinal)
        if ($expected.Length -lt 100 -or $at -lt $previous -or
            $actual.LastIndexOf($expected,[StringComparison]::Ordinal) -ne $at) { throw "Notice mismatch: $name." }
        $previous = $at + $expected.Length
        Write-Host "Verified displayed notice $name"
    }
    if (-not [PaperroomAbout]::PostMessage($dialog,16,0,0)) { throw 'Cannot dismiss About.' }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ([PaperroomAbout]::Find($app.Id,'#32770',$main) -ne [IntPtr]::Zero) {
        if ($timer.Elapsed.TotalSeconds -gt 5) { throw 'About dialog did not close.' }
        Start-Sleep -Milliseconds 50
    }
    Stop-Cleanly $main
    $after = Assert-ClosedDatabase
    if ($before -cne $after) { throw 'About acceptance changed the synthetic database.' }
    Write-Host "EXE SHA-256 $ExeSha256; synthetic database SHA-256 before $before after $after"
} finally {
    if ($null -ne $app) {
        if (-not $app.HasExited) { $app.Kill($true); [void]$app.WaitForExit(5000) }
        $app.Dispose()
    }
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
