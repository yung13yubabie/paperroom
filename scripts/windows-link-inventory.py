#!/usr/bin/env python3
"""Summarize private MSVC final-link observations without copying private logs."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
import re
import struct
import tomllib

MAX_EVIDENCE = 96 << 20
MAX_LIBRARY = 192 << 20
MAX_ROWS = 4_096
CRATES_IO = frozenset(("registry+https://github.com/rust-lang/crates.io-index",
                       "sparse+https://index.crates.io/"))
CARGO_NAME = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,127}", re.ASCII)
CARGO_VERSION = re.compile(
    r"[0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9.-]+)?(?:\+[a-zA-Z0-9.-]+)?", re.ASCII,
)
ROOT_KINDS = {"msvc", "sdk", "rust", "target"}
CRT = frozenset(
    "libcmt libcmtd libucrt libucrtd libvcruntime libvcruntimed libcpmt libcpmtd "
    "msvcrt msvcrtd ucrt ucrtd vcruntime vcruntimed msvcprt msvcprtd oldnames "
    "legacy_stdio_definitions legacy_stdio_wide_specifiers".split()
)
SYSTEM_IMPORTS = frozenset(
    "advapi32 bcrypt bcryptprimitives comctl32 comdlg32 gdi32 imm32 kernel32 "
    "ntdll ole32 oleaut32 powrprof secur32 shell32 shlwapi user32 userenv "
    "uxtheme version winmm ws2_32".split()
)
BASELINE_RUST = frozenset(
    "addr2line adler2 alloc cfg_if compiler_builtins core getopts gimli hashbrown "
    "libc memchr miniz_oxide object panic_abort panic_unwind proc_macro "
    "rustc_demangle rustc_std_workspace_alloc rustc_std_workspace_core std "
    "std_detect test unwind windows_link windows_targets windows_x86_64_msvc".split()
)
ASCII_MEMBER = re.compile(r"[a-zA-Z0-9_$.-]{1,180}\.(?:o|obj|dll)", re.ASCII | re.I)
MAP_ROW = re.compile(r"^\s*[0-9a-f]{4}:[0-9a-f]{8}\s+\S+\s+([0-9a-f]{8,16})\s+(.*?)\s*$", re.I)
NUMERIC_VERSION = re.compile(r"[0-9]{1,6}(?:\.[0-9]{1,8}){1,4}", re.ASCII)
VS_PRODUCTS = frozenset("Microsoft.VisualStudio.Product." + name for name in
                        ("BuildTools", "Community", "Enterprise", "Professional"))
SEARCHING = re.compile(r'^\s*Searching\s+(.+?\.(?:lib|rlib)"?)\s*:\s*$', re.I)
LOADED = re.compile(r'^\s*Loaded\s+(.+?\.(?:lib|rlib)"?)\s*\(([^()]*)\)\s*$', re.I)

LOADED_COLON = re.compile(r'^\s*Loaded\s+(.+?\.(?:lib|rlib)"?)\s*:\s*([^():]+)\s*$', re.I)
ANSI_COLOR = re.compile(r'\x1b\[[0-9;]{0,32}m')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def exact_hex(value, length):
    if not isinstance(value, str) or re.fullmatch(rf"[0-9a-fA-F]{{{length}}}", value) is None:
        raise ValueError("invalid evidence digest or source identity")
    return value.lower()


def bounded(path, limit=MAX_EVIDENCE):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("link evidence exceeds the permitted input size")
    return data


def decode_with_encoding(raw):
    if not raw:
        return "", "empty"
    if raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        return raw.decode("utf-16"), "utf-16-with-bom"
    sample = raw[:8192]
    pairs = len(sample) // 2
    if pairs >= 8:
        even = sample[:pairs * 2:2].count(0)
        odd = sample[1:pairs * 2:2].count(0)
        for high, low, encoding in ((odd, even, "utf-16-le"), (even, odd, "utf-16-be")):
            if high * 100 >= pairs * 35 and low * 100 <= pairs * 8:
                return raw.decode(encoding, errors="replace"), encoding + "-without-bom"
    try:
        return raw.decode("utf-8-sig"), "utf-8-with-bom" if raw.startswith(b"\xef\xbb\xbf") else "utf-8"
    except UnicodeDecodeError:
        return (raw.decode("mbcs" if os.name == "nt" else "windows-1252", errors="replace"),
                "windows-ansi" if os.name == "nt" else "windows-1252-fallback")


def decode(raw):
    return decode_with_encoding(raw)[0]


def canonical_windows(value):
    # Parse privately. Executing hash reads requires a second, actual Windows
    # containment check after resolving any filesystem indirection.
    text = value.strip().strip('"')
    if text.startswith("\\\\?\\") and not text.lower().startswith("\\\\?\\unc\\"):
        text = text[4:]
    path = PureWindowsPath(text)
    if (not re.fullmatch(r"[A-Za-z]:", path.drive) or not path.is_absolute()
        or any(part.endswith((".", " ")) or ":" in part for part in path.parts[1:])
        or any(ch in text for ch in "\r\n\0\ufffd")):
        return None
    return path


def parse_roots(records):
    roots = {}
    for record in records:
        key, sep, directory = record.partition("=")
        root = canonical_windows(directory)
        if not sep or key not in ROOT_KINDS or key in roots or root is None or len(root.parts) < 2:
            raise ValueError("approved roots must use distinct known Windows root kinds")
        roots[key] = root
    if len(set(roots.values())) != len(roots):
        raise ValueError("approved source roots must be distinguishable")
    return roots


def root_kind(path, roots):
    eligible = [(len(root.parts), key) for key, root in roots.items() if path.is_relative_to(root)]
    return max(eligible)[1] if eligible else "unresolved"


def basename(value):
    return PureWindowsPath(value.strip().strip('"')).name.lower()


def normalize_crate_name(name):
    return name.lower().replace("-", "_")


def lock_packages(raw):
    if raw is None:
        return {}
    document = tomllib.loads(raw.decode("utf-8"))
    packages = document.get("package")
    if document.get("version") not in (3, 4) or not isinstance(packages, list) or len(packages) > 4096:
        raise ValueError("unsupported Cargo.lock structure")
    registry = defaultdict(set)
    for package in packages:
        if not isinstance(package, dict) or not isinstance(package.get("source"), str) or package["source"] not in CRATES_IO:
            continue
        name, version = package.get("name"), package.get("version")
        if (not isinstance(name, str) or not CARGO_NAME.fullmatch(name)
            or not isinstance(version, str) or len(version) > 100 or not CARGO_VERSION.fullmatch(version)):
            raise ValueError("unexpected crates.io identity in Cargo.lock")
        registry[normalize_crate_name(name)].add((name, version))
    return registry


def family(value, registry, origin="unresolved"):
    name = basename(value)
    stem = name.removesuffix(".lib")
    if stem in CRT:
        return stem, "msvc-crt"
    if stem in SYSTEM_IMPORTS:
        return stem, "system-import-library"
    if stem in {"sqlite3", "libsqlite3"}:
        return "sqlite3", "sqlite-native-archive"
    rust = re.fullmatch(r"([a-z0-9_][a-z0-9_-]*)-[0-9a-f]{8,64}(?:\.rlib|\.lib)?", name)
    if rust is None:
        return None
    raw_family = rust[1]
    # Rust archives normally have a lib prefix. Retain a literal basename
    # match too, without silently preferring a lock package over another.
    candidates = {normalize_crate_name(raw_family)}
    if raw_family.startswith("lib") and len(raw_family) > 3:
        candidates.add(normalize_crate_name(raw_family[3:]))
    baseline = candidates & BASELINE_RUST
    cargo = candidates & registry.keys()
    if len(baseline) == len(cargo) == 1 and baseline == cargo:
        name = next(iter(cargo))
        if origin == "rust":
            return name, "rust-toolchain-archive"
        if origin == "target":
            return name, "cargo-registry-archive"
        return name, "rust-or-cargo-archive"
    if len(baseline) == 1 and not cargo:
        return next(iter(baseline)), "rust-toolchain-archive"
    if len(cargo) == 1 and not baseline:
        return next(iter(cargo)), "cargo-registry-archive"
    return None


def member_name(value):
    name = basename(value)
    return name if ASCII_MEMBER.fullmatch(name) else None


def pe_identity(data, expected):
    if sha(data) != expected or data[:2] != b"MZ" or len(data) < 64:
        raise ValueError("the supplied executable does not match the pinned PE digest")
    offset = struct.unpack_from("<I", data, 0x3C)[0]
    if offset + 24 + 64 > len(data) or data[offset:offset + 4] != b"PE\0\0":
        raise ValueError("candidate PE header is invalid")
    machine, _, timestamp = struct.unpack_from("<HHI", data, offset + 4)
    size, = struct.unpack_from("<H", data, offset + 20)
    opt = offset + 24
    if machine != 0x8664 or size < 64 or opt + size > len(data) or data[opt:opt + 2] != b"\x0b\x02":
        raise ValueError("candidate is not a valid x64 PE32+ image")
    base, = struct.unpack_from("<Q", data, opt + 24)
    image_size, = struct.unpack_from("<I", data, opt + 56)
    if not image_size:
        raise ValueError("candidate has an empty PE image")
    return timestamp, base, image_size


def parse_map(text, pe, registry=None):
    registry = registry or {}
    timestamp, base, image_size = pe
    stamp = re.findall(r"(?im)^\s*Timestamp is\s+([0-9a-f]{1,8})(?:\s|$)", text)
    preferred = re.findall(r"(?im)^\s*Preferred load address is\s+([0-9a-f]{8,16})\s*$", text)
    if (len(stamp) != 1 or len(preferred) != 1
        or int(stamp[0], 16) != timestamp or int(preferred[0], 16) != base):
        raise ValueError("MAP timestamp or image base does not match the exact PE")
    section = None
    rows = Counter()
    counts = Counter()
    for line in text.splitlines():
        trimmed = line.strip().lower()
        if "publics by value" in trimmed and "lib:object" in trimmed:
            section = "public"
        elif trimmed == "static symbols":
            section = "static"
        elif trimmed.startswith(("entry point", "line numbers", "exports")):
            section = None
        if section is None:
            continue
        match = MAP_ROW.fullmatch(line)
        if match is None:
            continue
        address, trailing = match.groups()
        provenance = re.sub(r"^(?:[fi]\s+)*", "", trailing, flags=re.I)
        archive, sep, member = provenance.rpartition(":")
        kind = family(archive, registry) if sep else None
        name = member_name(member) if kind else None
        if name is None or not base <= int(address, 16) < base + image_size:
            counts["unattributed_symbol_rows"] += 1
            continue
        rows[(*kind, name, section)] += 1
        counts["attributed_symbol_rows"] += 1
        if len(rows) > MAX_ROWS:
            raise ValueError("MAP evidence has too many distinct retained observations")
    return rows, counts


def archive_hash(path, root):
    if os.name != "nt" or path is None:
        return None
    actual = Path(path)
    try:
        normalized = actual.resolve(strict=True)
        approved = Path(root).resolve(strict=True)
        if not normalized.is_relative_to(approved) or not normalized.is_file():
            return None
        return sha(bounded(normalized, MAX_LIBRARY))
    except (OSError, RuntimeError, ValueError):
        return None


def parse_verbose(streams, roots, registry=None):
    registry = registry or {}
    rows = Counter()
    counts = Counter()
    instances = defaultdict(set)
    hashes = {}
    for text in streams:
        active = None
        for line in text.splitlines():
            counts["decoded_lines"] += 1
            line, removed = ANSI_COLOR.subn("", line)
            counts["ansi_color_sequences_removed"] += removed
            action = line.strip().lower()
            if action.startswith("searching "):
                counts["searching_literal_rows"] += 1
            if action.startswith("loaded "):
                counts["loaded_literal_rows"] += 1
            if action.startswith("found "):
                counts["found_symbol_marker_rows"] += 1
            if action.startswith("referenced in "):
                counts["referenced_in_marker_rows"] += 1
            if action == "finished searching libraries":
                counts["finished_search_marker_rows"] += 1
            search = SEARCHING.fullmatch(line)
            if search:
                active = search[1].strip().strip('"')
                counts["library_search_rows"] += 1
                continue
            if action.startswith("searching ") or action == "finished searching libraries":
                active = None
            selected = LOADED.fullmatch(line)
            if selected is not None:
                counts["loaded_parenthesized_format_rows"] += 1
            else:
                selected = LOADED_COLON.fullmatch(line)
                if selected is not None:
                    counts["loaded_colon_format_rows"] += 1
                else:
                    if action.startswith("loaded "):
                        counts["unattributed_loaded_rows"] += 1
                    continue
            loaded, member = selected.groups()
            # For bare "Loaded X.lib(member)" use the immediately preceding
            # Searching row only when archive basenames also agree.
            path = canonical_windows(loaded)
            if path is None and active and basename(loaded) == basename(active):
                path = canonical_windows(active)
            origin = root_kind(path, roots) if path else "unresolved"
            label = family(loaded, registry, origin)
            name = member_name(member)
            if label is None or name is None:
                counts["unattributed_loaded_rows"] += 1
                continue
            # Known archive names may be recognized without a path, but never
            # attribute an unapproved individual container to the MSVC SDK.
            rows[(*label, name, origin)] += 1
            counts["attributed_loaded_rows"] += 1
            if origin != "unresolved":
                instances[(*label, origin)].add(path)
                if len(hashes) >= MAX_ROWS and path not in hashes:
                    raise ValueError("MSVC trace refers to too many distinct private archives")
                if path not in hashes:
                    hashes[path] = archive_hash(path, roots[origin])
            if len(rows) > MAX_ROWS:
                raise ValueError("MSVC trace has too many distinct selected member observations")
    return rows, counts, instances, hashes


def check_toolchain(values):
    result = {}
    for key, value in values.items():
        if value is None:
            continue
        if key == "vs_product_id":
            if value not in VS_PRODUCTS:
                raise ValueError("Visual Studio product identity is not recognized")
        elif not NUMERIC_VERSION.fullmatch(value):
            raise ValueError("toolchain version is not a bounded numeric version")
        result[key] = value
    return result


def build_report(exe, expected, commit, map_data, verbose, stderr, roots, toolchain=None, cargo_lock=None):
    registry = lock_packages(cargo_lock)
    pe = pe_identity(exe, expected)
    mapped, map_counts = parse_map(decode(map_data), pe, registry)
    stdout_text, stdout_encoding = decode_with_encoding(verbose)
    stderr_text, stderr_encoding = decode_with_encoding(stderr)
    loaded, verbose_counts, instances, hashes = parse_verbose([stdout_text, stderr_text], roots, registry)
    seen_labels = {(archive, member) for archive, _, member, _ in loaded}
    different_roots = defaultdict(set)
    for archive, _, member, origin in loaded:
        different_roots[archive, member].add(origin)
    map_rows = [
        {"family": archive, "kind": kind, "member": member, "table": table, "symbol_rows": count,
         "also_observed_loaded": (archive, member) in seen_labels,
         "multiple_verbose_origins": len(different_roots[archive, member]) > 1}
        for (archive, kind, member, table), count in sorted(mapped.items())
    ]
    verbose_rows = [
        {"family": archive, "kind": kind, "member": member, "origin": origin, "loaded_rows": count}
        for (archive, kind, member, origin), count in sorted(loaded.items())
    ]
    containers = [
        {"family": archive, "kind": kind, "origin": origin, "distinct_paths_observed": len(paths),
         "sha256": sorted({hashes[path] for path in paths if hashes[path] is not None}),
         "unhashed_path_count": sum(hashes[path] is None for path in paths)}
        for (archive, kind, origin), paths in sorted(instances.items())
    ]
    observed = {row["family"] for row in map_rows + verbose_rows if row["kind"] in {"cargo-registry-archive", "rust-or-cargo-archive"}}
    return {
        "schema_version": 1,
        "cargo_lock_sha256": sha(cargo_lock) if cargo_lock is not None else None,
        "observed_cargo_registry_candidates": [
            {"family": name, "possible_packages": [
                {"name": package, "version": version} for package, version in sorted(registry[name])
            ]}
            for name in sorted(observed)
        ],
        "source_commit": commit, "exe_sha256": expected,
        "map_sha256": sha(map_data), "verbose_stdout_sha256": sha(verbose),
        "verbose_stderr_sha256": sha(stderr) if stderr else None,
        "map_header_matches_exe": True,
        "toolchain": check_toolchain(toolchain or {}),
        "map_observations": {
            "status": "observed" if map_rows else "unresolved",
            "attributed_symbol_rows": map_counts["attributed_symbol_rows"],
            "unattributed_symbol_rows": map_counts["unattributed_symbol_rows"],
            "rows": map_rows,
        },
        "verbose_selections": {
            "status": "observed" if verbose_rows else "unresolved",
            "trace_streams": {
                "stdout": {"byte_count": len(verbose), "encoding": stdout_encoding,
                           "nul_byte_count": verbose.count(0)},
                "stderr": {"byte_count": len(stderr), "encoding": stderr_encoding,
                           "nul_byte_count": stderr.count(0)},
            },
            "trace_marker_counts": {key: verbose_counts[key] for key in (
                "decoded_lines", "ansi_color_sequences_removed", "searching_literal_rows",
                "loaded_literal_rows", "found_symbol_marker_rows", "referenced_in_marker_rows",
                "finished_search_marker_rows", "loaded_parenthesized_format_rows",
                "loaded_colon_format_rows",
            )},
            "library_search_rows": verbose_counts["library_search_rows"],
            "attributed_loaded_rows": verbose_counts["attributed_loaded_rows"],
            "unattributed_loaded_rows": verbose_counts["unattributed_loaded_rows"],
            "rows": verbose_rows,
        },
        "recognized_containers": containers,
        "scope": "Allowlisted technical families only. MAP symbol observations and verbose linker Loaded "
                 "rows represent different evidence. Matching names do not prove archive identity. "
                 "The caller must supply logs from the same final link invocation. Unknown rows, "
                 "LTO, symbol stripping, identical COMDAT folding and unused-code elimination limit "
                 "coverage. Cargo.lock matches are name candidates, not observed versions or proof "
                 "that a registry supplied that object. SQLite classification covers known archive "
                 "basenames, not its version or build options. No exhaustive byte or legal redistribution conclusion.",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--map", required=True)
    parser.add_argument("--verbose", required=True)
    parser.add_argument("--verbose-stderr")
    parser.add_argument("--cargo-lock")
    parser.add_argument("--root", default=[], action="append", metavar="KIND=PATH")
    toolchain_fields = ("linker_version", "cl_version", "vc_toolset_version",
                        "sdk_version", "vs_version", "vs_product_id")
    for key in toolchain_fields:
        parser.add_argument("--" + key.replace("_", "-"))
    args = parser.parse_args(argv)
    try:
        report = build_report(
            bounded(args.exe), exact_hex(args.exe_sha256, 64), exact_hex(args.commit, 40),
            bounded(args.map), bounded(args.verbose),
            bounded(args.verbose_stderr) if args.verbose_stderr else b"", parse_roots(args.root),
            {key: getattr(args, key) for key in toolchain_fields},
            bounded(args.cargo_lock, 4 << 20) if args.cargo_lock else None,
        )
    except ValueError as error:
        # Do not publish exception text derived from private file content.
        safe = {
            "invalid evidence digest or source identity",
            "link evidence exceeds the permitted input size",
            "approved roots must use distinct known Windows root kinds",
            "approved source roots must be distinguishable",
            "unsupported Cargo.lock structure",
            "unexpected crates.io identity in Cargo.lock",
            "the supplied executable does not match the pinned PE digest",
            "candidate PE header is invalid",
            "candidate is not a valid x64 PE32+ image",
            "candidate has an empty PE image",
            "MAP timestamp or image base does not match the exact PE",
            "MAP evidence has too many distinct retained observations",
            "MSVC trace refers to too many distinct private archives",
            "MSVC trace has too many distinct selected member observations",
            "Visual Studio product identity is not recognized",
            "toolchain version is not a bounded numeric version",
        }
        detail = str(error)
        parser.exit(2, f"MSVC link evidence could not be verified. {detail if detail in safe else 'Invalid data.'}\n")
    except (OSError, struct.error):
        parser.exit(2, "MSVC link evidence could not be read or verified.\n")
    print(json.dumps(report, ensure_ascii=True, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
