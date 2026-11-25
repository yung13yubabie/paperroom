#!/usr/bin/env python3
"""Verify a candidate and, optionally, its manually reviewed notice bundle."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
from zipfile import BadZipFile, ZipFile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
INDEX = "licenses/NOTICE-INDEX.json"
EXE = "Paperroom.exe"
GUIDE = "README.zh-Hant.md"
EMBEDDED = {
    "adobe-cmap.txt": "742665db9c8e1bc72603c6d319ca3e90b83bd6d95202f0cd4ef11068a07a9c29",
    "alloc-stdlib.txt": "c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae",
    "icc-cc0.txt": "469d87b5fa48f8aaf69e25f4fcac3a309c97dbb7d561b917638abdb640e004d9",
    "pdfium-foxit.txt": "b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7",
}
RUST_NOTICE = "licenses/rust-toolchain/COPYRIGHT-library.html"
ROLES = {
    "embedded-pdf-notice", "crate-notice", "rust-toolchain-notice",
    "msvc-notice", "other-reviewed-notice",
}
MARKER_FAMILIES = {"libcmt", "libucrt", "libvcruntime"}
MAX_EXE = 64 << 20
MAX_NOTICE = 16 << 20
MAX_ZIP = 128 << 20
MAX_INDEX = 2 << 20
MAX_MEMBERS = 512
SAFE_COMPONENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,95}", re.ASCII)
RESERVED = re.compile(r"(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", re.I | re.ASCII)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def exact_digest(value, digits=64):
    if not isinstance(value, str) or not re.fullmatch(rf"[0-9a-fA-F]{{{digits}}}", value):
        raise ValueError("invalid evidence identity")
    return value.lower()


def bounded(stream, limit):
    data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("evidence input exceeds its size limit")
    return data


def read_file(path, limit):
    with Path(path).open("rb") as stream:
        return bounded(stream, limit)


def unique_pairs(pairs):
    record = dict(pairs)
    if len(record) != len(pairs):
        raise ValueError("JSON contains repeated keys")
    return record


def parse(data):
    return json.loads(data, object_pairs_hook=unique_pairs)


def notice_path(name):
    if not isinstance(name, str):
        return False
    parts = name.split("/")
    return (
        2 <= len(parts) <= 5 and parts[0] == "licenses"
        and all(SAFE_COMPONENT.fullmatch(part) and not RESERVED.fullmatch(part)
                and not part.endswith(".") for part in parts)
    )


def verify_receipt(receipt_bytes, receipt_sha, exe_sha):
    if sha(receipt_bytes) != receipt_sha:
        raise ValueError("Cargo provenance does not match its recorded digest")
    receipt = parse(receipt_bytes)
    if (type(receipt.get("schema_version")) is not int or receipt["schema_version"] != 1
        or exact_digest(receipt["exe_sha256"]) != exe_sha or receipt["target"] != TARGET
        or receipt.get("rustc", {}).get("host") != TARGET):
        raise ValueError("Cargo provenance refers to another candidate or toolchain target")
    packages = receipt.get("packages")
    root = receipt.get("root")
    if (not isinstance(packages, list) or not 1 <= len(packages) <= 2048
        or type(root) is not int or not 0 <= root < len(packages)
        or packages[root].get("name") != "paperroom" or packages[root].get("source") != "workspace"):
        raise ValueError("Cargo provenance does not identify the Paperroom workspace root")
    notices = receipt.get("rust_toolchain_notices", {}).get("available", [])
    copyright_rows = [exact_digest(row["sha256"]) for row in notices if row.get("filename") == "COPYRIGHT-library.html"]
    if len(copyright_rows) > 1:
        raise ValueError("more than one Rust library notice occupies the exact known path")
    return len(packages), copyright_rows[0] if copyright_rows else None


def verify_embedded(exe, notice_root):
    evidence = {}
    for name, digest in EMBEDDED.items():
        raw = read_file(notice_root / name, MAX_NOTICE)
        if sha(raw) != digest:
            raise ValueError("a source notice does not match the pinned About text")
        lf = raw.replace(b"\r\n", b"\n")
        if raw in exe:
            evidence[name] = {"manifest_sha256": digest, "binary_encoding": "canonical UTF-8"}
        elif lf in exe or lf.replace(b"\n", b"\r\n") in exe:
            evidence[name] = {"manifest_sha256": digest, "binary_encoding": "UTF-8 with normalized line endings"}
        else:
            raise ValueError("the EXE lacks the complete literal text of a pinned About notice")
    return evidence


def zip_entries(archive):
    entries = {}
    folded = set()
    total = 0
    infos = archive.infolist()
    if archive.comment or not 1 <= len(infos) <= MAX_MEMBERS:
        raise ValueError("Windows ZIP comment or entry count is unsupported")
    for entry in infos:
        name = entry.filename
        if (entry.orig_filename != name or name not in {EXE, GUIDE} and not notice_path(name)
            or entry.is_dir() or entry.comment or entry.extra or entry.flag_bits & 1
            or stat.S_IFMT(entry.external_attr >> 16) not in (0, stat.S_IFREG)
            or name.casefold() in folded):
            raise ValueError("Windows ZIP contains an unsafe, duplicate, or unexpected entry")
        limit = MAX_EXE if name == EXE else MAX_INDEX if name in {INDEX, GUIDE} else MAX_NOTICE
        total += entry.file_size
        if entry.file_size > limit or total > MAX_ZIP:
            raise ValueError("Windows ZIP content exceeds evidence limits")
        folded.add(name.casefold())
        entries[name] = entry
    if not {EXE, GUIDE, INDEX} <= entries.keys():
        raise ValueError("Windows ZIP omits expected candidate, guide, or notice index")
    return entries


def checked_markers(value, exe_sha):
    if value is None:
        return {"status": "not supplied"}
    if (set(value) != {"exe_sha256", "map_sha256", "symbol_rows"}
        or exact_digest(value["exe_sha256"]) != exe_sha or set(value["symbol_rows"]) != MARKER_FAMILIES
        or any(type(count) is not int or not 0 <= count <= 10_000_000
               for count in value["symbol_rows"].values())):
        raise ValueError("recorded MSVC MAP markers do not match the expected candidate")
    return {
        "status": "externally recorded symbol rows; private MAP not examined",
        "map_sha256": exact_digest(value["map_sha256"]),
        "symbol_rows": value["symbol_rows"],
    }


def verify_windows_zip(path, index_sha, commit, exe_sha, receipt_sha, rust_sha):
    if Path(path).stat().st_size > MAX_ZIP:
        raise ValueError("Windows ZIP exceeds evidence limits")
    with ZipFile(path) as archive:
        entries = zip_entries(archive)
        index_raw = archive.read(entries[INDEX])
        if sha(index_raw) != index_sha:
            raise ValueError("shipped notice index differs from the reviewed digest")
        index = parse(index_raw)
        mandatory = {"schema_version", "source_commit", "exe_sha256", "provenance_sha256", "notices"}
        if (set(index) not in (mandatory, mandatory | {"link_markers"})
            or type(index["schema_version"]) is not int or index["schema_version"] != 1
            or exact_digest(index["source_commit"], 40) != commit
            or exact_digest(index["exe_sha256"]) != exe_sha
            or exact_digest(index["provenance_sha256"]) != receipt_sha):
            raise ValueError("reviewed notice index is not bound to this source, EXE and Cargo receipt")
        notices = {}
        for row in index["notices"]:
            if set(row) != {"path", "sha256", "role"}:
                raise ValueError("notice index record has unexpected fields")
            name = row["path"]
            if not notice_path(name) or name == INDEX or name in notices or row["role"] not in ROLES:
                raise ValueError("notice index contains an unsafe or repeated notice")
            notices[name] = (exact_digest(row["sha256"]), row["role"])
        have = set(entries) - {EXE, GUIDE, INDEX}
        if have != set(notices):
            raise ValueError("the shipped notice inventory differs from the reviewed index")
        required = {f"licenses/pdf-assets/{name}": (digest, "embedded-pdf-notice")
                    for name, digest in EMBEDDED.items()}
        if any(notices.get(name) != expected for name, expected in required.items()):
            raise ValueError("the reviewed inventory omits or misidentifies a pinned About notice")
        if {name for name, (_, role) in notices.items() if role == "embedded-pdf-notice"} != set(required):
            raise ValueError("unexpected notice declared to be embedded in the EXE")
        for name, (expected, _) in notices.items():
            if sha(archive.read(entries[name])) != expected:
                raise ValueError("shipped notice bytes do not match the reviewed digest")
        if sha(archive.read(entries[EXE])) != exe_sha:
            raise ValueError("the ZIP contains a different candidate EXE")
        if exe_sha.encode("ascii") not in archive.read(entries[GUIDE]).lower():
            raise ValueError("the ZIP guide omits the exact candidate EXE digest")
        if rust_sha is not None and RUST_NOTICE in notices and notices[RUST_NOTICE] != (rust_sha, "rust-toolchain-notice"):
            raise ValueError("a supplied canonical Rust notice differs from the provenance receipt")
        return {"status": "reviewed index and shipped bytes match",
                "notice_index_sha256": sha(index_raw), "listed_notice_count": len(notices),
                "rust_copyright_library_match": rust_sha is not None and RUST_NOTICE in notices,
                "link_markers": checked_markers(index.get("link_markers"), exe_sha)}



def stage_qa_zip(destination, exe, commit, exe_sha, receipt_sha, rust_sha, notice_root):
    parent = destination.parent.resolve(strict=True)
    temp_roots = [Path(tempfile.gettempdir()).resolve()]
    if os.environ.get("RUNNER_TEMP"):
        temp_roots.append(Path(os.environ["RUNNER_TEMP"]).resolve())
    if not any(parent.is_relative_to(root) for root in temp_roots):
        raise ValueError("QA ZIP output must be inside an existing temporary directory")
    destination = parent / destination.name
    if destination.exists() or destination.is_symlink():
        raise ValueError("QA ZIP output already exists")
    notices = []
    payload = {}
    for name, digest in EMBEDDED.items():
        text = read_file(notice_root / name, MAX_NOTICE)
        if sha(text) != digest:
            raise ValueError("a source notice does not match the pinned About text")
        member = f"licenses/pdf-assets/{name}"
        notices.append({"path": member, "role": "embedded-pdf-notice", "sha256": digest})
        payload[member] = text
    index = json.dumps({
        "schema_version": 1, "source_commit": commit, "exe_sha256": exe_sha,
        "provenance_sha256": receipt_sha, "notices": notices,
    }, indent=2, sort_keys=True).encode() + b"\n"
    guide = (
        "# Paperroom internal QA sample only\n\n"
        "Candidate packaging verification. Do not publish or redistribute this ZIP. "
        "These four supplementary notices do not document all dependencies, "
        "the Rust standard library, or MSVC redistribution terms.\n\n"
        f"Candidate Paperroom.exe SHA-256: {exe_sha}\n"
    ).encode()
    with tempfile.NamedTemporaryFile(dir=parent, prefix=".paperroom-qa-", suffix=".zip", delete=False) as temp:
        staged = Path(temp.name)
    try:
        with ZipFile(staged, "w") as archive:
            archive.writestr(EXE, exe)
            archive.writestr(GUIDE, guide)
            archive.writestr(INDEX, index)
            for member, text in payload.items():
                archive.writestr(member, text)
        info = verify_windows_zip(staged, sha(index), commit, exe_sha, receipt_sha, rust_sha)
        with staged.open("rb") as contents:
            info["windows_zip_sha256"] = hashlib.file_digest(contents, "sha256").hexdigest()
        # NTFS and other supported temporary filesystems publish this fully
        # checked file only when the requested name does not yet exist.
        os.link(staged, destination)
    finally:
        staged.unlink(missing_ok=True)
    return {"qa_only": True, "notice_scope_complete": False, **info}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--exe-sha256", required=True)
    parser.add_argument("--provenance", required=True, type=Path)
    parser.add_argument("--provenance-sha256", required=True)
    parser.add_argument("--commit", required=True)
    zip_mode = parser.add_mutually_exclusive_group()
    zip_mode.add_argument("--windows-zip", type=Path)
    zip_mode.add_argument("--stage-qa-zip", type=Path)
    parser.add_argument("--notice-index-sha256")
    args = parser.parse_args(argv)
    try:
        commit = exact_digest(args.commit, 40)
        exe_sha = exact_digest(args.exe_sha256)
        receipt_sha = exact_digest(args.provenance_sha256)
        exe = read_file(args.exe, MAX_EXE)
        if not exe.startswith(b"MZ") or sha(exe) != exe_sha:
            raise ValueError("the supplied EXE does not match the pinned candidate")
        count, rust_sha = verify_receipt(read_file(args.provenance, MAX_INDEX), receipt_sha, exe_sha)
        if (args.windows_zip is None) != (args.notice_index_sha256 is None):
            raise ValueError("Windows ZIP and reviewed notice-index digest must be supplied together")
        notice_root = ROOT / "docs" / "third-party" / "pdf-assets"
        embedded = verify_embedded(exe, notice_root)
        zip_info = {"status": "not supplied"}
        if args.stage_qa_zip is not None:
            zip_info = stage_qa_zip(args.stage_qa_zip, exe, commit, exe_sha, receipt_sha, rust_sha, notice_root)
        elif args.windows_zip is not None:
            zip_info = verify_windows_zip(args.windows_zip, exact_digest(args.notice_index_sha256),
                                          commit, exe_sha, receipt_sha, rust_sha)
        output = {
            "source_commit": commit, "exe_sha256": exe_sha,
            "scope": "Exact candidate and packaging evidence only. No redistribution or legal conclusion.",
            "cargo_resolution": {"receipt_sha256": receipt_sha, "resolved_package_count": count,
                                 "rust_library_notice_sha256": rust_sha, "linked_inventory": False},
            "embedded_notice_literals": embedded,
            "shipped_windows_zip": zip_info,
        }
    except ValueError as error:
        # Do not pass arbitrary JSON parser diagnostics through to a CI transcript.
        if type(error) is not ValueError:
            parser.exit(2, "Candidate notice evidence is malformed.\n")
        parser.exit(2, f"{error}\n")
    except (AttributeError, KeyError, OSError, BadZipFile, RuntimeError, TypeError):
        parser.exit(2, "Candidate notice evidence could not be read or verified.\n")
    print(json.dumps(output, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
