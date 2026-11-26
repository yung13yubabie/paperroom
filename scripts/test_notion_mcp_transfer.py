import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

import notion_mcp_transfer as transfer


def sample():
    pages = [{"id": digit * 32, "title": "同名", "markdown": body,
              "truncated": False, "unknown_block_count": 0, "unknown_block_ids": []}
             for digit, body in [("1", "文字甲 📝\r\nhttps://example.invalid/a"),
                                 ("2", "<callout>文字乙</callout>")]]
    return {"schema": transfer.SCHEMA, "confirmed_page_ids": [p["id"] for p in pages], "pages": pages}


def raw(document):
    return json.dumps(document, ensure_ascii=False).encode()


class TransferTests(unittest.TestCase):
    def test_cli_preserves_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "notes.zip"
            command = [sys.executable, str(Path(__file__).with_name("notion_mcp_transfer.py")), str(output)]
            doc = sample()
            source = raw(doc)
            run = subprocess.run(command, input=source, capture_output=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            packaged = output.read_bytes()
            self.assertEqual(packaged, transfer.build(source))
            with zipfile.ZipFile(io.BytesIO(packaged)) as archive:
                self.assertEqual(archive.read("paperroom-transfer/original.json"), source)
                receipt = json.loads(archive.read("paperroom-transfer/manifest.json"))
                self.assertEqual(receipt["original_sha256"], hashlib.sha256(source).hexdigest())
                self.assertEqual(len(archive.namelist()), 4)
                for page, record in zip(doc["pages"], receipt["pages"], strict=True):
                    member = archive.read(record["member"])
                    self.assertEqual(member.decode(), "# 同名\n\n" + page["markdown"])
                    self.assertEqual(record["member_sha256"], hashlib.sha256(member).hexdigest())
            again = subprocess.run(command, input=source, capture_output=True)
            self.assertNotEqual(again.returncode, 0)
            self.assertEqual(output.read_bytes(), packaged)

    def test_incomplete_or_unapproved_refused(self):
        for key,value in [("truncated",True),("unknown_block_count",False),("unknown_block_count",1),
                          ("unknown_block_ids",["x"]),("id","3"*32),("id","2"*32),
                          ("attachment_path","./image.png"),("title"," 請保留"),("markdown","a\0b")]:
            with self.subTest(key=key):
                doc = sample()
                doc["pages"][0][key] = value
                with self.assertRaises(ValueError):
                    transfer.build(raw(doc))

    def test_bounds_and_duplicate_json(self):
        for data in [b" " * (transfer.INPUT_LIMIT + 1), b'{"schema":1,"schema":2}', b"\xff"]:
            with self.assertRaises(ValueError):
                transfer.build(data)
        doc = sample()
        doc["pages"][0]["markdown"] = "a" * transfer.PAGE_LIMIT
        with self.assertRaises(ValueError):
            transfer.build(raw(doc))


if __name__ == "__main__":
    unittest.main()
