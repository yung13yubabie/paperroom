#!/usr/bin/env python3
"""Audit the exact GNU PE/link, collect matching notices and read back the ZIP."""

import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import struct
import subprocess
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

from windows_gnu_dependency_evidence import INDEX as DEPENDENCY_INDEX, live_rust_members, make_evidence

TARGET = "x86_64-pc-windows-gnu"
EXE = "Paperroom.exe"
MAX_EXE_BYTES = 24 << 20
INDEX = "evidence/gnu-candidate.json"
ROOT = Path(__file__).resolve().parents[1]
KNOWN_DLLS = {
    "advapi32.dll", "api-ms-win-core-synch-l1-2-0.dll", "bcrypt.dll", "bcryptprimitives.dll",
    "comctl32.dll", "comdlg32.dll", "crypt32.dll", "gdi32.dll", "iphlpapi.dll", "kernel32.dll",
    "msvcrt.dll", "ntdll.dll", "ole32.dll", "oleaut32.dll", "shell32.dll", "user32.dll",
    "userenv.dll", "version.dll", "ws2_32.dll",
}
MAP_INPUT = re.compile(r"\b0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(/[^\r\n]+)$", re.I)
INPUT_NAME = re.compile(r"[A-Za-z0-9_.+-]{1,200}\Z", re.ASCII)
HEX64 = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)


def digest(data):
    return sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, ensure_ascii=False, sort_keys=True) + "\n").encode()


def check_exe_size(size):
    if not 0 < size <= MAX_EXE_BYTES:
        raise ValueError(f"GNU candidate EXE size {size} exceeds the allowed range of 1–{MAX_EXE_BYTES} bytes")


def pe(exe, path):
    check_exe_size(len(exe))
    if not exe.startswith(b"MZ"):
        raise ValueError("Expected the Windows executable MZ signature")
    offset = struct.unpack_from("<I", exe, 0x3C)[0]
    header = offset + 24
    if exe[offset:offset+4] != b"PE\0\0" or struct.unpack_from("<H", exe, offset+4)[0] != 0x8664:
        raise ValueError("The candidate is not x64 PE")
    if struct.unpack_from("<H", exe, header)[0] != 0x20B:
        raise ValueError("The candidate is not PE32+")
    if struct.unpack_from("<I", exe, header+4)[0] == 0:
        raise ValueError("The PE has no code")
    if struct.unpack_from("<I", exe, header+108)[0] < 14:
        raise ValueError("The PE data directories are incomplete")
    directories = header + 112
    if any(struct.unpack_from("<II", exe, directories+8*4)):
        raise ValueError("Expected no Authenticode certificate table")
    if any(struct.unpack_from("<II", exe, directories+8*13)):
        raise ValueError("Delay-load imports require review")
    stamp = struct.unpack_from("<I", exe, offset+8)[0]
    if stamp != 0:
        raise ValueError("Expected a reproducible zero PE timestamp")
    parsed = subprocess.check_output(["objdump", "-p", str(path)], text=True)
    imports = sorted({name.lower() for name in re.findall(r"DLL Name:\s+(\S+)", parsed)})
    if not {"kernel32.dll", "msvcrt.dll"} <= set(imports) or set(imports) - KNOWN_DLLS:
        raise ValueError(f"Unexpected import set: {imports}")
    return {"machine": "x64", "format": "PE32+", "timestamp": stamp,
            "authenticode": "no certificate table", "delay_imports": [], "direct_dll_imports": imports}


def runtime_members(map_file, vendor, sysroot):
    text = map_file.read_text(encoding="utf-8", errors="strict")
    _, found, memory = text.partition("Linker script and memory map")
    if not found:
        raise ValueError("GNU ld did not emit the memory map")
    records = set()
    hashes = {}
    runtime_names = {"crt2.o", "crtbegin.o", "crtend.o", "libgcc_eh.a", "libmingwex.a", "libmingw32.a"}
    for line in memory.splitlines():
        match = MAP_INPUT.search(line)
        if not match or not int(match[1], 16) or not int(match[2], 16):
            continue
        raw = match[3].strip()
        file, sep, member = raw.partition("(")
        resolved = Path(file).resolve()
        name = resolved.name
        original_name = Path(file).name
        if original_name.lower() in {"libcmt.lib", "libvcruntime.lib", "libucrt.lib"}:
            raise ValueError("A Microsoft static runtime entered the GNU link")
        if resolved.is_relative_to(vendor):
            origin = "Debian GCC Runtime" if resolved.is_relative_to(vendor / "usr/lib/gcc") else "Debian MinGW-w64"
        elif resolved.is_relative_to(sysroot):
            origin = "Rust sysroot"
        else:
            continue
        member = member.removesuffix(")") if sep else ""
        if not INPUT_NAME.fullmatch(name) or member and not INPUT_NAME.fullmatch(member):
            raise ValueError("Unfamiliar object name in live runtime evidence")
        if not resolved.is_file():
            raise ValueError("A runtime input reported by ld cannot be verified")
        if resolved not in hashes:
            hashes[resolved] = digest(resolved.read_bytes())
        records.add((origin, name, member, hashes[resolved]))
    if not runtime_names <= {row[1] for row in records}:
        raise ValueError("Required GNU runtime objects were not observed in live sections")
    if not any(row[0] == "Rust sysroot" for row in records):
        raise ValueError("Expected compiled Rust runtime inputs")
    return [{"origin": a, "input": b, "member": c or None, "sha256": d}
            for a,b,c,d in sorted(records)]


def add_tree(entries, source, prefix=""):
    for path in sorted(source.rglob("*")):
        if path.is_file():
            entries[prefix + path.relative_to(source).as_posix()] = path.read_bytes()


def rust_licenses(entries, sysroot):
    helper_path = ROOT / "scripts/windows-reviewed-notice-package.py"
    spec = importlib.util.spec_from_file_location("shared_notice_parser", helper_path)
    parser = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parser)
    docs = sysroot / "share/doc/rust"
    standard = (docs / "COPYRIGHT-library.html").read_bytes()
    entries["licenses/rust-toolchain/COPYRIGHT-library.html"] = standard
    for name in parser.relative_links(standard):
        entries[f"licenses/rust-toolchain/{name}"] = (docs / name).read_bytes()


def build(work, sysroot, commit, output, permit_different_rebuild):
    if output.exists():
        raise ValueError("Use a fresh output path")
    exe_path = work / f"proof_a/out/{TARGET}/release/paperroom.exe"
    binary = exe_path.read_bytes()
    second_path = work / f"proof_b/out/{TARGET}/release/paperroom.exe"
    second = second_path.read_bytes()
    is_identical = binary == second
    if not is_identical and not permit_different_rebuild:
        raise ValueError("Two independent builds produced different EXE bytes")
    for candidate in (binary, second):
        for root in (work, work / "proof_a/src", work / "proof_b/src"):
            name = str(root)
            if name.encode() in candidate or name.encode("utf-16-le") in candidate:
                raise ValueError("An absolute build path entered the PE")
    stamp = pe(binary, exe_path)
    if pe(second, second_path) != stamp:
        raise ValueError("The independent rebuild changed PE dependency evidence")
    vendor = (work / "gnu").resolve()
    std = sysroot.resolve()
    members = runtime_members(work / "proof_a/final.map", vendor, std)
    if members != runtime_members(work / "proof_b/final.map", vendor, std):
        raise ValueError("Live runtime inputs differ between independent builds")

    entries = {EXE: binary}
    add_tree(entries, work / "registry")
    entries["evidence/debian-toolchain.txt"] = (work / "metadata/debian-packages.txt").read_bytes()
    entries["evidence/cargo-provenance.json"] = (work / "metadata/cargo-provenance.json").read_bytes()
    entries["evidence/gnu-runtime-members.json"] = json_bytes(members)
    for name in ("gcc.txt", "ld.txt", "rustc.txt"):
        entries[f"evidence/{name}"] = (work / "metadata" / name).read_bytes()

    vendors = vendor / "usr/share/doc"
    entries["licenses/gnu/mingw-w64-copyright.txt"] = (vendors / "mingw-w64-common/copyright").read_bytes()
    entries["licenses/gnu/gcc-mingw-w64-copyright.txt"] = (vendors / "gcc-mingw-w64-base/copyright").read_bytes()
    known = ROOT / "docs/third-party/gnu-runtime"
    for name in ("GCC-exception-3.1.txt", "GPL-3.0-or-later.txt"):
        entries["licenses/gnu/" + name] = (known / name).read_bytes()
    add_tree(entries, ROOT / "docs/third-party/pdf-assets", "licenses/pdf-assets/")
    rust_licenses(entries, sysroot)

    registry = json.loads(entries["evidence/registry-notices.json"])
    receipt = json.loads(entries["evidence/cargo-provenance.json"])
    exe_sha = digest(binary)
    if registry["target"] != TARGET or receipt["target"] != TARGET or registry["exe_sha256"] != exe_sha or receipt["exe_sha256"] != exe_sha:
        raise ValueError("Dependency evidence does not apply to this GNU EXE")
    draft_notes = ROOT / "docs/WINDOWS-GNU-DRAFT.md"
    audit_notes = draft_notes if draft_notes.is_file() else ROOT / "docs/WINDOWS-GNU-AUDIT.md"
    entries["GNU-AUDIT.md"] = audit_notes.read_bytes()
    observations = live_rust_members((work / "proof_a/final.map").read_text(encoding="utf-8"), exe_path.parent)
    artifact_messages = (work / "proof_a/cargo-artifacts.jsonl").read_text(encoding="utf-8")
    cargo_metadata = json.loads((work / "metadata/cargo.json").read_bytes())
    lock_raw = (work / "proof_a/src/Cargo.lock").read_bytes()
    evidence = make_evidence(entries, cargo_metadata, lock_raw, observations, artifact_messages,
                             commit, TARGET, exe_sha,
                             stamp["direct_dll_imports"])
    entries[DEPENDENCY_INDEX] = json_bytes(evidence)
    details = {
        "source_commit": commit, "target": TARGET, "exe_sha256": exe_sha,
        "independent_builds_match": 2 if is_identical else 0,
        "independent_builds_sha256": [exe_sha, digest(second)],
        "pe": stamp,
        "cargo_packages": len(receipt["packages"]), "target_registry_packages": len(registry["packages"]),
        "no_matching_registry_notice": [
            {"name": row["name"], "version": row["version"], "license_expression": row["license_expression"]}
            for row in registry["packages"] if not row["notices"]
        ],
        "notice_scope": "Original registry notices, Rust runtime, PDF assets and GNU toolchain. Release clearance not asserted.",
        "member_sha256": {p: digest(entries[p]) for p in sorted(entries)},
    }
    entries[INDEX] = json_bytes(details)
    with ZipFile(output, "x", compression=ZIP_DEFLATED, compresslevel=9) as archive:
        for name in sorted(entries):
            info = ZipInfo(name, (2020, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, entries[name], compress_type=ZIP_DEFLATED, compresslevel=9)
    zip_sha = digest(output.read_bytes())
    verify(output, commit, exe_sha, zip_sha, require_dependency_evidence=True)
    print(f"GNU candidate EXE {exe_sha} ZIP {zip_sha}")


def verify(path, commit, exe_sha, zip_sha, require_dependency_evidence=False):
    if not HEX64.fullmatch(exe_sha) or not HEX64.fullmatch(zip_sha):
        raise ValueError("Expected SHA-256 digests")
    if path.stat().st_size > 48 << 20 or digest(path.read_bytes()) != zip_sha:
        raise ValueError("ZIP exceeds bounds or its digest differs")
    with ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) > 1024 or len(names) != len(set(names)) or sum(i.file_size for i in archive.infolist()) > 64 << 20:
            raise ValueError("Invalid ZIP inventory")
        if len(names) != len({n.casefold() for n in names}) or any(
            not re.fullmatch(r"[a-zA-Z0-9_.+/-]+", n, re.ASCII)
            or any(part in {"", ".", ".."} or part.endswith(".") for part in n.split("/"))
            for n in names
        ):
            raise ValueError("The package contains an unsafe or ambiguous path")
        if any(re.search(r"(?i)\.(?:dll|lib|pdb|a|deb)$", n) for n in names):
            raise ValueError("The package contains a raw toolchain or runtime file")
        if EXE not in names:
            raise ValueError("The package does not contain Paperroom.exe")
        exe_size = archive.getinfo(EXE).file_size
        check_exe_size(exe_size)
        if archive.testzip() is not None:
            raise ValueError("A ZIP member has a CRC error")
        index = json.loads(archive.read(INDEX))
        if (index["source_commit"], index["target"], index["exe_sha256"]) != (commit, TARGET, exe_sha):
            raise ValueError("Package identity differs")
        match_count = index["independent_builds_match"]
        rebuild = index.get("independent_builds_sha256", [exe_sha, exe_sha])
        if (not isinstance(rebuild, list) or len(rebuild) != 2 or rebuild[0] != exe_sha
                or not all(isinstance(s, str) and HEX64.fullmatch(s) for s in rebuild)
                or match_count != (2 if rebuild[0] == rebuild[1] else 0)):
            raise ValueError("Invalid rebuild evidence")
        print("GNU rebuild is byte-identical." if match_count == 2
              else "GNU rebuild differs; this ZIP is a native test sample only.")
        expected = index["member_sha256"]
        if set(names) != {INDEX, *expected} or expected[EXE] != exe_sha:
            raise ValueError("Manifest inventory differs")
        for name, value in expected.items():
            if digest(archive.read(name)) != value:
                raise ValueError("File SHA-256 differs: " + name)
        if DEPENDENCY_INDEX in names:
            evidence = json.loads(archive.read(DEPENDENCY_INDEX))
            if (evidence["source_commit"], evidence["target"], evidence["exe_sha256"]) != (commit, TARGET, exe_sha):
                raise ValueError("Dependency evidence has a different source or EXE identity")
            if (evidence["cargo_provenance_sha256"] != expected["evidence/cargo-provenance.json"]
                    or evidence["registry_notices_sha256"] != expected["evidence/registry-notices.json"]
                    or evidence["runtime_link_inventory"]["sha256"] != expected["evidence/gnu-runtime-members.json"]):
                raise ValueError("Dependency evidence references different underlying receipts")
        elif require_dependency_evidence:
            raise ValueError("The package has no per-dependency redistribution evidence")
    print(f"Verified GNU ZIP readback, including all {len(names)} members and the {exe_size}-byte EXE.")


def describe_difference(first, second):
    def layout(path):
        binary = path.read_bytes()
        pe_start = struct.unpack_from("<I", binary, 0x3C)[0]
        count = struct.unpack_from("<H", binary, pe_start + 6)[0]
        optional_size = struct.unpack_from("<H", binary, pe_start + 20)[0]
        optional = pe_start + 24
        sections = optional + optional_size
        result = {}
        for at in range(sections, sections + 40 * count, 40):
            name = binary[at:at+8].split(b"\0", 1)[0].decode("ascii")
            size, start = struct.unpack_from("<II", binary, at + 16)
            result[name] = (size, start, digest(binary[start:start+size]))
        return binary, sections + 40 * count, result

    a, a_header, a_sections = layout(first)
    b, b_header, b_sections = layout(second)
    differences = [(name, a_sections.get(name, ())[:2], b_sections.get(name, ())[:2])
                   for name in sorted(a_sections.keys() | b_sections.keys())
                   if a_sections.get(name) != b_sections.get(name)]
    print(f"PE sizes {len(a)} / {len(b)}, header lengths {a_header} / {b_header}")
    print(f"Different header byte offsets: {[i for i, (x, y) in enumerate(zip(a[:a_header], b[:b_header])) if x != y][:48]}")
    print(f"Sections with size, offset or content differences: {differences}")
    for name, left, right in differences:
        if not left or not right:
            continue
        size_a, start_a = left
        size_b, start_b = right
        section_a = a[start_a:start_a+size_a]
        section_b = b[start_b:start_b+size_b]
        offsets = [i for i, pair in enumerate(zip(section_a, section_b)) if pair[0] != pair[1]]
        print(f"{name}: {len(offsets)} changed overlapping bytes, first offsets {offsets[:16]}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    jobs = parser.add_subparsers(dest="mode", required=True)
    b = jobs.add_parser("build")
    b.add_argument("--work", type=Path, required=True)
    b.add_argument("--sysroot", type=Path, required=True)
    b.add_argument("--output", type=Path, required=True)
    b.add_argument("--commit", required=True)
    b.add_argument("--permit-different-rebuild", action="store_true")
    d = jobs.add_parser("compare")
    d.add_argument("first", type=Path)
    d.add_argument("second", type=Path)
    v = jobs.add_parser("verify")
    v.add_argument("--package", type=Path, required=True)
    v.add_argument("--commit", required=True)
    v.add_argument("--exe-sha256", required=True)
    v.add_argument("--zip-sha256", required=True)
    v.add_argument("--require-dependency-evidence", action="store_true")
    a = parser.parse_args()
    if a.mode == "build":
        build(a.work, a.sysroot, a.commit, a.output, a.permit_different_rebuild)
    elif a.mode == "compare":
        describe_difference(a.first, a.second)
    else:
        verify(a.package, a.commit, a.exe_sha256, a.zip_sha256, a.require_dependency_evidence)


if __name__ == "__main__":
    main()
