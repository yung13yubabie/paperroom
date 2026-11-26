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


def reference_sample():
    doc = sample()
    doc["schema"] = transfer.REFERENCE_SCHEMA
    source, target = doc["confirmed_page_ids"]
    ref = {"source_page_id": source, "block_id": "3"*32, "block_parent_page_id": source,
           "block_type": "paragraph", "rich_text_type": "mention", "mention_type": "page",
           "target_page_id": target, "code": False}
    doc["confirmed_page_references"] = [ref, dict(ref), dict(ref, code=True),
                                       dict(ref, target_page_id="4"*32),
                                       dict(ref, mention_type="database")]
    return doc


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

    def test_reference_companion_preserves_originals_and_paths(self):
        doc = reference_sample()
        doc["pages"][0]["markdown"] += "\n`<mention url=... />`"
        doc["pages"][1]["title"] = "同名 ](http://example.invalid) ["
        v1 = {k: v for k, v in doc.items() if k != "confirmed_page_references"}
        v1["schema"] = transfer.SCHEMA
        with zipfile.ZipFile(io.BytesIO(transfer.build(raw(v1)))) as old, \
                zipfile.ZipFile(io.BytesIO(transfer.build(raw(doc)))) as archive:
            for page in doc["pages"]:
                name = "Page " + page["id"] + ".md"
                self.assertEqual(archive.read(name), old.read(name))
            self.assertEqual(archive.read("paperroom-transfer/original.json"), raw(doc))
            receipt = json.loads(archive.read("paperroom-transfer/manifest.json"))
            self.assertEqual([r["result"] for r in receipt["reference_records"]],
                ["listed", "already_listed", "unsupported_type_or_code",
                 "target_outside_confirmed_batch", "unsupported_type_or_code"])
            outside = doc["confirmed_page_references"][3]["target_page_id"]
            self.assertNotIn(outside, archive.read("paperroom-transfer/manifest.json").decode())
            self.assertIn(outside, archive.read("paperroom-transfer/original.json").decode())
            self.assertNotIn("target_page_id", receipt["reference_records"][3])
            [companion] = receipt["reference_companions"]
            source, target = doc["confirmed_page_ids"]
            self.assertEqual(companion["member"], f"Page {source}/Local page references.md")
            links = archive.read(companion["member"]).decode()
            self.assertEqual(links.count(f"](../Page%20{target}.md)"), 1)
            self.assertNotIn("](http://", links)
            self.assertEqual(hashlib.sha256(links.encode()).hexdigest(), companion["member_sha256"])

    def test_reference_evidence_rejects_mismatch_and_no_auto_discovery(self):
        doc = reference_sample()
        for fields in [dict(source_page_id="4"*32), dict(block_parent_page_id="4"*32),
                       dict(target_page_id="abc"), dict(code=0)]:
            with self.subTest(fields=fields):
                bad = dict(doc, confirmed_page_references=[dict(doc["confirmed_page_references"][0], **fields)])
                with self.assertRaises(ValueError):
                    transfer.build(raw(bad))
        doc["confirmed_page_references"] = []
        doc["pages"][0]["markdown"] = "<mention url=\"https://www.notion.so/" + "2"*32 + "\" />"
        with zipfile.ZipFile(io.BytesIO(transfer.build(raw(doc)))) as archive:
            self.assertEqual(len(archive.namelist()), 4)


    def test_typed_edges_require_confirmed_target_membership(self):
        doc = sample()
        a, b = doc["confirmed_page_ids"]
        doc.update(schema=transfer.RELATION_SCHEMA, confirmed_page_data_sources={a: "a"*32, b: "b"*32})
        edge = dict(source_page_id=a, property_name="取景", property_type="relation",
                    target_data_source_id="b"*32, target_page_id=b)
        doc["confirmed_database_relations"] = [edge]
        with zipfile.ZipFile(io.BytesIO(transfer.build(raw(doc)))) as archive:
            plan = json.loads(archive.read("paperroom-transfer/typed-relations-v1.json"))
            self.assertEqual(plan[2], [[a, "取景", "relation", "b"*32, b]])
            self.assertEqual(archive.read("paperroom-transfer/original.json"), raw(doc))
        for changes in [dict(target_page_id=a), dict(target_page_id="9"*32),
                        dict(target_data_source_id="c"*32), dict(property_type="url")]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                transfer.build(raw(dict(doc, confirmed_database_relations=[dict(edge, **changes)])))


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
