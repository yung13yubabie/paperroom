import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from zipfile import ZipFile

SCRIPT = Path(__file__).with_name("windows-reviewed-notice-package.py")
spec = importlib.util.spec_from_file_location("reviewed_package", SCRIPT)
pack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pack)
COMMIT = "a0" * 20
VERSION = "0.1.2"


class ReviewedNoticePackage(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.doc = self.root / "share/doc/rust"
        (self.doc / "licenses").mkdir(parents=True)
        self.html = b"""<a href="#section">Section</a>
<a href="https://example.invalid">Remote text</a>
<a href="licenses/MIT.txt">Local MIT</a>
<a href="./licenses/Apache-2.0.txt">Local Apache</a>
<a href="licenses/MIT.txt">Same link</a>"""
        (self.doc / "COPYRIGHT-library.html").write_bytes(self.html)
        (self.doc / "licenses/MIT.txt").write_bytes(b"original MIT fixture")
        (self.doc / "licenses/Apache-2.0.txt").write_bytes(b"original Apache fixture")
        (self.doc / "licenses/unreferenced.txt").write_bytes(b"not in the notice")
        pe = bytearray(100)
        pe[:2] = b"MZ"
        pe[60:64] = (64).to_bytes(4, "little")
        pe[64:70] = b"PE\0\0d\x86"
        pe[88:90] = b"\x0b\x02"
        pdf_root = pack.notice.ROOT / "docs/third-party/pdf-assets"
        pe.extend(b"\0".join((pdf_root / name).read_bytes() for name in pack.notice.EMBEDDED))
        self.exe = self.root / "Paperroom.exe"
        self.exe.write_bytes(pe)
        self.exe_sha = pack.notice.sha(pe)
        self.receipt = self.root / "provenance.json"
        self.receipt.write_bytes(pack.json_bytes({
            "schema_version": 1, "exe_sha256": self.exe_sha, "target": pack.notice.TARGET,
            "rustc": {"host": pack.notice.TARGET, "release": pack.RUST_RELEASE},
            "root": 0,
            "packages": [{"name": "paperroom", "source": "workspace", "version": VERSION}],
            "rust_toolchain_notices": {"available": [{
                "filename": "COPYRIGHT-library.html", "sha256": pack.notice.sha(self.html),
            }]},
        }))
        self.dependency = self.root / "dependency-inventory.tsv"
        self.dependency.write_bytes(pack.DEPENDENCY_HEADER + b"\n")
        self.link = self.root / "sanitized-link.json"
        self.toolchain = {"vc_toolset_version": "14.44.35112", "sdk_version": "10.0.26100.0",
                          "linker_version": "14.44.35207.1", "cl_version": "19.44.35207.1"}
        self.link.write_bytes(pack.json_bytes({
            "schema_version": 1, "exe_sha256": self.exe_sha, "source_commit": COMMIT,
            "map_sha256": "01" * 32, "verbose_stdout_sha256": "02" * 32,
            "map_header_matches_exe": True, "recognized_containers": [],
            "map_observations": {"rows": [{"family": "libcmt", "member": "startup.obj", "symbol_rows": 2}]},
            "verbose_selections": {"rows": []}, "toolchain": self.toolchain,
        }))
        self.zip = self.root / "notice.zip"

    def expected(self, include_link=True):
        return pack.identity(COMMIT, VERSION, self.exe_sha, pack.notice.sha(self.receipt.read_bytes()),
                             pack.notice.sha(self.dependency.read_bytes()),
                             pack.notice.sha(self.link.read_bytes()) if include_link else None)

    def build(self, include_link=True):
        return pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, self.expected(include_link),
                                  self.link if include_link else None)

    def runtime(self):
        candidate = {"schema_version": 1, "source_commit": COMMIT, "exe_sha256": self.exe_sha,
                     "link_inventory_sha256": pack.notice.sha(self.link.read_bytes())}
        versions = {name: self.toolchain[name] for name in pack.TOOLCHAIN_IDENTITY}
        families = [
            {"family": name, "search": {"trace_search_rows": 0, "absolute_paths": 0, "unbound_rows": 0,
                                       "unapproved_paths": 0, "unreadable_approved_paths": 0},
             "archives": [], "map_members": []} for name in pack.CRT_FAMILIES
        ]
        selected = families[0]
        selected["search"].update(trace_search_rows=1, absolute_paths=1)
        selected["archives"] = [{
            "archive": "libcmt.lib", "origin": "msvc", "version": versions["vc_toolset_version"],
            "sha256": "10" * 32, "archive_members": 200, "members_without_normalized_obj_basename": 1,
        }]
        selected["map_members"] = [{
            "member": "startup.obj", "map_symbol_rows": 2, "observed_occurrences": 1,
            "status": pack.CRT_MATCH, "archive_sha256": "10" * 32,
        }]
        proof = {**candidate, "target": pack.notice.TARGET, "toolchain": versions, "families": families,
                 "scope": "Test-only archive correlation.", "map_sha256": "01" * 32,
                 "verbose_stdout_sha256": "02" * 32, "verbose_stderr_sha256": None}
        empty_stream = {"bytes": 0, "encoding": "utf-8-or-ansi", **dict.fromkeys(pack.PROBE_COUNTERS, 0)}
        probe = {
            **candidate, "toolchain": self.toolchain,
            "comparison": {
                "scope": pack.PROBE_SCOPE, "linker_version": "14.44.35207.1", "compiler_version": "19.44.35207.1",
                "librarian_version": "14.44.35207.1",
                "cases": [
                    {"case": name, "verbosity_flag": flag, "explicit_crt_arguments": explicit,
                     "canary_map_symbol_rows": 1, "crt_map_symbol_rows": 2, "synthetic_exe_sha256": "11" * 32,
                     "stdout": empty_stream, "stderr": empty_stream}
                    for name, (flag, explicit) in pack.PROBE_CASES.items()
                ],
            },
            "installed_terms": {
                **versions, "scope": pack.TERMS_SCOPE, "status": "installed version-specific metadata observed",
                "checked_location_count": 8, "observed": [
                    {"key": "sdk-version-license-rtf-1033", "bytes": 250, "sha256": "12" * 32,
                     **dict.fromkeys(pack.TERMS_MARKERS, False)}
                ],
            },
        }
        return proof, probe

    def runtime_files(self, proof, probe):
        files = {
            pack.CRT_PROOF: (self.root / "archive-proof.json", pack.json_bytes(proof)),
            pack.CRT_NOTICE: (self.root / "authored-terms.md", "# 資料來源\n本專案撰寫的查核紀錄。\n".encode()),
            pack.LINK_PROBE: (self.root / "link-probe.json", pack.json_bytes(probe)),
        }
        for path, content in files.values():
            path.write_bytes(content)
        expected = {
            **self.expected(),
            **{pack.RUNTIME_EVIDENCE[name][0]: pack.notice.sha(content) for name, (_, content) in files.items()},
        }
        return expected, files

    def build_runtime(self, expected, files):
        return pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, expected, self.link,
                                  crt_proof=files[pack.CRT_PROOF][0], crt_notice=files[pack.CRT_NOTICE][0],
                                  link_probe=files[pack.LINK_PROBE][0])

    def test_round_trip_binds_binary_toolchain_and_local_links(self):
        report = self.build()
        self.assertEqual(report["rust_relative_license_count"], 2)
        self.assertTrue(report["link_inventory_included"])
        self.assertFalse(report["notice_scope_complete"])
        with ZipFile(self.zip) as archive:
            self.assertEqual(archive.read(pack.RUST_HTML), self.html)
            self.assertEqual(archive.read(pack.RUST_LINK_ROOT + "licenses/MIT.txt"), b"original MIT fixture")
            self.assertEqual(archive.read(pack.RUST_LINK_ROOT + "licenses/Apache-2.0.txt"), b"original Apache fixture")
            self.assertNotIn(pack.RUST_LINK_ROOT + "licenses/unreferenced.txt", archive.namelist())
            self.assertEqual(archive.read(pack.PROVENANCE), self.receipt.read_bytes())
            self.assertEqual(archive.read(pack.DEPENDENCY_INVENTORY), self.dependency.read_bytes())
            self.assertEqual(archive.read(pack.LINK_INVENTORY), self.link.read_bytes())
        self.assertEqual(pack.verify_package(self.zip, report["package_sha256"], report["notice_index_sha256"],
                                             self.expected()), report)
        with self.assertRaisesRegex(ValueError, "output already"):
            self.build()
        self.assertEqual(list(self.root.glob(".paperroom-notices-*")), [])

    def test_download_digest_missing_link_changed_bytes_and_unsafe_members_fail(self):
        report = self.build(False)
        with self.assertRaisesRegex(ValueError, "expected download identity"):
            pack.verify_package(self.zip, "aa" * 32, report["notice_index_sha256"], self.expected(False))
        with ZipFile(self.zip) as archive:
            members = {entry.filename: archive.read(entry) for entry in archive.infolist()}
        mit = pack.RUST_LINK_ROOT + "licenses/MIT.txt"
        for changed, error in [
            ({name: raw for name, raw in members.items() if name != mit}, "unresolved Rust"),
            ({**members, mit: b"changed bytes"}, "file bytes and identities"),
            ({**members, "licenses/Rust-Toolchain/licenses/mit.txt": b"extra"}, "unsafe"),
            ({**members, "licenses/msvc/vcruntime.dll": b"extra"}, "unsafe"),
        ]:
            with ZipFile(self.zip, "w") as archive:
                for name, raw in changed.items():
                    archive.writestr(name, raw)
            with self.assertRaisesRegex(ValueError, error):
                pack.verify_package(self.zip, pack.notice.sha(self.zip.read_bytes()),
                                    report["notice_index_sha256"], self.expected(False))

    def test_build_requires_real_receipt_matched_html_and_installed_targets(self):
        (self.doc / "licenses/MIT.txt").unlink()
        with self.assertRaises(FileNotFoundError):
            self.build()
        self.assertFalse(self.zip.exists())
        (self.doc / "licenses/MIT.txt").write_bytes(b"original MIT fixture")
        (self.doc / "COPYRIGHT-library.html").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "Rust library copyright"):
            self.build()
        self.assertFalse(self.zip.exists())
        (self.doc / "COPYRIGHT-library.html").write_bytes(self.html)
        self.link.write_bytes(pack.json_bytes({"schema_version": 1, "source_commit": "ae" * 20,
                                               "exe_sha256": self.exe_sha}))
        with self.assertRaisesRegex(ValueError, "another candidate or source"):
            self.build()

    def test_rejects_unsafe_relative_link_spelling(self):
        for relative in ("../x.txt", "licenses/%2E%2E/x.txt", "licenses/NUL.txt", "licenses/example.txt?other",
                         "file:///C:/Windows/x.txt", "//arbitrary.example/license.txt"):
            with self.subTest(relative=relative), self.assertRaisesRegex(ValueError, "unsupported"):
                pack.relative_links(f'<a href="{relative}">text</a>'.encode())
        with self.assertRaisesRegex(ValueError, "base URL"):
            pack.relative_links(b'<base href="https://example.invalid/"><a href="licenses/MIT.txt">MIT</a>')
        with self.assertRaisesRegex(ValueError, "case-conflicting"):
            pack.relative_links(b'<a href="licenses/MIT.txt"></a><a href="licenses/Mit.txt"></a>')

    def test_registry_lane_checks_exact_reachable_scope_and_payload_bytes(self):
        receipt = pack.notice.parse(self.receipt.read_bytes())
        record = {
            "name": "crate_qa", "version": "1.2.3", "roles": ["host-build", "target-normal"],
            "license_expression": "MIT", "license_file_declared": True, "license_file_basename": "LICENSE.txt",
        }
        receipt["packages"].append({"source": "crates.io", **record})
        self.receipt.write_bytes(pack.json_bytes(receipt))
        root = self.root / "registry-notices"
        member_name = "licenses/crates-io/crate_qa/1.2.3/legal/LICENSE.txt"
        path = root / member_name
        path.parent.mkdir(parents=True)
        path.write_bytes(b"original legal fixture")
        registry_file = root / pack.REGISTRY_INDEX
        registry_file.parent.mkdir()
        manifest = {
            "schema_version": 1, "exe_sha256": self.exe_sha,
            "provenance_sha256": pack.notice.sha(self.receipt.read_bytes()),
            "cargo_lock_sha256": "aa" * 32, "target": pack.notice.TARGET, "scope": "Synthetic third-party scope",
            "packages": [{
                **record, "archive_sha256": "bb" * 32, "status": "collected",
                "notices": [{"path": member_name, "sha256": pack.notice.sha(path.read_bytes()), "bytes": path.stat().st_size}],
            }],
        }
        registry_file.write_bytes(pack.json_bytes(manifest))
        bound = {**self.expected(), "registry_notice_index_sha256": pack.notice.sha(registry_file.read_bytes())}
        found = pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound,
                                   self.link, root)
        self.assertEqual(found["registry_notice_file_count"], 1)
        self.assertEqual(pack.verify_package(self.zip, found["package_sha256"], found["notice_index_sha256"], bound), found)
        with ZipFile(self.zip) as archive:
            self.assertEqual(archive.read(member_name), b"original legal fixture")
        with self.assertRaisesRegex(ValueError, "unexpected members"):
            pack.verify_package(self.zip, found["package_sha256"], found["notice_index_sha256"], self.expected())
        self.zip.unlink()
        path.write_bytes(b"modified legal fixture")
        with self.assertRaisesRegex(ValueError, "Registry notice bytes"):
            pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound, self.link, root)
        self.assertFalse(self.zip.exists())
        manifest["packages"] = []
        registry_file.write_bytes(pack.json_bytes(manifest))
        bound["registry_notice_index_sha256"] = pack.notice.sha(registry_file.read_bytes())
        with self.assertRaisesRegex(ValueError, "does not account for every resolved"):
            pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound, self.link, root)

    def test_runtime_proof_exact_members_and_expected_digests(self):
        expected, files = self.runtime_files(*self.runtime())
        report = self.build_runtime(expected, files)
        with ZipFile(self.zip) as archive:
            for name, (_, raw) in files.items():
                self.assertEqual(archive.read(name), raw)
            index = pack.notice.parse(archive.read(pack.INDEX))
            self.assertEqual(set(archive.namelist()) & set(pack.RUNTIME_EVIDENCE), set(files))
            self.assertFalse(index["notice_scope_complete"])
            self.assertFalse(any(name.endswith((".lib", ".dll")) for name in archive.namelist()))
        self.assertEqual(pack.verify_package(self.zip, report["package_sha256"], report["notice_index_sha256"],
                                             expected), report)
        wrong = {**expected, "crt_notice_sha256": "aa" * 32}
        with self.assertRaisesRegex(ValueError, "CRT notice bytes differ"):
            pack.verify_package(self.zip, report["package_sha256"], report["notice_index_sha256"], wrong)
        with self.assertRaisesRegex(ValueError, "unexpected members"):
            pack.verify_package(self.zip, report["package_sha256"], report["notice_index_sha256"], self.expected())

    def test_runtime_source_trace_and_private_metadata_fail_without_zip(self):
        for modify, error in (
            (lambda proof, probe: proof.update(source_commit="0b" * 20), "another candidate"),
            (lambda proof, probe: proof.update(verbose_stdout_sha256="ff" * 32), "traces differ"),
            (lambda proof, probe: proof["families"][0]["map_members"].clear(), "exact link inventory MAP"),
            (lambda proof, probe: probe["installed_terms"]["observed"][0].update(key=r"C:\private\Redist.txt"),
             "unrecognized installed terms record"),
            (lambda proof, probe: probe["comparison"]["cases"][0].update(synthetic_exe_sha256=self.exe_sha),
             "separate canary"),
        ):
            with self.subTest(error=error):
                proof, probe = self.runtime()
                modify(proof, probe)
                expected, files = self.runtime_files(proof, probe)
                with self.assertRaisesRegex(ValueError, error):
                    self.build_runtime(expected, files)
                self.assertFalse(self.zip.exists())
        with self.assertRaisesRegex(ValueError, "require the exact sanitized link inventory"):
            pack.identity(COMMIT, VERSION, self.exe_sha, "ab" * 32, "ac" * 32, crt_proof_sha="ad" * 32)
        expected, files = self.runtime_files(*self.runtime())
        with self.assertRaisesRegex(ValueError, "CRT archive proof together"):
            pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, expected, self.link)
        self.assertFalse(self.zip.exists())

    def test_cli_paths_and_component_digests(self):
        bound = self.expected(False)
        same = ["--commit", COMMIT, "--version", VERSION, "--exe-sha256", self.exe_sha, "--provenance-sha256",
                bound["provenance_sha256"], "--dependency-inventory-sha256", bound["dependency_inventory_sha256"]]
        def cli(*args):
            return subprocess.run([sys.executable, "-B", str(SCRIPT), *args], capture_output=True, check=True, text=True)
        result = cli("build", *same, "--output", str(self.zip), "--exe", str(self.exe),
                     "--provenance", str(self.receipt), "--dependency-inventory", str(self.dependency),
                     "--rust-sysroot", str(self.root))
        report = pack.notice.parse(result.stdout)
        verified = cli("verify", *same, "--package", str(self.zip), "--package-sha256", report["package_sha256"],
                       "--notice-index-sha256", report["notice_index_sha256"])
        self.assertEqual(pack.notice.parse(verified.stdout), report)

    def test_cli_runtime_proof_and_notice_are_optional_hash_pairs(self):
        bound, files = self.runtime_files(*self.runtime())
        flags = {pack.CRT_PROOF: "--crt-proof", pack.CRT_NOTICE: "--crt-notice", pack.LINK_PROBE: "--link-probe"}
        both = ["--commit", COMMIT, "--version", VERSION, "--exe-sha256", self.exe_sha, "--provenance-sha256",
                bound["provenance_sha256"], "--dependency-inventory-sha256", bound["dependency_inventory_sha256"],
                "--link-inventory-sha256", bound["link_inventory_sha256"]]
        paths = []
        for name, (path, _) in files.items():
            flag = flags[name]
            both.extend((flag + "-sha256", bound[pack.RUNTIME_EVIDENCE[name][0]]))
            paths.extend((flag, str(path)))
        prefix = [sys.executable, "-B", str(SCRIPT)]
        build = subprocess.run(
            [*prefix, "build", *both, *paths, "--output", str(self.zip), "--exe", str(self.exe),
             "--provenance", str(self.receipt), "--dependency-inventory", str(self.dependency),
             "--link-inventory", str(self.link), "--rust-sysroot", str(self.root)],
            capture_output=True, text=True, check=True,
        )
        report = pack.notice.parse(build.stdout)
        verified = subprocess.run(
            [*prefix, "verify", *both, "--package", str(self.zip), "--package-sha256", report["package_sha256"],
             "--notice-index-sha256", report["notice_index_sha256"]],
            capture_output=True, text=True, check=True,
        )
        self.assertEqual(pack.notice.parse(verified.stdout), report)


if __name__ == "__main__":
    unittest.main()
