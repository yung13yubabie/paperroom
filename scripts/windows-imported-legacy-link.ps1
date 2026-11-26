# Runs in the exact downloaded draft EXE suite, using its isolated workspaces.
$originalTitle = '同編號的本機頁'
$receivedTitle = '交接的 Markdown'
$legacyHome = Join-Path $root 'legacy-link'
$legacyDb = Join-Path $legacyHome 'paperroom-data/notes.sqlite3'
$originalFile = Join-Path $legacyHome "$originalTitle.md"
$receivedFile = Join-Path $legacyHome "$receivedTitle.md"
[void][IO.Directory]::CreateDirectory($legacyHome)
Copy-Item -LiteralPath $Exe -Destination (Join-Path $legacyHome 'Paperroom.exe')
[IO.File]::WriteAllText($originalFile, '本機筆記原文', [Text.UTF8Encoding]::new($false))

function Read-LegacyFixture([string]$Title, [bool]$ExpectImported) {
    $information = @'
import sqlite3, sys
from contextlib import closing
from pathlib import Path
path, title, received, original = sys.argv[1:]
with closing(sqlite3.connect(Path(path).as_uri()+'?mode=ro',uri=True)) as db:
    assert db.execute('pragma journal_mode').fetchone() == ('wal',)
    assert db.execute('pragma user_version').fetchone() == (36,)
    id, body = db.execute('SELECT id,body FROM notes WHERE title=?', (title,)).fetchone()
    if received == "True":
        assert body.encode('utf-8') == Path(original).read_bytes()
        assert db.execute('SELECT count(*) FROM legacy_note_link_bindings WHERE source_id=?',
                          (id,)).fetchone() == (0,)
    print(id)
'@ | python - $legacyDb $Title "$ExpectImported" $receivedFile
    if ($LASTEXITCODE -ne 0) { throw 'Legacy fixture changed Markdown or did not reopen in WAL.' }
    return [long]$information
}

function Assert-NoNativeLegacyJump {
    Click-WorkspaceButton 120
    $menu = Wait-For 'owned native Links popup' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0)
    }
    $options = @([PaperroomHighlightGui]::CurrentMenuLabels($window,$process.Id,$menu,$false))
    if ($options.Count -ne 5 -or
        @($options | Where-Object { $_ -match '^(前往|反向連結)' }).Count -ne 0) {
        throw "Imported legacy link unexpectedly became a native menu entry: $($options -join ', ')."
    }
    [PaperroomHighlightGui]::TapKey($window,$process.Id,0x1B)
    $null = Wait-For 'legacy link menu dismissed' {
        [PaperroomHighlightGui]::OwnedWindow($window,$process.Id,'#32768',$null,0) -eq [IntPtr]::Zero
    }
}

$process = Start-Notebook $legacyHome
$window = $process.MainWindowHandle
Native-File 1 '匯入 Markdown…' $originalFile '已匯入 1 篇 Markdown'
$localId = Read-LegacyFixture $originalTitle $false
$legacyText = '[[paperroom:note:{0}|舊工作區的筆記]]' -f $localId
[IO.File]::WriteAllText($receivedFile, $legacyText, [Text.UTF8Encoding]::new($false))
$receivedSha = (Get-FileHash -LiteralPath $receivedFile).Hash
Native-File 1 '匯入 Markdown…' $receivedFile '已匯入 1 篇 Markdown'
$null = Read-LegacyFixture $receivedTitle $true
Assert-NoNativeLegacyJump
Close-Notebook $process
$process.Dispose()
$process = Start-Notebook $legacyHome
$window = $process.MainWindowHandle
Select-Note $receivedTitle
Assert-NoNativeLegacyJump
$null = Read-LegacyFixture $receivedTitle $true
Select-Note $originalTitle
Assert-NoNativeLegacyJump
Close-Notebook $process
$process.Dispose()
$process = $null
if ((Get-FileHash -LiteralPath $receivedFile).Hash -cne $receivedSha) {
    throw 'The source Markdown file changed.'
}
Write-Host "PASS: native Markdown import preserved the legacy source text without cross-note jumps or backlinks, and the actual workspace reopened in WAL. EXE SHA-256 $ExeSha256."
