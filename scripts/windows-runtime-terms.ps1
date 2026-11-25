param(
    [Parameter(Mandatory)][string]$SdkRoot,
    [Parameter(Mandatory)][ValidatePattern('^[0-9]+[.][0-9]+[.][0-9]+[.][0-9]+$')][string]$SdkVersion,
    [Parameter(Mandatory)][string]$VisualStudioRoot,
    [Parameter(Mandatory)][ValidatePattern('^[0-9]+[.][0-9]+[.][0-9]+$')][string]$VcToolsetVersion
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$candidates = [ordered]@{
    'sdk-version-license-rtf-1033' = Join-Path $SdkRoot "Licenses/$SdkVersion/1033/SDKLicense.rtf"
    'sdk-version-license-txt-1033' = Join-Path $SdkRoot "Licenses/$SdkVersion/1033/SDKLicense.txt"
    'sdk-version-license-rtf-root' = Join-Path $SdkRoot "Licenses/$SdkVersion/SDKLicense.rtf"
    'sdk-version-license-txt-root' = Join-Path $SdkRoot "Licenses/$SdkVersion/SDKLicense.txt"
    'sdk-version-license-folder-redist' = Join-Path $SdkRoot "Licenses/$SdkVersion/1033/Redist.txt"
    'sdk-version-redist' = Join-Path $SdkRoot "Redist/$SdkVersion/Redist.txt"
    'sdk-version-ucrt-redist' = Join-Path $SdkRoot "Redist/$SdkVersion/ucrt/Redist.txt"
    'vc-toolset-redist' = Join-Path $VisualStudioRoot "VC/Redist/MSVC/$VcToolsetVersion/Redist.txt"
}
$observed = @(
    foreach ($entry in $candidates.GetEnumerator()) {
        if (!(Test-Path -LiteralPath $entry.Value -PathType Leaf)) { continue }
        $file = Get-Item -LiteralPath $entry.Value
        if ($file.Length -le 0 -or $file.Length -gt 2097152 -or
            ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { continue }
        $text = [IO.File]::ReadAllText($file.FullName)
        [ordered]@{
            key = $entry.Key
            bytes = $file.Length
            sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            references_distributable_code = $text -match '(?i)distributable\s+code'
            references_redist_list = $text -match '(?i)redist(?:\.txt|\s+list)'
            names_libucrt = $text -match '(?i)\blibucrt\.lib\b'
            names_libcmt = $text -match '(?i)\blibcmt\.lib\b'
            names_libvcruntime = $text -match '(?i)\blibvcruntime\.lib\b'
        }
    }
)
[ordered]@{
    scope = 'Catalog-only installed files under the exact version; hashes and markers only.'
    status = if ($observed.Count) { 'installed version-specific metadata observed' } else { 'no version-specific metadata at checked locations' }
    sdk_version = $SdkVersion
    vc_toolset_version = $VcToolsetVersion
    checked_location_count = $candidates.Count
    observed = $observed
}
