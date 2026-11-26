#!/usr/bin/env python3
"""Collect bounded source notices for the exact candidate's Cargo target graph."""
import argparse
from collections import defaultdict, deque
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import tarfile
import tempfile
import tomllib


TARGET = "x86_64-pc-windows-msvc"
GNU_TARGET = "x86_64-pc-windows-gnu"
BUILD_HOSTS = {TARGET: TARGET, GNU_TARGET: "x86_64-unknown-linux-gnu"}
CRATES_IO = {"registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/"}
CACHE = re.compile(r"(?:index\.crates\.io|github\.com)-[0-9a-f]{16}", re.ASCII)
NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}", re.ASCII)
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", re.ASCII)
SAFE_COMPONENT = re.compile(r"[A-Za-z0-9_.+-][A-Za-z0-9_.+ -]{0,95}", re.ASCII)
RESERVED = re.compile(r"(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])", re.I)
NOTICE = re.compile(
    r"MIT-LICENSE|APACHE-LICENSE|"
    r"(?:LICEN[CS]E|COPYRIGHT|NOTICE|COPYING)"
    r"(?:(?:[-_ ][A-Z0-9]|[.](?:APACHE|BSD|GPL|LGPL|AGPL|MIT|MPL|ISC|ZLIB|CC0|BSL))"
    r"(?:[A-Z0-9_+ -]|[.][0-9]+)*)?"
    r"(?:[.](?:TXT|MD|MARKDOWN|RST|HTML|HTM))?", re.I | re.ASCII,
)
LICENSE_EXPR = re.compile(r"[A-Za-z0-9(][A-Za-z0-9.+() -]{0,239}", re.ASCII)
ROLES = {"target-normal", "host-build"}
EDGE_FIELDS = ["from", "to", "from_role", "to_role", "kind"]
MAX_EXE = 64 << 20
MAX_INDEX = 2 << 20
MAX_ARCHIVE = 32 << 20
MAX_TAR = 96 << 20
MAX_MEMBERS = 16000
MAX_NOTICE = 2 << 20
MAX_NOTICE_COUNT = 1024
MAX_NOTICE_TOTAL = 48 << 20
INDEX_PATH = "evidence/registry-notices.json"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", value):
        raise ValueError("invalid SHA-256 identity")
    return value.lower()


def bounded(stream, maximum):
    data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("an evidence input exceeds its size limit")
    return data


def is_reparse(info):
    return bool(getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0))


def read_regular(path, maximum):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or is_reparse(info) or info.st_size > maximum:
        raise ValueError("an evidence input is not a bounded regular file")
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or (info.st_dev, info.st_ino) != (opened.st_dev, opened.st_ino):
            raise ValueError("an evidence input changed while opening")
        return bounded(stream, maximum)


def unique_object(pairs):
    values = dict(pairs)
    if len(values) != len(pairs):
        raise ValueError("repeated fields in Cargo provenance")
    return values


def safe_component(name):
    return (SAFE_COMPONENT.fullmatch(name) and not name.endswith((".", " "))
            and not RESERVED.fullmatch(name.split(".", 1)[0].rstrip(" ")))


def package_identity(row):
    name, version = row["name"], row["version"]
    if not NAME.fullmatch(name) or not VERSION.fullmatch(version) or not safe_component(version):
        raise ValueError("invalid crate identity")
    if not safe_component(name):
        raise ValueError("crate identity is not a portable path")
    return name, version


def select_receipt(raw, expected_sha, exe_sha, target=TARGET):
    if sha(raw) != expected_sha:
        raise ValueError("Cargo provenance SHA-256 differs")
    doc = json.loads(raw, object_pairs_hook=unique_object)
    if (type(doc["schema_version"]) is not int or doc["schema_version"] != 1
            or digest(doc["exe_sha256"]) != exe_sha or doc["target"] != target
            or doc["rustc"]["host"] != BUILD_HOSTS[target] or doc["edge_fields"] != EDGE_FIELDS):
        raise ValueError("Cargo provenance does not match the supplied Windows candidate")
    packages, edges, root = doc["packages"], doc["edges"], doc["root"]
    if (not isinstance(packages, list) or not 1 <= len(packages) <= 2048
            or not isinstance(edges, list) or len(edges) > 16384
            or type(root) is not int or not 0 <= root < len(packages)
            or packages[root]["name"] != "paperroom" or packages[root]["source"] != "workspace"):
        raise ValueError("invalid bounded Cargo resolution root")
    proofs = []
    for row in packages:
        roles = row["roles"]
        if (not isinstance(roles, list) or not roles or len(roles) > len(ROLES)
                or not set(roles) <= ROLES or len(roles) != len(set(roles))):
            raise ValueError("invalid roles in Cargo provenance")
        proofs.append(set(roles))
    neighbors = defaultdict(set)
    for source, dest, from_role, to_role, kind in edges:
        if (type(source) is not int or not 0 <= source < len(packages)
                or type(dest) is not int or not 0 <= dest < len(packages)
                or from_role not in proofs[source] or to_role not in proofs[dest]
                or kind not in ("normal", "build")
                or (to_role != "host-build" and (kind == "build" or from_role == "host-build"))):
            raise ValueError("invalid dependency edge in Cargo provenance")
        neighbors[(source, from_role)].add((dest, to_role))
    reached = set()
    pending = deque([(root, "target-normal")])
    while pending:
        node = pending.popleft()
        if node not in reached:
            reached.add(node)
            pending.extend(neighbors[node] - reached)
    if reached != {(index, role) for index, roles in enumerate(proofs) for role in roles}:
        raise ValueError("declared Cargo graph roles do not match root reachability")

    selected = {}
    for index, row in enumerate(packages):
        if row["source"] != "crates.io" or (index, "target-normal") not in reached:
            continue
        name, version = package_identity(row)
        license_value = row["license_expression"]
        declared, basename = row["license_file_declared"], row["license_file_basename"]
        if (license_value is not None and license_value not in {"MIT/Apache-2.0", "Apache-2.0/MIT"}
                and not LICENSE_EXPR.fullmatch(license_value)):
            raise ValueError("invalid sanitized license declaration")
        if type(declared) is not bool or (basename is not None and (
            not declared or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_. -]{0,127}", basename, re.ASCII)
        )):
            raise ValueError("invalid sanitized license file declaration")
        key = (name.casefold(), version.casefold())
        if key in selected:
            raise ValueError("reachable registry crate identities collide")
        selected[key] = {
            "name": name, "version": version, "roles": sorted(proofs[index]),
            "license_expression": license_value, "license_file_declared": declared,
            "license_file_basename": basename,
        }
    if not selected:
        raise ValueError("Cargo provenance contains no reachable crates.io target packages")
    return sorted(selected.values(), key=lambda row: (row["name"], row["version"]))


def lock_checksums(raw):
    lock = tomllib.loads(raw.decode("utf-8"))
    if type(lock["version"]) is not int or lock["version"] not in {3, 4}:
        raise ValueError("unsupported Cargo.lock format")
    checksums = {}
    for row in lock["package"]:
        if row.get("source") in CRATES_IO:
            identity = package_identity(row)
            if identity in checksums:
                raise ValueError("duplicate crates.io package in Cargo.lock")
            checksums[identity] = digest(row["checksum"])
    return checksums


def cache_directories(cargo_home):
    cache = cargo_home
    for part in ("registry", "cache"):
        cache /= part
        info = cache.lstat()
        if not stat.S_ISDIR(info.st_mode) or is_reparse(info):
            raise ValueError("Cargo registry cache is not a normal directory")
    found = []
    for path in cache.iterdir():
        if CACHE.fullmatch(path.name):
            if len(found) >= 16:
                raise ValueError("too many crates.io cache folders")
            info = path.lstat()
            if not stat.S_ISDIR(info.st_mode) or is_reparse(info):
                raise ValueError("crates.io cache folder is not a normal directory")
            found.append(path)
    if not found:
        raise ValueError("official crates.io cache folders are missing")
    return sorted(found)


def verified_crate(cache, package, expected):
    basename = f"{package}.crate"
    for directory in cache:
        try:
            data = read_regular(directory / basename, MAX_ARCHIVE)
        except FileNotFoundError:
            continue
        if sha(data) == expected:
            return data
    raise ValueError(f"no cached {basename} matches the Cargo.lock SHA-256")


def tar_path(member, package):
    parts = member.name.split("/")
    if (parts[0] != package or not 1 <= len(parts) <= 32 or len(member.name) > 4096
            or any(part in {"", ".", ".."} or ":" in part or "\\" in part
                   or any(ord(c) < 32 or ord(c) == 127 for c in part) for part in parts)
            or not (member.type in {tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE})
            or member.sparse is not None or member.size < 0
            or (member.isdir() and member.size != 0)
            or (len(parts) == 1 and not member.isdir())):
        raise ValueError(f"invalid member in verified {package} archive")
    return parts[1:]


def notice_files(raw, package):
    with gzip.GzipFile(fileobj=io.BytesIO(raw)) as stream:
        contents = bounded(stream, MAX_TAR)
    seen_members, seen_notices = set(), {}
    notices = []
    notice_bytes = 0
    with tarfile.open(fileobj=io.BytesIO(contents), mode="r:") as archive:
        for count, member in enumerate(archive, start=1):
            if count > MAX_MEMBERS:
                raise ValueError(f"too many members in verified {package} archive")
            parts = tar_path(member, package)
            if member.name in seen_members:
                raise ValueError(f"duplicate member in verified {package} archive")
            seen_members.add(member.name)
            if not member.isfile() or not NOTICE.fullmatch(parts[-1]):
                continue
            relative = "/".join(parts)
            if (len(parts) > 12 or len(relative) > 640 or not all(map(safe_component, parts))
                    or not 0 < member.size <= MAX_NOTICE):
                raise ValueError(f"notice has an unsupported path or size in verified {package} archive")
            notice_bytes += member.size
            if len(notices) >= MAX_NOTICE_COUNT or notice_bytes > MAX_NOTICE_TOTAL:
                raise ValueError(f"notice payloads exceed the limit in verified {package} archive")
            for index in range(1, len(parts) + 1):
                path_prefix = "/".join(parts[:index])
                spelling = (path_prefix, index == len(parts))
                if seen_notices.setdefault(path_prefix.casefold(), spelling) != spelling:
                    raise ValueError(f"notice paths collide in verified {package} archive")
            with archive.extractfile(member) as stream:
                data = bounded(stream, MAX_NOTICE)
            if len(data) != member.size:
                raise ValueError(f"notice has a truncated payload in verified {package} archive")
            notices.append((relative, data))
        end = memoryview(contents)[archive.offset:]
        if not seen_members or len(end) < 1024 or any(end):
            raise ValueError(f"missing or malformed end marker in verified {package} archive")
    return sorted(notices)


def publish_stage(stage, output):
    if os.name == "nt":
        stage.rename(output)  # Win32 rename refuses an existing destination.
    else:
        output.mkdir()
        try:
            stage.replace(output)
        except OSError:
            output.rmdir()
            raise


def collect(exe, exe_sha, provenance, provenance_sha, lock, cargo_home, output, target=TARGET):
    exe_sha, provenance_sha = digest(exe_sha), digest(provenance_sha)
    if sha(read_regular(exe, MAX_EXE)) != exe_sha:
        raise ValueError("Windows EXE SHA-256 differs")
    selected = select_receipt(read_regular(provenance, MAX_INDEX), provenance_sha, exe_sha, target)
    lock_raw = read_regular(lock, MAX_INDEX)
    checksums = lock_checksums(lock_raw)
    cache = cache_directories(cargo_home)
    if output.exists() or output.is_symlink():
        raise ValueError("registry notice output folder already exists")
    total_bytes, count = 0, 0
    with tempfile.TemporaryDirectory(prefix=".registry-notices-", dir=output.parent) as tmp:
        stage = Path(tmp) / "result"
        stage.mkdir()
        for row in selected:
            identity = row["name"], row["version"]
            checksum = checksums.get(identity)
            if checksum is None:
                raise ValueError(f"crate is missing from Cargo.lock: {identity[0]} {identity[1]}")
            package = "-".join(identity)
            raw = verified_crate(cache, package, checksum)
            notices = []
            for relative, artifact in notice_files(raw, package):
                count += 1
                total_bytes += len(artifact)
                if count > MAX_NOTICE_COUNT or total_bytes > MAX_NOTICE_TOTAL:
                    raise ValueError("combined registry notices exceed their size limit")
                path = f"licenses/crates-io/{identity[0]}/{identity[1]}/{relative}"
                notice_path = stage / path
                notice_path.parent.mkdir(parents=True, exist_ok=True)
                with notice_path.open("xb") as stream:
                    stream.write(artifact)
                notices.append({"path": path, "sha256": sha(artifact), "bytes": len(artifact)})
            row.update(archive_sha256=checksum, status="collected" if notices else "no-matching-notice", notices=notices)
        report = {
            "schema_version": 1, "target": target, "exe_sha256": exe_sha,
            "provenance_sha256": provenance_sha, "cargo_lock_sha256": sha(lock_raw),
            "scope": ("Verbatim allowlisted source notices from checksum-verified cached crates.io "
                      "packages reachable as target-normal, including dual-role packages once. "
                      "Absence refers only to matching archive filenames. This is not linker "
                      "evidence or a determination of distribution obligations."),
            "packages": selected,
        }
        index = stage / INDEX_PATH
        index.parent.mkdir()
        index.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
        publish_stage(stage, output)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--provenance", type=Path, required=True)
    parser.add_argument("--provenance-sha256", required=True)
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--cargo-home", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=(TARGET, GNU_TARGET), default=TARGET)
    args = parser.parse_args(argv)
    try:
        report = collect(args.exe, args.exe_sha256, args.provenance, args.provenance_sha256,
                         args.lock, args.cargo_home, args.output, args.target)
    except (TypeError, KeyError, AttributeError, OSError, EOFError, tarfile.TarError, ValueError) as error:
        if type(error) is ValueError:  # These explicit validation messages omit local paths.
            parser.exit(2, f"Could not collect registry notices: {error}.\n")
        parser.exit(2, "Could not collect registry notices from the supplied local evidence.\n")
    rows = report["packages"]
    print(f"Collected {sum(len(row['notices']) for row in rows)} notices for {len(rows)} target packages.")


if __name__ == "__main__":
    main()
