import gzip
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


script = Path(__file__).with_name("windows-registry-notices.py")
spec = importlib.util.spec_from_file_location("windows_registry_notices", script)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
PACKAGE = "probe-1.2.3"
COPYRIGHT = b"Source copyright\r\nVerbatim bytes \xff\r\n"


def archive_bytes(package, entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as archive:
        for name, payload in entries:
            item = tarfile.TarInfo(f"{package}/{name}")
            if isinstance(payload, bytes):
                item.size = len(payload)
                archive.addfile(item, io.BytesIO(payload))
            else:
                item.type = tarfile.SYMTYPE
                item.linkname = "OTHER"
                archive.addfile(item)
    return gzip.compress(output.getvalue(), mtime=0)


def package_row(name, roles, source="crates.io", license_expression="MIT OR Apache-2.0", basename=None):
    return {"name": name, "version": "1.2.3", "source": source, "roles": roles,
            "license_expression": license_expression, "license_file_declared": basename is not None,
            "license_file_basename": basename}


class WindowsRegistryNoticesTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.folder = Path(directory.name)
        self.exe = self.folder / "candidate.exe"
        self.exe.write_bytes(b"MZ synthetic candidate")
        self.exe_sha = notices.sha(self.exe.read_bytes())
        self.provenance = self.folder / "candidate-provenance.json"
        self.lock = self.folder / "Cargo.lock"
        self.cargo = self.folder / "cargo"
        self.cache = self.cargo / "registry/cache/index.crates.io-1949cf8c6b5b557f"
        self.cache.mkdir(parents=True)
        self.output = self.folder / "collected"
        self.published = {
            "probe": archive_bytes(PACKAGE, [
                ("LICENSE-APACHE", b"Apache text\n"),
                ("vendor/module/notice.txt", b"Nested notice\n"),
                ("vendor/module/COPYRIGHT", COPYRIGHT),
                ("license.rs", b"source code is not a notice text\n"),
                ("unlicensed.txt", b"ignored"),
            ]),
            "silent": archive_bytes("silent-1.2.3", [("README.md", b"no matching notice here")]),
        }
        self.doc = {
            "schema_version": 1, "exe_sha256": self.exe_sha, "target": notices.TARGET,
            "rustc": {"host": notices.TARGET}, "root": 0, "edge_fields": notices.EDGE_FIELDS,
            "packages": [
                package_row("paperroom", ["target-normal"], source="workspace", license_expression=None),
                package_row("probe", ["target-normal", "host-build"]),
                package_row("silent", ["target-normal"], basename="legal-source.md"),
                package_row("hostonly", ["host-build"]),
                package_row("foreign", ["target-normal"], source="other-registry"),
            ],
            "edges": [
                [0, 1, "target-normal", "target-normal", "normal"],
                [0, 2, "target-normal", "target-normal", "normal"],
                [0, 3, "target-normal", "host-build", "build"],
                [0, 4, "target-normal", "target-normal", "normal"],
                [3, 1, "host-build", "host-build", "normal"],
            ],
        }

    def inputs(self):
        for name, data in self.published.items():
            (self.cache / f"{name}-1.2.3.crate").write_bytes(data)
        lock_rows = ["version = 4"]
        for name, data in self.published.items():
            lock_rows.extend(("", "[[package]]", f'name = "{name}"', 'version = "1.2.3"',
                              f'source = "{CRATES_IO}"', f'checksum = "{notices.sha(data)}"'))
        self.lock.write_text("\n".join(lock_rows), encoding="utf-8")
        self.provenance.write_text(json.dumps(self.doc), encoding="utf-8")

    def collect(self, *, exe_sha=None, provenance_sha=None):
        return notices.collect(
            self.exe, exe_sha or self.exe_sha, self.provenance,
            provenance_sha or notices.sha(self.provenance.read_bytes()),
            self.lock, self.cargo, self.output,
        )

    def test_collect_reachable_dual_role_notices_verbatim_and_report_absence(self):
        self.inputs()
        report = self.collect()
        packed = (self.output / notices.INDEX_PATH).read_bytes()
        self.assertEqual(json.loads(packed), report)
        self.assertNotIn(str(self.folder).encode(), packed)
        self.assertEqual(report["cargo_lock_sha256"], notices.sha(self.lock.read_bytes()))
        self.assertEqual([row["name"] for row in report["packages"]], ["probe", "silent"])
        active, missing = report["packages"]
        self.assertEqual(active["roles"], ["host-build", "target-normal"])
        self.assertEqual(active["archive_sha256"], notices.sha(self.published["probe"]))
        self.assertEqual(active["status"], "collected")
        files = {item["path"]: item for item in active["notices"]}
        self.assertEqual(len(files), 3)
        prefix = "licenses/crates-io/probe/1.2.3/"
        path = prefix + "vendor/module/COPYRIGHT"
        self.assertEqual(files[path], {"path": path, "sha256": notices.sha(COPYRIGHT), "bytes": len(COPYRIGHT)})
        self.assertEqual((self.output / path).read_bytes(), COPYRIGHT)
        self.assertEqual(missing["license_file_basename"], "legal-source.md")
        self.assertEqual(missing["status"], "no-matching-notice")
        self.assertEqual(missing["notices"], [])
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.collect()
        self.assertEqual((self.output / notices.INDEX_PATH).read_bytes(), packed)

    def test_preserves_allowlisted_reversed_license_names(self):
        data = archive_bytes(PACKAGE, [
            ("MIT-LICENSE", b"MIT text\r\n"),
            ("Apache-LICENSE", b"Apache text\n"),
            ("MIT-LICENSE.rs", b"source code"),
            ("custom-LICENSE", b"not allowlisted"),
        ])
        self.assertEqual(notices.notice_files(data, PACKAGE), [
            ("Apache-LICENSE", b"Apache text\n"),
            ("MIT-LICENSE", b"MIT text\r\n"),
        ])

    def test_locked_checksum_precedes_tar_read_and_rolls_back(self):
        self.inputs()
        (self.cache / f"{PACKAGE}.crate").write_bytes(b"not the specified archive")
        with patch.object(notices, "notice_files", side_effect=AssertionError("must not parse wrong bytes")):
            with self.assertRaisesRegex(ValueError, "matches the Cargo.lock SHA-256"):
                self.collect()
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.folder.glob(".registry-notices-*")), [])

    def test_selected_graph_must_follow_root_and_receipt_matches_exe(self):
        self.inputs()
        with self.assertRaisesRegex(ValueError, "EXE SHA-256 differs"):
            self.collect(exe_sha="a" * 64)
        with self.assertRaisesRegex(ValueError, "provenance SHA-256 differs"):
            self.collect(provenance_sha="b" * 64)
        self.doc["edges"].pop(1)
        self.inputs()
        with self.assertRaisesRegex(ValueError, "root reachability"):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_cannot_read_nonofficial_cache_or_duplicate_lock(self):
        self.inputs()
        nonofficial = self.cache.parent / "registry.example.invalid-1949cf8c6b5b557f"
        self.cache.rename(nonofficial)
        with self.assertRaisesRegex(ValueError, "official crates.io"):
            self.collect()
        self.assertFalse(self.output.exists())
        nonofficial.rename(self.cache)
        lock = self.lock.read_text(encoding="utf-8")
        self.lock.write_text(lock + lock[lock.find("\n[[package]]"):], encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate crates.io"):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_verified_invalid_archive_keeps_all_initial_output_unpublished(self):
        fixtures = [
            (archive_bytes("silent-1.2.3", [("../NOTICE", b"outside")]), "invalid member"),
            (archive_bytes("silent-1.2.3", [("LICENSE", None)]), "invalid member"),
            (archive_bytes("silent-1.2.3", [("LICENSE", b"one"), ("license", b"two")]), "paths collide"),
            (archive_bytes("silent-1.2.3", [("LICENSE", b"one"), ("LICENSE", b"two")]), "duplicate member"),
            (gzip.compress(gzip.decompress(archive_bytes("silent-1.2.3", [("NOTICE", b"x")]))[:1024]),
             "malformed end marker"),
        ]
        for raw, error in fixtures:
            with self.subTest(error=error):
                self.published["silent"] = raw
                self.inputs()
                with self.assertRaisesRegex(ValueError, error):
                    self.collect()
                self.assertFalse(self.output.exists())
                self.assertEqual(list(self.folder.glob(".registry-notices-*")), [])


if __name__ == "__main__":
    unittest.main()
