"""Build Windows candidates and verify the exact assets before staging a release."""

import argparse
import hashlib
from io import BytesIO
import os
from pathlib import Path
import re
import shlex
import stat
import struct
import subprocess
import tempfile
from zipfile import BadZipFile, ZipFile


ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-gnu"
GENERIC_PATHS = (
    "/workspace/scratch/", "/home/", "/tmp/", "/private/var/", "/var/folders/",
    "/Users/", "/__w/", "/github/workspace/",
)
PROVENANCE_ENV = (
    "CARGO_HOME", "RUSTUP_HOME", "CARGO_TARGET_DIR", "GITHUB_WORKSPACE",
    "RUNNER_TEMP", "USERPROFILE",
)
TEST_MARKERS = (
    b"PAPERROOM_MEDIA_REPORT_FIXTURE", b"QA_MULTILINE_NARROW",
    b"QA_TAG_GRAND", b"RENAME_TARGET_MARKER",
)
BUILD_WORKSPACE = re.compile(rb"/workspace/scratch/[a-f0-9]{12,}/", re.IGNORECASE)
CREDENTIAL = re.compile(
    rb"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----|"
    rb"gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|"
    rb"AKIA[A-Z0-9]{16}|https?://(?:www\.)?chatgpt\.com/c/[0-9a-f-]{20,}"
)


def assert_windows_x64(data):
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("Expected a Windows PE executable")
    offset = struct.unpack_from("<I", data, 0x3C)[0]
    if offset + 26 > len(data) or data[offset:offset + 4] != b"PE\0\0":
        raise ValueError("Invalid Windows PE header")
    machine = struct.unpack_from("<H", data, offset + 4)[0]
    magic = struct.unpack_from("<H", data, offset + 24)[0]
    if machine != 0x8664 or magic != 0x20B:
        raise ValueError("Expected a Windows x64 PE32+ executable")
    return offset


def path_spellings(prefix):
    forward = str(prefix).replace("\\", "/").rstrip("/") + "/"
    return {forward, forward.replace("/", "\\")}


def hits(data, needles):
    # Win32 resources use UTF-16LE, while Rust/C strings commonly use UTF-8.
    folded = data.lower()
    offsets = set()
    for needle in set(needles):
        for encoding in ("utf-8", "utf-16le"):
            pattern = re.escape(needle.encode(encoding).lower())
            offsets.update(match.start() for match in re.finditer(pattern, folded))
    return len(offsets)


def credential_matches(data):
    # Count known formats without including the contents in error messages.
    return len(CREDENTIAL.findall(data)) + sum(
        len(CREDENTIAL.findall(data[offset::2])) for offset in (0, 1)
    )


def audit(exe, extra_prefixes=()):
    data = exe.read_bytes()
    assert_windows_x64(data)
    roots = {ROOT, ROOT.parent, Path.home(), *extra_prefixes}
    roots.update(Path(os.environ[key]) for key in PROVENANCE_ENV if os.environ.get(key))
    roots = {path for path in roots if path.is_absolute() and path != Path(path.anchor)}
    spellings = set()
    for path in (*roots, *GENERIC_PATHS):
        spellings.update(path_spellings(path))
    path_hits = hits(data, spellings)
    test_hits = hits(data, (marker.decode("ascii") for marker in TEST_MARKERS))
    credential_hits = credential_matches(data)
    if path_hits or test_hits or credential_hits:
        raise ValueError(
            f"Candidate contains forbidden data (path matches {path_hits}, "
            f"test-marker matches {test_hits}, credential matches {credential_hits})"
        )
    print(f"Windows x64 PE audit passed, SHA-256 {hashlib.sha256(data).hexdigest()}")
    return data


def archive_entries(archive):
    entries = {}
    normalized = set()
    for entry in archive.infolist():
        name = entry.filename.rstrip("/") if entry.is_dir() else entry.filename
        parts = name.split("/")
        if (
            entry.orig_filename != entry.filename
            or any(part in ("", ".", "..") or part.endswith((".", " ")) for part in parts)
            or any(char in name for char in ("\\", ":", "\0"))
            or stat.S_ISLNK(entry.external_attr >> 16)
            or name.casefold() in normalized
        ):
            raise ValueError("Archive contains an unsafe or repeated member name")
        normalized.add(name.casefold())
        entries[entry.filename] = entry
    if archive.testzip() is not None:
        raise ValueError("Archive failed its CRC check")
    return entries


def audit_bundle(args):
    exe = audit(args.exe, args.prefix)
    windows_data = args.windows.read_bytes()
    with ZipFile(BytesIO(windows_data)) as windows, ZipFile(args.reference_notices) as reference:
        entries = archive_entries(windows)
        notices = {name for name in reference.namelist() if name.startswith("licenses/")}
        if not notices or entries.keys() != notices | {"Paperroom.exe", "README.zh-Hant.md"}:
            raise ValueError("Windows ZIP differs from the expected payload")
        if windows.comment or any(entry.comment or entry.extra for entry in entries.values()):
            raise ValueError("Unexpected metadata in Windows ZIP")
        if windows.read("Paperroom.exe") != exe:
            raise ValueError("Windows ZIP executable differs from the separately supplied EXE")
        if any(windows.read(name) != reference.read(name) for name in notices):
            raise ValueError("A Windows ZIP license notice differs from the reviewed reference")
        guide = windows.read("README.zh-Hant.md")
        if hashlib.sha256(exe).hexdigest().encode("ascii") not in guide:
            raise ValueError("The Windows ZIP guide omits the executable checksum")
        if credential_matches(guide) or hits(guide, ("/workspace/scratch/", "/home/", "/Users/")):
            raise ValueError("Windows ZIP guide contains material reserved for local use")

    if not re.fullmatch(r"[a-f0-9]{40}", args.commit):
        raise ValueError("Expected a full Git commit ID")
    if not re.fullmatch(r"paperroom-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*", args.source_prefix):
        raise ValueError("Expected one Paperroom directory name for the source archive prefix")
    canonical = subprocess.check_output(
        ["git", "-c", "core.autocrlf=false", "-c", "core.eol=lf", "archive", "--format=zip",
         f"--prefix={args.source_prefix}/", args.commit],
        cwd=ROOT,
    )
    source_data = args.source.read_bytes()
    with ZipFile(BytesIO(source_data)) as source, ZipFile(BytesIO(canonical)) as tracked:
        entries = archive_entries(source)
        if source.comment != args.commit.encode("ascii") or entries.keys() != set(tracked.namelist()):
            raise ValueError("Source ZIP does not match the specified commit")
        count = 0
        for name, entry in entries.items():
            original = tracked.getinfo(name)
            data = source.read(entry)
            if (entry.extra, entry.comment, entry.external_attr, data) != (
                original.extra, original.comment, original.external_attr, tracked.read(original)
            ):
                raise ValueError("Source ZIP content or metadata differs from the Git archive")
            if entry.is_dir():
                continue
            if (
                Path(name).suffix.lower() in {".db", ".sqlite", ".sqlite3", ".pfx", ".p12", ".pem", ".exe", ".bundle", ".key"}
                or data.startswith(b"SQLite format 3\0")
                or credential_matches(data)
                or BUILD_WORKSPACE.search(data.replace(b"\\", b"/"))
                or hits(data, (str(ROOT.parent),))
            ):
                raise ValueError("Source ZIP contains material reserved for local use")
            count += 1
    print(f"Windows ZIP matches the executable and {len(notices)} notices; source ZIP matches {count} tracked files")
    return {
        path.name: hashlib.sha256(data).hexdigest()
        for path, data in ((args.exe, exe), (args.windows, windows_data), (args.source, source_data))
    }


def verified_manifest(path, names):
    if len({name.casefold() for name in names}) != 3:
        raise ValueError("Release assets must have three distinct filenames")
    pattern = re.compile(r"([a-fA-F0-9]{64}) [ *](.+)")
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = pattern.fullmatch(line)
        if not match:
            raise ValueError("Malformed SHA-256 manifest entry")
        digest, name = match.groups()
        if name not in names or name in result:
            raise ValueError("Unexpected or repeated filename in the SHA-256 manifest")
        result[name] = digest.lower()
    if result.keys() != names:
        raise ValueError("The SHA-256 manifest must include all three assets")
    return result


def preflight(args):
    names = {args.exe.name, args.windows.name, args.source.name}
    expected = verified_manifest(args.manifest, names)
    tested = args.tested_exe_sha256.lower()
    if not re.fullmatch(r"[a-f0-9]{64}", tested) or expected[args.exe.name] != tested:
        raise ValueError("Manifest EXE does not match the SHA-256 recorded by acceptance testing")
    actual = audit_bundle(args)
    if actual != expected:
        raise ValueError("An audited asset does not match the SHA-256 manifest")
    print(f"Release preflight passed for source commit {args.commit}")
    for name, digest in actual.items():
        print(f"{digest}  {name}")


def build_gnu(args):
    if os.name == "nt":
        raise ValueError("Use this GNU cross-build command from a Linux build environment")
    target_dir = args.target_dir.resolve()
    cargo = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).expanduser()
    rustup = Path(os.environ.get("RUSTUP_HOME", Path.home() / ".rustup")).expanduser()
    prefixes = {
        Path(tempfile.gettempdir()): "/build-temp",
        target_dir: "/build",
        ROOT: "/paperroom",
        cargo: "/cargo",
        rustup: "/toolchain",
    }
    # Rust uses the last matching prefix. The most specific paths come last.
    mappings = sorted(
        {(str(p.resolve()), token) for p, token in prefixes.items() if p.resolve() != Path("/")},
        key=lambda pair: (len(pair[0]), pair[0], pair[1]),
    )
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target_dir)
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(
        f"--remap-path-prefix={old}={new}" for old, new in mappings
    )
    # cc-rs honors target-specific CFLAGS for the bundled SQLite amalgamation.
    env["CC_SHELL_ESCAPED_FLAGS"] = "1"
    env["CFLAGS_x86_64_pc_windows_gnu"] = shlex.join(
        f"-ffile-prefix-map={old}={new}" for old, new in mappings
    )
    common = ["--locked", "--release", "--target", TARGET]
    if args.offline:
        common.append("--offline")
    # Supply the GNU PE option at the application's final link, without
    # modifying the linked binary or changing dependency build flags.
    subprocess.run(
        ["cargo", "rustc", *common, "--bin", "paperroom", "--", "-Clink-arg=-Wl,--no-insert-timestamp"],
        cwd=ROOT, env=env, check=True,
    )
    exe = target_dir / TARGET / "release" / "paperroom.exe"
    data = exe.read_bytes()
    pe = assert_windows_x64(data)
    if struct.unpack_from("<I", data, pe + 8)[0] != 0:
        raise ValueError("GNU linked an unexpected nonzero PE timestamp")
    audit(exe, (target_dir, cargo.resolve(), rustup.resolve()))
    if args.tests:
        subprocess.run(["cargo", "test", *common, "--no-run"], cwd=ROOT, env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="command", required=True)
    build = actions.add_parser("build-gnu")
    build.add_argument("--target-dir", type=Path, default=ROOT / "target" / "windows-candidate")
    build.add_argument("--offline", action="store_true")
    build.add_argument("--tests", action="store_true")
    check = actions.add_parser("audit")
    check.add_argument("exe", type=Path)
    check.add_argument("--prefix", type=Path, action="append", default=[])
    for action in ("audit-bundle", "preflight"):
        bundle = actions.add_parser(action)
        bundle.add_argument("exe", type=Path)
        bundle.add_argument("windows", type=Path)
        bundle.add_argument("source", type=Path)
        bundle.add_argument("reference_notices", type=Path)
        bundle.add_argument("commit")
        bundle.add_argument("--prefix", type=Path, action="append", default=[])
        bundle.add_argument("--source-prefix", default="paperroom-offline-workspace")
        if action == "preflight":
            bundle.add_argument("--manifest", type=Path, required=True)
            bundle.add_argument("--tested-exe-sha256", required=True)
    args = parser.parse_args()
    try:
        if args.command == "build-gnu":
            build_gnu(args)
        elif args.command == "audit-bundle":
            audit_bundle(args)
        elif args.command == "preflight":
            preflight(args)
        else:
            audit(args.exe, args.prefix)
    except (OSError, ValueError, BadZipFile, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
