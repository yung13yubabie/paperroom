# Windows PowerShell 5.1+. Observe local files without launching or writing them.
[CmdletBinding()]
param(
    [string]$OuterZip,
    [string]$InnerZip,
    [string]$DefaultExe,
    [string]$PortableExe,
    [string]$RestoreExe
)

$paths = [ordered]@{
    outer_zip = $OuterZip
    inner_zip = $InnerZip
    default_exe = $DefaultExe
    portable_exe = $PortableExe
    restore_exe = $RestoreExe
}
if (-not ($paths.Values | Where-Object { $_ })) {
    throw 'Specify at least one file to observe.'
}

$available = $false
if ($env:OS -ceq 'Windows_NT') {
    try {
        if (-not ('PaperroomZoneIdentifier' -as [type])) {
            Add-Type -ErrorAction Stop -TypeDefinition @'
using Microsoft.Win32.SafeHandles;
using System;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

public static class PaperroomZoneIdentifier
{
    private const int Missing = -1;
    private const int Unknown = -2;
    private const uint NamedStreams = 0x00040000;
    private const int FileNotFound = 2;
    private const uint GenericRead = 0x80000000;
    private const uint OpenExisting = 3;
    private const int MaxBytes = 16 * 1024;

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetVolumeInformationByHandleW(
        SafeFileHandle file, IntPtr label, uint labelSize, IntPtr serial,
        IntPtr maxNameLength, out uint flags, IntPtr fileSystem, uint fileSystemSize);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern SafeFileHandle CreateFileW(
        string path, uint access, uint share, IntPtr security,
        uint disposition, uint flags, IntPtr templateFile);

    public static int Read(string path)
    {
        // Anchor the volume check to this file, including on mounted volumes.
        // Keep its name stable while testing whether the alternate stream exists.
        using (var file = File.Open(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
        {
            uint flags;
            if (!GetVolumeInformationByHandleW(file.SafeFileHandle, IntPtr.Zero, 0,
                    IntPtr.Zero, IntPtr.Zero, out flags, IntPtr.Zero, 0) ||
                (flags & NamedStreams) == 0)
                return Unknown;

            var stream = CreateFileW(path + ":Zone.Identifier:$DATA", GenericRead,
                (uint)(FileShare.ReadWrite | FileShare.Delete), IntPtr.Zero,
                OpenExisting, 0, IntPtr.Zero);
            int error = Marshal.GetLastWin32Error();
            using (stream)
            {
                if (stream.IsInvalid)
                    return error == FileNotFound ? Missing : Unknown;

                var bytes = new byte[MaxBytes + 1];
                int length = 0;
                using (var reader = new FileStream(stream, FileAccess.Read))
                {
                    while (length < bytes.Length)
                    {
                        int count = reader.Read(bytes, length, bytes.Length - length);
                        if (count == 0)
                            break;
                        length += count;
                    }
                }
                if (length > MaxBytes)
                    return Unknown;
                using (var text = new StreamReader(new MemoryStream(bytes, 0, length),
                    Encoding.UTF8, true))
                    return Parse(text);
            }
        }
    }

    private static int Parse(TextReader text)
    {
        const RegexOptions options = RegexOptions.IgnoreCase | RegexOptions.CultureInvariant;
        bool sawSection = false;
        bool inSection = false;
        int zone = Unknown;
        string line;
        while ((line = text.ReadLine()) != null)
        {
            line = line.Trim();
            if (line.StartsWith("[", StringComparison.Ordinal))
            {
                inSection = string.Equals(line, "[ZoneTransfer]", StringComparison.OrdinalIgnoreCase);
                if (inSection && sawSection)
                    return Unknown;
                sawSection |= inSection;
            }
            else if (inSection && Regex.IsMatch(line, @"^ZoneId(?:[ \t]*=|$)", options))
            {
                var match = Regex.Match(line, @"^ZoneId[ \t]*=[ \t]*([0-9]+)$", options);
                int parsed;
                if (zone != Unknown || !match.Success || !int.TryParse(match.Groups[1].Value,
                        NumberStyles.None, CultureInfo.InvariantCulture, out parsed))
                    return Unknown;
                zone = parsed;
            }
        }
        return zone;
    }
}
'@
        }
        $available = $true
    } catch {
        # Receipt output never includes an exception, path or stream content.
    }
}

$receipt = [ordered]@{}
foreach ($entry in $paths.GetEnumerator()) {
    if (-not $entry.Value) { continue }
    $result = -2
    if ($available) {
        try {
            $item = Get-Item -LiteralPath $entry.Value -ErrorAction Stop
            if ($item -is [IO.FileInfo]) {
                $result = [PaperroomZoneIdentifier]::Read($item.FullName)
            }
        } catch {
            # Missing/unreadable base file, access failures and races are inconclusive.
        }
    }
    $receipt[$entry.Key] = if ($result -ge 0) {
        [ordered]@{ status = 'read'; zone_id = $result }
    } elseif ($result -eq -1) {
        [ordered]@{ status = 'absent'; zone_id = $null }
    } else {
        [ordered]@{ status = 'undetermined'; zone_id = $null }
    }
}
$receipt | ConvertTo-Json -Depth 3 -Compress
