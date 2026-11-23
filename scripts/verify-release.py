"""Verify public Paperroom artifacts without checking out the release tag."""

import argparse
from hashlib import file_digest
from pathlib import Path
import re
import shutil
import subprocess
from urllib.request import urlopen
from zipfile import ZipFile


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, stdout=subprocess.PIPE, text=True
    ).stdout.strip()


def digest(path):
    with path.open("rb") as file:
        return file_digest(file, "sha256").hexdigest()


def read_manifest(path, expected):
    hashes = {}
    for line in path.read_text(encoding="ascii").splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](.+)", line)
        if not match:
            raise ValueError("Malformed SHA256SUMS.txt entry")
        sha, name = match.groups()
        if name not in expected or name in hashes:
            raise ValueError("Unexpected or repeated filename in SHA256SUMS.txt")
        hashes[name] = sha.lower()
    if hashes.keys() != expected:
        raise ValueError("SHA256SUMS.txt must list all three release assets")
    return hashes


def archive_members(archive):
    members = {}
    for entry in archive.infolist():
        if entry.filename in members:
            raise ValueError(f"Repeated source ZIP entry: {entry.filename!r}")
        with archive.open(entry) as file:
            members[entry.filename] = file_digest(file, "sha256").digest()
    return members


def verify_release(repo, tag, folder):
    if len(tag) > 80 or not re.fullmatch(
        r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?", tag
    ):
        raise ValueError("Use an exact version tag such as v0.1.1 or v0.1.2-rc.1")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("Expected an owner/repository name")

    git("fetch", "--no-tags", "--no-recurse-submodules", "origin", f"refs/tags/{tag}")
    commit = git("rev-parse", "--verify", "FETCH_HEAD^{commit}")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Expected a GitHub commit ID")
    print(f"Resolved {tag} to {commit}", flush=True)

    exe = "Paperroom.exe"
    windows_zip = f"Paperroom-{tag}-Windows-x64.zip"
    source_zip = f"Paperroom-{tag}-source.zip"
    expected = {exe, windows_zip, source_zip}
    folder.mkdir(parents=True, exist_ok=False)
    for name in ("SHA256SUMS.txt", exe, windows_zip, source_zip):
        url = f"https://github.com/{repo}/releases/download/{tag}/{name}"
        with urlopen(url, timeout=60) as response, (folder / name).open("xb") as file:
            if response.status != 200:
                raise ValueError(f"Public download returned HTTP {response.status}")
            shutil.copyfileobj(response, file)

    hashes = read_manifest(folder / "SHA256SUMS.txt", expected)
    for name, sha in hashes.items():
        if digest(folder / name) != sha:
            raise ValueError(f"Published {name} SHA-256 mismatch")
        print(f"Verified {name} SHA-256 {sha}", flush=True)
    print(f"Public SHA256SUMS.txt SHA-256 {digest(folder / 'SHA256SUMS.txt')}", flush=True)

    with ZipFile(folder / windows_zip) as archive:
        if archive.namelist().count(exe) != 1:
            raise ValueError("Expected exactly one Paperroom.exe at the ZIP root")
        with archive.open(exe) as bundled, (folder / exe).open("rb") as standalone:
            while chunk := bundled.read(1024 * 1024):
                if chunk != standalone.read(len(chunk)):
                    raise ValueError("Windows ZIP EXE differs from the standalone EXE")
            if standalone.read(1):
                raise ValueError("Windows ZIP EXE is shorter than the standalone EXE")
        if archive.testzip() is not None:
            raise ValueError("Windows ZIP failed its CRC check")

    canonical = folder / "tag-tree.zip"
    git(
        "archive", "--format=zip", f"--prefix=paperroom-{tag[1:]}/",
        "-o", str(canonical), commit,
    )
    with ZipFile(folder / source_zip) as published, ZipFile(canonical) as tagged:
        if published.comment != commit.encode("ascii"):
            raise ValueError("Source ZIP comment does not name the resolved commit")
        if archive_members(published) != archive_members(tagged):
            raise ValueError("Source ZIP contents differ from the resolved commit")
    print("Verified source archive contents and the bundled Windows EXE", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("repo")
    parser.add_argument("tag")
    parser.add_argument("folder", type=Path)
    args = parser.parse_args()
    verify_release(args.repo, args.tag, args.folder)
