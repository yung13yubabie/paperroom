#!/usr/bin/env python3
"""Build or verify a Windows QA draft containing the byte-checked notice subset."""
import argparse
import hashlib
from html.parser import HTMLParser
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import tempfile
from urllib.parse import urlsplit
from zipfile import BadZipFile, ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo

_helper = importlib.util.spec_from_file_location(
    "notice_bundle", Path(__file__).with_name("verify-windows-notice-bundle.py"),
)
notice = importlib.util.module_from_spec(_helper)
_helper.loader.exec_module(notice)

EXE = notice.EXE
GUIDE = notice.GUIDE
INDEX = notice.INDEX
RUST_HTML = notice.RUST_NOTICE
RUST_LINK_ROOT = "licenses/rust-toolchain/"
PROVENANCE = "evidence/paperroom-msvc-provenance.json"
DEPENDENCY_INVENTORY = "evidence/dependency-inventory.tsv"
LINK_INVENTORY = "evidence/msvc-link-inventory.json"
REGISTRY_INDEX = "evidence/registry-notices.json"
REGISTRY_ROOT = "licenses/crates-io/"
REGISTRY_COMPONENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.+-]{0,95}\Z", re.ASCII)
REGISTRY_FIELDS = ("name", "version", "roles", "license_expression", "license_file_declared", "license_file_basename")
DRAFT = "windows-qa-draft-not-release"
RUST_RELEASE = "1.100.0"
DEPENDENCY_HEADER = (
    b"crate\tversion\tlock_source\tmanifest_license\trepository\twindows_role\t"
    b"notice_in_v011_zip\tregistry_license_files"
)
RUST_LINK = re.compile(r"licenses/([A-Za-z0-9][A-Za-z0-9_.-]{0,88}\.txt)\Z", re.ASCII)
PDF_NOTICES = {f"licenses/pdf-assets/{name}": digest for name, digest in notice.EMBEDDED.items()}
FIXED = {EXE, GUIDE, INDEX, PROVENANCE, DEPENDENCY_INVENTORY, LINK_INVENTORY, REGISTRY_INDEX, RUST_HTML, *PDF_NOTICES}
REQUIRED = FIXED - {LINK_INVENTORY, REGISTRY_INDEX}


class RustNoticeLinks(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.files = set()

    def handle_starttag(self, tag, attrs):
        if tag == "base":
            raise ValueError("Rust notice HTML rewrites the base URL")
        for name, value in attrs:
            if name != "href":
                continue
            if not value:
                raise ValueError("Rust notice HTML has an empty link destination")
            if value.startswith("#"):
                continue
            parts = urlsplit(value)
            if parts.scheme.casefold() in {"http", "https"} and parts.netloc:
                continue
            href = value.removeprefix("./")
            if not RUST_LINK.fullmatch(href) or not notice.notice_path(href):
                raise ValueError("Rust notice HTML has an unsupported relative or non-web link")
            self.files.add(href)


def relative_links(raw):
    parser = RustNoticeLinks()
    parser.feed(raw.decode("utf-8-sig"))
    parser.close()
    if len(parser.files) > notice.MAX_MEMBERS - len(FIXED):
        raise ValueError("Rust notice HTML references too many local files")
    if len({item.casefold() for item in parser.files}) != len(parser.files):
        raise ValueError("Rust notice HTML has case-conflicting local links")
    return sorted(parser.files)


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode("utf-8")


def identity(commit, version, exe_sha, provenance_sha, dependency_sha, link_sha=None, registry_sha=None):
    if not isinstance(version, str) or re.fullmatch(
        r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version
    ) is None or len(version) > 64:
        raise ValueError("invalid Paperroom version")
    return {
        "artifact_kind": DRAFT,
        "source_commit": notice.exact_digest(commit, 40),
        "product_version": version,
        "target": notice.TARGET,
        "exe_sha256": notice.exact_digest(exe_sha),
        "provenance_sha256": notice.exact_digest(provenance_sha),
        "dependency_inventory_sha256": notice.exact_digest(dependency_sha),
        "link_inventory_sha256": None if link_sha is None else notice.exact_digest(link_sha),
        "registry_notice_index_sha256": None if registry_sha is None else notice.exact_digest(registry_sha),
    }


def guide(expected):
    link_path = f"- 建置連結清冊：`{LINK_INVENTORY}`（已移除本機路徑）\n" if expected["link_inventory_sha256"] else ""
    registry_path = (f"- crates.io 原始通知及查核範圍：`{REGISTRY_INDEX}`。\n"
                     if expected["registry_notice_index_sha256"] else "")
    return (
        "# Paperroom Windows x64 通知草稿（僅供 QA）\n\n"
        "這份 ZIP 供測試驗收使用。請勿當作正式 Release 或再散布套件。\n\n"
        f"- 紙間版本：`{expected['product_version']}`\n"
        f"- 來源提交：`{expected['source_commit']}`\n"
        f"- EXE SHA-256：`{expected['exe_sha256']}`\n"
        f"- Cargo 建置證據：`{PROVENANCE}`\n"
        f"- 鎖檔相依清冊：`{DEPENDENCY_INVENTORY}`\n"
        f"{link_path}{registry_path}"
        f"- [Rust {RUST_RELEASE} 標準函式庫通知]({RUST_HTML})與其可離線開啟的相對授權檔。\n"
        "- 內嵌 PDF 素材的四份原文通知位於 `licenses/pdf-assets/`。\n\n"
        "涵蓋範圍仍待確認。Cargo 相依資料與連結清冊提供建置證據。完整 crate 通知、實際進入最終 "
        "EXE 的物件及適用的 MSVC／Windows SDK 再散布條款仍需複核。電腦可離線核對本包的檔案，"
        "此結果不等於完成全部通知或取得再散布許可。包內未附 Microsoft 原始檔。\n"
    ).encode("utf-8")


def msvc_pe(data):
    if len(data) < 64 or not data.startswith(b"MZ"):
        return False
    header = int.from_bytes(data[0x3C:0x40], "little")
    return (64 <= header <= len(data) - 26 and data[header:header+6] == b"PE\0\0d\x86"
            and data[header+24:header+26] == b"\x0b\x02")


def require_exact(data, digest, label):
    if notice.sha(data) != digest:
        raise ValueError(f"{label} bytes differ from the expected SHA-256")


def validate_evidence(payload, expected):
    exe = payload[EXE]
    require_exact(exe, expected["exe_sha256"], "EXE")
    if not msvc_pe(exe):
        raise ValueError("expected a Windows x64 PE32+ executable")
    receipt = payload[PROVENANCE]
    packages, rust_sha = notice.verify_receipt(receipt, expected["provenance_sha256"], expected["exe_sha256"])
    data = notice.parse(receipt)
    if (data["packages"][data["root"]].get("version") != expected["product_version"]
            or data["rustc"].get("release") != RUST_RELEASE or rust_sha is None):
        raise ValueError("provenance differs from this product version or required Rust release")
    require_exact(payload[DEPENDENCY_INVENTORY], expected["dependency_inventory_sha256"], "Dependency inventory")
    if payload[DEPENDENCY_INVENTORY].partition(b"\n")[0].rstrip(b"\r") != DEPENDENCY_HEADER:
        raise ValueError("unrecognized lock-derived dependency inventory")
    link_sha = expected["link_inventory_sha256"]
    if link_sha is not None:
        raw = payload[LINK_INVENTORY]
        require_exact(raw, link_sha, "Link inventory")
        evidence = notice.parse(raw)
        if (type(evidence.get("schema_version")) is not int or evidence["schema_version"] != 1
                or notice.exact_digest(evidence["exe_sha256"]) != expected["exe_sha256"]
                or notice.exact_digest(evidence["source_commit"], 40) != expected["source_commit"]):
            raise ValueError("link inventory refers to another candidate or source")
        if (evidence.get("map_header_matches_exe") is not True
                or not isinstance(evidence.get("recognized_containers"), list)
                or not isinstance(evidence.get("map_observations", {}).get("rows"), list)
                or not isinstance(evidence.get("verbose_selections", {}).get("rows"), list)):
            raise ValueError("link inventory lacks the sanitized exact-build evidence shape")
        notice.exact_digest(evidence["map_sha256"])
        notice.exact_digest(evidence["verbose_stdout_sha256"])
        if evidence.get("verbose_stderr_sha256") is not None:
            notice.exact_digest(evidence["verbose_stderr_sha256"])
    for path, digest in PDF_NOTICES.items():
        raw = payload[path]
        require_exact(raw, digest, "Pinned PDF notice")
        lf = raw.replace(b"\r\n", b"\n")
        if raw not in exe and lf not in exe and lf.replace(b"\n", b"\r\n") not in exe:
            raise ValueError("the EXE lacks the complete pinned PDF notice")
    rust = payload[RUST_HTML]
    require_exact(rust, rust_sha, "Rust library copyright")
    return packages, rust_sha, relative_links(rust)


def is_registry_member(path):
    if not path.startswith(REGISTRY_ROOT):
        return False
    remainder = path[len(REGISTRY_ROOT):].split("/")
    return 3 <= len(remainder) <= 14 and all(
        REGISTRY_COMPONENT.fullmatch(part) and not notice.RESERVED.fullmatch(part) and not part.endswith(".")
        for part in remainder
    )


def registry_records(raw, receipt, expected):
    require_exact(raw, expected["registry_notice_index_sha256"], "Registry notice index")
    index = notice.parse(raw)
    if (set(index) != {"schema_version", "exe_sha256", "provenance_sha256", "cargo_lock_sha256", "target", "scope", "packages"}
            or type(index["schema_version"]) is not int or index["schema_version"] != 1
            or notice.exact_digest(index["exe_sha256"]) != expected["exe_sha256"]
            or notice.exact_digest(index["provenance_sha256"]) != expected["provenance_sha256"]
            or index["target"] != notice.TARGET or not isinstance(index["packages"], list)
            or not isinstance(index["scope"], str) or not 1 <= len(index["scope"]) <= 1024):
        raise ValueError("registry notice index refers to another build or scope")
    notice.exact_digest(index["cargo_lock_sha256"])
    all_packages = notice.parse(receipt)["packages"]
    eligible = {(pkg["name"], pkg["version"]): pkg for pkg in all_packages
                if pkg["source"] == "crates.io" and "target-normal" in pkg["roles"]}
    seen = set()
    files = {}
    folded = set()
    for package in index["packages"]:
        if set(package) != {*REGISTRY_FIELDS, "archive_sha256", "status", "notices"}:
            raise ValueError("registry notice record has unexpected fields")
        key = (package["name"], package["version"])
        current = eligible.get(key)
        if (key in seen or current is None or any(package[field] != current[field] for field in REGISTRY_FIELDS)
                or not isinstance(package["notices"], list)):
            raise ValueError("registry notice package differs from the lock-resolved target scope")
        seen.add(key)
        notice.exact_digest(package["archive_sha256"])
        if package["status"] != ("collected" if package["notices"] else "no-matching-notice"):
            raise ValueError("registry notice package has an inconsistent status")
        base = f"{REGISTRY_ROOT}{key[0]}/{key[1]}/"
        for item in package["notices"]:
            if set(item) != {"path", "sha256", "bytes"}:
                raise ValueError("registry file record has unexpected fields")
            path = item["path"]
            if (not is_registry_member(path) or not path.startswith(base) or path.casefold() in folded
                    or type(item["bytes"]) is not int or not 0 <= item["bytes"] <= notice.MAX_NOTICE):
                raise ValueError("registry notice path collides or escapes its source package")
            files[path] = (notice.exact_digest(item["sha256"]), item["bytes"])
            folded.add(path.casefold())
            if len(files) > notice.MAX_MEMBERS - len(FIXED):
                raise ValueError("registry notice files exceed the package entry bound")
    if seen != set(eligible):
        raise ValueError("registry notice index does not account for every resolved crates.io target package")
    return files


def members_expected(links, expected, registry):
    members = set(REQUIRED) - {INDEX}
    if expected["link_inventory_sha256"] is not None:
        members.add(LINK_INVENTORY)
    if expected["registry_notice_index_sha256"] is not None:
        members.add(REGISTRY_INDEX)
        members.update(registry)
    members.update(RUST_LINK_ROOT + item for item in links)
    return members


def manifest(payload, expected, rust_sha, links):
    return {
        "schema_version": 1,
        **expected,
        "notice_scope_complete": False,
        "rust_release": RUST_RELEASE,
        "rust_copyright_library_sha256": rust_sha,
        "rust_relative_license_links": links,
        "member_sha256": {name: notice.sha(raw) for name, raw in payload.items()},
    }


def is_rust_member(name):
    if not name.startswith(RUST_LINK_ROOT):
        return False
    path = name[len(RUST_LINK_ROOT):]
    return RUST_LINK.fullmatch(path) is not None and notice.notice_path(name)


def bounded_entries(archive):
    found = {}
    sizes = 0
    folded = set()
    infos = archive.infolist()
    if archive.comment or not 1 <= len(infos) <= notice.MAX_MEMBERS:
        raise ValueError("unsupported ZIP comment or member count")
    for entry in infos:
        path = entry.filename
        if (entry.orig_filename != path
                or path not in FIXED and not is_rust_member(path) and not is_registry_member(path)
                or path.casefold() in folded or entry.is_dir() or entry.extra or entry.comment
                or entry.flag_bits & 1 or entry.compress_type not in {ZIP_STORED, ZIP_DEFLATED}
                or stat.S_IFMT(entry.external_attr >> 16) not in {0, stat.S_IFREG}):
            raise ValueError("ZIP member is duplicated, unsupported, or unsafe")
        limit = (notice.MAX_EXE if path == EXE else notice.MAX_INDEX
                 if path in {GUIDE, INDEX, PROVENANCE, DEPENDENCY_INVENTORY, LINK_INVENTORY, REGISTRY_INDEX}
                 else notice.MAX_NOTICE)
        sizes += entry.file_size
        if entry.file_size > limit or sizes > notice.MAX_ZIP:
            raise ValueError("uncompressed ZIP members exceed their bounds")
        folded.add(path.casefold())
        found[path] = (entry, limit)
    if not REQUIRED <= found.keys():
        raise ValueError("ZIP misses required draft, executable, evidence, or notice members")
    return found


def verify_package(package, package_sha, index_sha, expected):
    package_sha = notice.exact_digest(package_sha)
    index_sha = notice.exact_digest(index_sha)
    with Path(package).open("rb") as stream:
        if os.fstat(stream.fileno()).st_size > notice.MAX_ZIP:
            raise ValueError("draft ZIP exceeds its size limit")
        if hashlib.file_digest(stream, "sha256").hexdigest() != package_sha:
            raise ValueError("draft ZIP SHA-256 differs from the expected download identity")
        stream.seek(0)
        with ZipFile(stream) as archive:
            entries = bounded_entries(archive)
            def read_member(name):
                entry, limit = entries[name]
                with archive.open(entry) as source:
                    return notice.bounded(source, limit)

            index = read_member(INDEX)
            require_exact(index, index_sha, "Notice index")
            html = read_member(RUST_HTML)
            links = relative_links(html)
            receipt = read_member(PROVENANCE)
            cached = {RUST_HTML: html, PROVENANCE: receipt}
            registry = {}
            if expected["registry_notice_index_sha256"] is not None:
                if REGISTRY_INDEX not in entries:
                    raise ValueError("ZIP omits the expected registry notice index")
                registry_raw = read_member(REGISTRY_INDEX)
                registry = registry_records(registry_raw, receipt, expected)
                cached[REGISTRY_INDEX] = registry_raw
            if set(entries) != members_expected(links, expected, registry) | {INDEX}:
                raise ValueError("ZIP contains unexpected members or unresolved Rust notice links")
            payload = {name: cached[name] if name in cached else read_member(name) for name in entries if name != INDEX}
            for name, (digest, size) in registry.items():
                if len(payload[name]) != size:
                    raise ValueError("ZIP registry notice length differs from collected original")
                require_exact(payload[name], digest, "Registry notice")
    packages, rust_sha, verified_links = validate_evidence(payload, expected)
    if payload[GUIDE] != guide(expected):
        raise ValueError("draft guide or unresolved-coverage statement changed")
    if index != json_bytes(manifest(payload, expected, rust_sha, verified_links)):
        raise ValueError("draft notice index does not match the exact file bytes and identities")
    return {
        **expected,
        "package_sha256": package_sha,
        "notice_index_sha256": index_sha,
        "rust_release": RUST_RELEASE,
        "rust_copyright_library_sha256": rust_sha,
        "rust_relative_license_count": len(links),
        "resolved_package_count": packages,
        "notice_scope_complete": False,
        "link_inventory_included": LINK_INVENTORY in payload,
        "registry_notice_file_count": len(registry),
        "file_count": len(payload) + 1,
    }


def document_file(root, name, maximum=notice.MAX_NOTICE, allow_empty=False):
    resolved = (root / name).resolve(strict=True)
    if not resolved.is_relative_to(root) or not resolved.is_file():
        raise ValueError("notice file does not stay inside its approved source directory")
    text = notice.read_file(resolved, maximum)
    if not text and not allow_empty:
        raise ValueError("notice target is empty")
    return text


def build_package(output, executable, provenance, dependency, rust_sysroot, expected, link_inventory=None,
                  registry_notice_root=None):
    output = Path(output)
    parent = output.parent.resolve(strict=True)
    target = parent / output.name
    if target.exists() or target.is_symlink():
        raise ValueError("an output already occupies the draft ZIP name")
    if (link_inventory is None) != (expected["link_inventory_sha256"] is None):
        raise ValueError("supply the sanitized link inventory together with its digest")
    if (registry_notice_root is None) != (expected["registry_notice_index_sha256"] is None):
        raise ValueError("supply the registry notice directory and its exact index digest together")
    docs = (Path(rust_sysroot) / "share/doc/rust").resolve(strict=True)
    payload = {
        EXE: notice.read_file(executable, notice.MAX_EXE),
        PROVENANCE: notice.read_file(provenance, notice.MAX_INDEX),
        DEPENDENCY_INVENTORY: notice.read_file(dependency, notice.MAX_INDEX),
        GUIDE: guide(expected),
        RUST_HTML: document_file(docs, "COPYRIGHT-library.html"),
    }
    if link_inventory is not None:
        payload[LINK_INVENTORY] = notice.read_file(link_inventory, notice.MAX_INDEX)
    root = notice.ROOT / "docs/third-party/pdf-assets"
    for name in notice.EMBEDDED:
        payload[f"licenses/pdf-assets/{name}"] = notice.read_file(root / name, notice.MAX_NOTICE)
    _, rust_sha, links = validate_evidence(payload, expected)
    running_size = sum(len(raw) for raw in payload.values())
    for name in links:
        raw = document_file(docs, name)
        running_size += len(raw)
        if running_size > notice.MAX_ZIP:
            raise ValueError("Rust notice links exceed the draft ZIP size bounds")
        payload[RUST_LINK_ROOT + name] = raw
    if registry_notice_root is not None:
        registry_root = Path(registry_notice_root).resolve(strict=True)
        registry_raw = document_file(registry_root, REGISTRY_INDEX, notice.MAX_INDEX)
        records = registry_records(registry_raw, payload[PROVENANCE], expected)
        running_size += len(registry_raw)
        payload[REGISTRY_INDEX] = registry_raw
        for name, (digest, size) in records.items():
            raw = document_file(registry_root, name, maximum=size, allow_empty=True)
            if len(raw) != size:
                raise ValueError("registry notice length differs from collected original")
            require_exact(raw, digest, "Registry notice")
            running_size += len(raw)
            if running_size > notice.MAX_ZIP:
                raise ValueError("registry notices exceed the draft ZIP size bounds")
            payload[name] = raw
    index = json_bytes(manifest(payload, expected, rust_sha, links))
    payload[INDEX] = index
    if len(index) > notice.MAX_INDEX or running_size + len(index) > notice.MAX_ZIP:
        raise ValueError("draft ZIP metadata exceeds its limits")
    with tempfile.NamedTemporaryFile(dir=parent, prefix=".paperroom-notices-", suffix=".zip", delete=False) as file:
        staging = Path(file.name)
    try:
        with ZipFile(staging, "w", allowZip64=False) as archive:
            for name, raw in sorted(payload.items()):
                entry = ZipInfo(name)
                entry.create_system = 3
                entry.external_attr = 0o100644 << 16
                archive.writestr(entry, raw, compress_type=ZIP_DEFLATED, compresslevel=6)
        with staging.open("rb") as file:
            package_sha = hashlib.file_digest(file, "sha256").hexdigest()
        verified = verify_package(staging, package_sha, notice.sha(index), expected)
        os.link(staging, target)
        return verified
    finally:
        staging.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)
    build = subparsers.add_parser("build", help="stage, verify and publish a new draft ZIP locally")
    verify = subparsers.add_parser("verify", help="verify a downloaded draft ZIP offline")
    for command in (build, verify):
        command.add_argument("--commit", required=True)
        command.add_argument("--version", required=True)
        command.add_argument("--exe-sha256", required=True)
        command.add_argument("--provenance-sha256", required=True)
        command.add_argument("--dependency-inventory-sha256", required=True)
        command.add_argument("--link-inventory-sha256")
    build.add_argument("--output", type=Path, required=True)
    build.add_argument("--exe", type=Path, required=True)
    build.add_argument("--provenance", type=Path, required=True)
    build.add_argument("--dependency-inventory", type=Path, required=True)
    build.add_argument("--link-inventory", type=Path)
    build.add_argument("--rust-sysroot", type=Path, required=True)
    build.add_argument("--registry-notice-root", type=Path)
    verify.add_argument("--registry-notice-index-sha256")
    verify.add_argument("--package", type=Path, required=True)
    verify.add_argument("--package-sha256", required=True)
    verify.add_argument("--notice-index-sha256", required=True)
    args = parser.parse_args(argv)
    try:
        if args.mode == "build":
            registry_sha = None if args.registry_notice_root is None else notice.sha(document_file(
                args.registry_notice_root.resolve(strict=True), REGISTRY_INDEX, notice.MAX_INDEX))
        else:
            registry_sha = args.registry_notice_index_sha256
        expected = identity(args.commit, args.version, args.exe_sha256, args.provenance_sha256,
                            args.dependency_inventory_sha256, args.link_inventory_sha256, registry_sha)
        if args.mode == "build":
            report = build_package(args.output, args.exe, args.provenance, args.dependency_inventory,
                                   args.rust_sysroot, expected, args.link_inventory, args.registry_notice_root)
        else:
            report = verify_package(args.package, args.package_sha256, args.notice_index_sha256, expected)
    except ValueError as error:
        if type(error) is ValueError:
            parser.exit(2, f"{error}\n")
        parser.exit(2, "Draft ZIP evidence is malformed.\n")
    except (AttributeError, KeyError, OSError, BadZipFile, RuntimeError, TypeError):
        parser.exit(2, "Draft ZIP evidence could not be read or verified.\n")
    print(json_bytes(report).decode("utf-8"), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
