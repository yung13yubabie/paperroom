#!/usr/bin/env python3
"""Correlate positive same-link MAP rows with member names in actual searched MSVC CRT archives."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat

_spec = importlib.util.spec_from_file_location(
    "link_inventory", Path(__file__).with_name("windows-link-inventory.py"),
)
link = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(link)

TARGET = "x86_64-pc-windows-msvc"
FAMILIES = ("libcmt", "libucrt", "libvcruntime")
MAX_EXE = 64 << 20
MAX_INVENTORY = 4 << 20
MAX_LONGNAMES = 16 << 20
MAX_MEMBERS = 200_000
MAX_SEARCH_PATHS = 64
COFF_SIGNATURE = b"!<arch>\n"
HEADER_BYTES = 60
MATCHED = "unique-map-member-in-approved-searched-archive"


def coff_long_names(raw):
    if raw and raw[-1] != 0:
        raise ValueError("unterminated COFF longnames table")
    names = {}
    offset = 0
    while offset < len(raw):
        end = raw.find(b"\0", offset)
        if end < 0:
            raise ValueError("unterminated COFF longnames string")
        if offset == end:
            if raw[offset:].strip(b"\0"):
                raise ValueError("empty COFF longnames entry precedes another name")
            break
        if end - offset > 4096 or len(names) >= MAX_MEMBERS:
            raise ValueError("COFF longnames record exceeds the evidence limits")
        names[offset] = raw[offset:end].decode("ascii")
        offset = end + 1
    return names


def member_header(stream, offset, size):
    if offset % 2 or size - offset < HEADER_BYTES:
        raise ValueError("truncated or unaligned COFF archive header")
    stream.seek(offset)
    raw = stream.read(HEADER_BYTES)
    if len(raw) != HEADER_BYTES or raw[58:] != b"`\n":
        raise ValueError("invalid COFF archive member header")
    number = raw[48:58].rstrip(b" ")
    if not number.isdigit():
        raise ValueError("invalid COFF archive member length")
    length = int(number)
    start = offset + HEADER_BYTES
    end = start + length
    next_offset = end + (length & 1)
    if next_offset > size:
        raise ValueError("truncated COFF archive member")
    if end != next_offset:
        stream.seek(end)
        if stream.read(1) != b"\n":
            raise ValueError("invalid COFF archive alignment marker")
    name = raw[:16].rstrip(b" ")
    return name, start, length, next_offset


def canonical_object(name):
    if not name or any(ord(char) < 32 or ord(char) == 127 for char in name):
        raise ValueError("invalid COFF archive member name")
    value = link.member_name(name)
    return value if value and value.endswith(".obj") else None


def coff_members(stream, size, wanted):
    """Read archive headers, two linker directory entries, and optional MS longnames."""
    if size < len(COFF_SIGNATURE) or size > link.MAX_LIBRARY:
        raise ValueError("COFF archive exceeds its allowed size or lacks its signature")
    stream.seek(0)
    if stream.read(8) != COFF_SIGNATURE:
        raise ValueError("expected a standard COFF archive signature")
    offset = len(COFF_SIGNATURE)
    for minimum in (4, 8):
        name, start, length, offset = member_header(stream, offset, size)
        if name != b"/" or length < minimum:
            raise ValueError("expected the first and second Microsoft COFF linker members")
        stream.seek(start)
        rows = int.from_bytes(stream.read(4), "big" if minimum == 4 else "little")
        if minimum + 4 * rows > length:
            raise ValueError("COFF linker directory exceeds its recorded length")
    names = {}
    if offset < size:
        name, start, length, following = member_header(stream, offset, size)
        if name == b"//":
            if length > MAX_LONGNAMES:
                raise ValueError("COFF longnames table exceeds the evidence limit")
            stream.seek(start)
            names = coff_long_names(stream.read(length))
            offset = following

    matches = Counter()
    total = unnamed = 0
    while offset < size:
        name, _, length, offset = member_header(stream, offset, size)
        total += 1
        if length == 0 or total > MAX_MEMBERS:
            raise ValueError("invalid or excessive COFF object-member entries")
        if re.fullmatch(rb"/[0-9]{1,15}", name):
            original = names.get(int(name[1:]))
            if original is None:
                raise ValueError("COFF longname offset is not an entry boundary")
        elif name.endswith(b"/") and not name.startswith(b"/"):
            original = name[:-1].decode("ascii")
        else:
            raise ValueError("unrecognized Microsoft COFF archive member-name form")
        short = canonical_object(original)
        if short is None:
            unnamed += 1
        elif short in wanted:
            matches[short] += 1
    if total == 0:
        raise ValueError("CRT archive has no object members")
    return matches, total, unnamed


def stable_stat(instance):
    return (instance.st_dev, instance.st_ino, instance.st_size,
            instance.st_mtime_ns, instance.st_ctime_ns)


def archive_file(actual, wanted):
    with Path(actual).open("rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > link.MAX_LIBRARY:
            raise ValueError("CRT archive is not a bounded regular file")
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
        matches, count, unnamed = coff_members(stream, metadata.st_size, wanted)
        if stable_stat(os.fstat(stream.fileno())) != stable_stat(metadata):
            raise ValueError("CRT archive changed during inspection")
    return digest, matches, count, unnamed


def versioned_roots(vc_directory, sdk_directory, vc_version, sdk_version):
    roots = link.parse_roots([f"msvc={vc_directory}", f"sdk={sdk_directory}"])
    if (not link.NUMERIC_VERSION.fullmatch(vc_version)
            or not link.NUMERIC_VERSION.fullmatch(sdk_version)
            or roots["msvc"].name.casefold() != vc_version.casefold()):
        raise ValueError("approved roots do not identify the installed toolchain versions")
    return roots


def approved_path(family, roots, sdk_version):
    basename = family + ".lib"
    if family == "libucrt":
        return "sdk", roots["sdk"] / "Lib" / sdk_version / "ucrt" / "x64" / basename
    return "msvc", roots["msvc"] / "lib" / "x64" / basename


def read_traced_archive(traced, root, wanted):
    if os.name != "nt":
        raise OSError("the native archive path requires Windows")
    relative = traced.relative_to(root)
    approved = Path(root).resolve(strict=True)
    actual = Path(traced).resolve(strict=True)
    if not actual.is_relative_to(approved) or actual != approved.joinpath(*relative.parts):
        raise ValueError("traced CRT archive resolves outside its approved versioned location")
    return archive_file(actual, wanted)


def verify_same_link(exe, exe_sha, commit, inventory_raw, inventory_sha, stdout, stderr,
                     vc_version, sdk_version):
    link.pe_identity(exe, exe_sha)
    if link.sha(inventory_raw) != inventory_sha:
        raise ValueError("same-link inventory bytes do not match the expected SHA-256")
    evidence = json.loads(inventory_raw, object_pairs_hook=unique_keys)
    toolchain = evidence.get("toolchain", {})
    if (type(evidence.get("schema_version")) is not int or evidence["schema_version"] != 1
            or evidence.get("map_header_matches_exe") is not True
            or link.exact_hex(evidence["source_commit"], 40) != commit
            or link.exact_hex(evidence["exe_sha256"], 64) != exe_sha
            or toolchain.get("vc_toolset_version") != vc_version
            or toolchain.get("sdk_version") != sdk_version):
        raise ValueError("same-link inventory refers to another EXE, source or toolset")
    if (evidence.get("verbose_stdout_sha256") != link.sha(stdout)
            or evidence.get("verbose_stderr_sha256") != (link.sha(stderr) if stderr else None)):
        raise ValueError("private verbose trace does not match the same-link inventory")
    link.exact_hex(evidence["map_sha256"], 64)
    rows = evidence["map_observations"]["rows"]
    if not isinstance(rows, list) or len(rows) > link.MAX_ROWS:
        raise ValueError("same-link MAP observation rows exceed the supported bounds")
    return evidence, rows


def unique_keys(pairs):
    record = dict(pairs)
    if len(record) != len(pairs):
        raise ValueError("same-link inventory contains repeated JSON keys")
    return record


def map_members(rows):
    observed = {family: Counter() for family in FAMILIES}
    for row in rows:
        family = row.get("family")
        if family not in observed:
            continue
        name = row.get("member")
        count = row.get("symbol_rows")
        if (row.get("kind") != "msvc-crt" or row.get("table") not in {"public", "static"}
                or not isinstance(name, str) or canonical_object(name) != name
                or type(count) is not int or not 1 <= count <= 10_000_000):
            raise ValueError("invalid positive CRT MAP member record")
        observed[family][name] += count
    return observed


def search_paths(streams):
    paths = {family: set() for family in FAMILIES}
    trace_rows = Counter()
    bare_rows = Counter()
    for raw in streams:
        for line in link.decode(raw).splitlines():
            found = link.SEARCHING.fullmatch(link.ANSI_COLOR.sub("", line))
            if found is None:
                continue
            label = found[1]
            basename = link.basename(label)
            family = basename.removesuffix(".lib")
            if basename == family or family not in paths:
                continue
            trace_rows[family] += 1
            resolved = link.canonical_windows(label)
            if resolved is None:
                bare_rows[family] += 1
            else:
                paths[family].add(resolved)
                if len(paths[family]) > MAX_SEARCH_PATHS:
                    raise ValueError("the CRT trace contains too many distinct searched archive paths")
    return paths, trace_rows, bare_rows


def family_report(family, names, searched, trace_count, unbound_count, roots, vc_version, sdk_version,
                  reader=read_traced_archive):
    origin, approved = approved_path(family, roots, sdk_version)
    unapproved = len(searched - {approved})
    inspected = []
    matches = Counter()
    unreadable = 0
    if approved in searched:
        try:
            digest, matches, count, unnamed = reader(approved, roots[origin], set(names))
        except (OSError, RuntimeError, UnicodeError, ValueError):
            unreadable = 1
        else:
            inspected.append({
                "archive": family + ".lib",
                "origin": origin,
                "version": sdk_version if origin == "sdk" else vc_version,
                "sha256": digest,
                "archive_members": count,
                "members_without_normalized_obj_basename": unnamed,
            })
    unresolved_alternatives = bool(unapproved or unbound_count or unreadable)
    rows = []
    for name, count in sorted(names.items()):
        found = matches[name]
        if found > 1:
            status = "ambiguous-member-occurrences"
        elif unresolved_alternatives:
            status = "unresolved-searched-containers"
        elif not inspected:
            status = "no-approved-archive-searched"
        elif found == 0:
            status = "not-found-in-approved-searched-archive"
        else:
            status = MATCHED
        row = {"member": name, "map_symbol_rows": count, "observed_occurrences": found, "status": status}
        if status == MATCHED:
            row["archive_sha256"] = inspected[0]["sha256"]
        rows.append(row)
    return {
        "family": family,
        "search": {
            "trace_search_rows": trace_count,
            "absolute_paths": len(searched),
            "unbound_rows": unbound_count,
            "unapproved_paths": unapproved,
            "unreadable_approved_paths": unreadable,
        },
        "archives": inspected,
        "map_members": rows,
    }


def build_report(exe, exe_sha, commit, inventory_raw, inventory_sha, stdout, stderr,
                 vc_directory, sdk_directory, vc_version, sdk_version, reader=read_traced_archive):
    roots = versioned_roots(vc_directory, sdk_directory, vc_version, sdk_version)
    same_link, rows = verify_same_link(exe, exe_sha, commit, inventory_raw, inventory_sha, stdout, stderr,
                                      vc_version, sdk_version)
    by_family = map_members(rows)
    paths, counts, bare = search_paths((stdout, stderr))
    return {
        "schema_version": 1,
        "target": TARGET,
        "source_commit": commit,
        "exe_sha256": exe_sha,
        "link_inventory_sha256": inventory_sha,
        "map_sha256": same_link["map_sha256"],
        "verbose_stdout_sha256": same_link["verbose_stdout_sha256"],
        "verbose_stderr_sha256": same_link["verbose_stderr_sha256"],
        "toolchain": {"vc_toolset_version": vc_version, "sdk_version": sdk_version},
        "families": [
            family_report(family, by_family[family], paths[family], counts[family], bare[family], roots,
                          vc_version, sdk_version, reader) for family in FAMILIES
        ],
        "scope": (
            "Positive rows correlate a same-link MAP member basename with a single occurrence in the "
            "actual approved archive recorded as searched. This is not a Loaded event or a proof that "
            "every member byte entered the EXE. Searching alone and standalone archive membership attest "
            "neither selection nor complete linked coverage. No redistribution terms are assessed."
        ),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--link-inventory", type=Path, required=True)
    parser.add_argument("--link-inventory-sha256", required=True)
    parser.add_argument("--verbose", type=Path, required=True)
    parser.add_argument("--verbose-stderr", type=Path)
    parser.add_argument("--vc-tools-root", required=True)
    parser.add_argument("--sdk-root", required=True)
    parser.add_argument("--vc-toolset-version", required=True)
    parser.add_argument("--sdk-version", required=True)
    args = parser.parse_args(argv)
    try:
        report = build_report(
            link.bounded(args.exe, MAX_EXE), link.exact_hex(args.exe_sha256, 64), link.exact_hex(args.commit, 40),
            link.bounded(args.link_inventory, MAX_INVENTORY), link.exact_hex(args.link_inventory_sha256, 64),
            link.bounded(args.verbose), link.bounded(args.verbose_stderr) if args.verbose_stderr else b"",
            args.vc_tools_root, args.sdk_root, args.vc_toolset_version, args.sdk_version,
        )
    except ValueError as error:
        # Subtype messages can contain a raw JSON/Unicode parse fragment.
        if type(error) is ValueError:
            parser.exit(2, f"{error}\n")
        parser.exit(2, "CRT evidence is malformed.\n")
    except (AttributeError, KeyError, OSError, RuntimeError, TypeError):
        parser.exit(2, "CRT archival evidence could not be read or verified.\n")
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
