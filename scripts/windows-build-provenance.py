#!/usr/bin/env python3
"""Describe Cargo's MSVC resolution, bound to an exact candidate digest."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path, PureWindowsPath
import re
import sys

TARGET = "x86_64-pc-windows-msvc"
METADATA_COMMAND = f"cargo metadata --locked --filter-platform {TARGET} --format-version 1"
CRATES_IO = {
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
}
IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.+-]{0,159}", re.ASCII)
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", re.ASCII)
HASH = re.compile(r"[0-9a-fA-F]{64}", re.ASCII)
EXPRESSION = re.compile(r"[A-Za-z0-9(][A-Za-z0-9.+() -]{0,239}", re.ASCII)
NOTICE_NAMES = (
    "COPYRIGHT-library.html", "COPYRIGHT.html", "LICENSE-APACHE", "LICENSE-MIT",
    "LICENSE-THIRD-PARTY", "COPYRIGHT",
)


def bounded(stream, maximum):
    data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("an evidence input exceeds its size limit")
    return data


def read_file(path, maximum):
    with Path(path).open("rb") as stream:
        return bounded(stream, maximum)


def source_kind(source, workspace):
    if source is None:
        return "workspace" if workspace else "path"
    if source in CRATES_IO:
        return "crates.io"
    if source.startswith(("registry+", "sparse+")):
        return "other-registry"
    if source.startswith("git+"):
        return "git"
    return "other"


def notice_basename(value):
    if value is None:
        return None
    name = PureWindowsPath(value).name
    return name if re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_. -]{0,127}", name, re.ASCII) else None


def package_record(package, workspace):
    name, version = package["name"], package["version"]
    if not IDENTIFIER.fullmatch(name) or not VERSION.fullmatch(version) or len(version) > 100:
        raise ValueError("package name or version has an unexpected format")
    license_value = package.get("license")
    if license_value is not None:
        if license_value in {"MIT/Apache-2.0", "Apache-2.0/MIT"}:
            pass  # historical Cargo expression syntax; keep the declaration verbatim
        elif not EXPRESSION.fullmatch(license_value):
            license_value = None
    license_path = package.get("license_file")
    return {
        "name": name, "version": version,
        "source": source_kind(package.get("source"), workspace),
        "license_expression": license_value,
        "license_file_declared": license_path is not None,
        "license_file_basename": notice_basename(license_path),
    }


def unique_index(items):
    result = {item["id"]: item for item in items}
    if len(result) != len(items):
        raise ValueError("duplicate opaque IDs in Cargo metadata")
    return result


def dependency_graph(metadata, exe_hash):
    if not HASH.fullmatch(exe_hash):
        raise ValueError("expected an exact 64-digit EXE SHA-256")
    if metadata.get("version") != 1:
        raise ValueError("expected Cargo metadata format version 1")
    packages = unique_index(metadata["packages"])
    resolve = metadata["resolve"]
    nodes = unique_index(resolve["nodes"])
    members = set(metadata["workspace_members"])
    root = resolve["root"]
    if root not in members or packages[root]["name"] != "paperroom" or packages[root]["source"] is not None:
        raise ValueError("metadata root is not the local Paperroom package")
    roles = defaultdict(set)
    edges = set()
    pending = [(root, "target-normal")]
    while pending:
        source, role = pending.pop()
        if role in roles[source]:
            continue
        roles[source].add(role)
        for dep in nodes[source]["deps"]:
            target = dep["pkg"]
            proc_macro = any("proc-macro" in item["kind"] for item in packages[target]["targets"])
            kinds = dep["dep_kinds"]
            if not kinds:
                raise ValueError("reachable dependency has no dependency-kind evidence")
            for kind_record in kinds:
                kind = kind_record["kind"]
                if kind == "dev":
                    continue
                if kind not in (None, "build"):
                    raise ValueError("unknown Cargo dependency kind")
                target_role = "host-build" if role == "host-build" or kind == "build" or proc_macro else role
                edges.add((source, target, role, target_role, kind or "normal"))
                pending.append((target, target_role))

    records = []
    for opaque, platform_roles in roles.items():
        record = package_record(packages[opaque], opaque in members)
        record["roles"] = sorted(platform_roles)
        # Hashing an absolute checkout path leaks enough information to correlate
        # runs. Public identity here is the explicitly projected safe fields.
        records.append((opaque, record))
    records.sort(key=lambda row: json.dumps(row[1], sort_keys=True))
    fingerprints = {json.dumps(record, sort_keys=True) for _, record in records}
    if len(fingerprints) != len(records):
        raise ValueError("dependency identities collide after removing private provenance")
    ids = {opaque: index for index, (opaque, _) in enumerate(records)}
    return {
        "schema_version": 1,
        "exe_sha256": exe_hash.lower(),
        "target": TARGET,
        "metadata_command": METADATA_COMMAND,
        "scope": "Reachable Cargo resolution; development-only edges omitted. Target and host roles "
                 "derive from dependency kinds in the provided platform-filtered metadata. "
                 "This is not the set of linked objects, notice compliance, or a legal conclusion.",
        "root": ids[root],
        "packages": [record for _, record in records],
        "edges": sorted(
            [ids[fr], ids[to], fr_role, to_role, kind]
            for fr, to, fr_role, to_role, kind in edges
        ),
        "edge_fields": ["from", "to", "from_role", "to_role", "kind"],
    }


def rustc_record(raw):
    checks = {
        "commit-hash": r"[0-9a-f]{40}",
        "commit-date": r"[0-9]{4}-[0-9]{2}-[0-9]{2}",
        "host": r"[a-zA-Z0-9_-]{3,80}",
        "release": VERSION.pattern,
        "LLVM version": r"[0-9]+\.[0-9]+\.[0-9]+",
    }
    result = {}
    for line in raw.splitlines():
        label, sep, value = line.partition(": ")
        if sep and label in checks and re.fullmatch(checks[label], value):
            result[label] = value
    if not {"commit-hash", "host", "release"} <= result.keys():
        raise ValueError("Rust toolchain evidence is missing a valid release, host, or commit")
    return result


def rust_notices(sysroot):
    entries = []
    doc = Path(sysroot) / "share" / "doc" / "rust"
    for name in NOTICE_NAMES:
        path = doc / name
        if path.is_file():
            artifact = read_file(path, 32 * 1024 * 1024)
            entries.append({"filename": name, "sha256": hashlib.sha256(artifact).hexdigest()})
    return {
        "available": entries,
        "copyright_library_observed": any(item["filename"] == NOTICE_NAMES[0] for item in entries),
        "scope": "Bounded file hashes from the supplied installed Rust sysroot; no notice texts "
                 "copied or distributed. Absence here does not establish that notices are unnecessary.",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", default="-", metavar="FILE|-")
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--rustc-info", metavar="FILE")
    parser.add_argument("--rust-sysroot", metavar="DIRECTORY")
    args = parser.parse_args(argv)
    try:
        raw = bounded(sys.stdin.buffer, 32 * 1024 * 1024) if args.metadata == "-" else read_file(args.metadata, 32 * 1024 * 1024)
        report = dependency_graph(json.loads(raw), args.exe_sha256)
        if args.rustc_info is not None:
            report["rustc"] = rustc_record(read_file(args.rustc_info, 32768).decode("utf-8-sig"))
        if args.rust_sysroot is not None:
            report["rust_toolchain_notices"] = rust_notices(args.rust_sysroot)
    except (AttributeError, KeyError, OSError, TypeError, UnicodeError, ValueError):
        parser.exit(2, "Windows build provenance could not be generated from the supplied evidence.\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
