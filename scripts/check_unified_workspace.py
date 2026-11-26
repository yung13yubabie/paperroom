"""Read an existing synthetic Paperroom workspace in one SQLite snapshot.

Usage: python check_unified_workspace.py DB MANIFEST [PRIOR_RECEIPT]
Prints a stable identity receipt as JSON. Save and pass it again after toggling
the relation, reopening the workspace or restoring its full backup.
"""
import argparse
import hashlib
import json
import sqlite3
from contextlib import closing
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def exactly_one(db, statement, parameters, description):
    rows = db.execute(statement, parameters).fetchall()
    require(len(rows) == 1, f"{description}: expected one result, found {len(rows)}")
    return rows[0]


def check(db, spec):
    require(db.execute("PRAGMA integrity_check").fetchall() == [("ok",)], "integrity_check failed")
    require(not db.execute("PRAGMA foreign_key_check").fetchone(), "foreign_key_check failed")
    roles = {}
    for role, note in spec["notes"].items():
        note_id, token = exactly_one(db, """
            SELECT n.id, i.token FROM notes n JOIN note_link_identities i ON i.note_id=n.id
            WHERE n.trash_root IS NULL AND n.title=? AND instr(n.body,?)>0
            """, (note["title"], note["marker"]), role)
        require(bool(note["marker"]) and bool(token), "empty identity marker or token")
        roles[role] = [note_id, token]
    ids = [item[0] for item in roles.values()]
    require(len(ids) == len(set(ids)), "fixture roles resolved to the same note")
    attachments = {}
    for role, asset in spec["attachments"].items():
        asset_id, data = exactly_one(db,
            "SELECT id,content FROM attachments WHERE note_id=? AND name=?",
            (roles[role][0], asset["name"]), f"{role} attachment")
        digest = hashlib.sha256(data).hexdigest()
        require(digest == asset["sha256"], f"{role} attachment bytes differ")
        attachments[role] = [asset_id, digest]
    f = spec["relation"]
    field_id, source_collection, target_collection = exactly_one(db, """
        SELECT f.id,s.id,t.id FROM relation_fields f
        JOIN collections s ON s.id=f.source_collection JOIN collections t ON t.id=f.target_collection
        WHERE f.name=? AND s.name=? AND t.name=?
        """, (f["name"], f["source_collection"], f["target_collection"]), "scoped field")
    for cid, key in [(source_collection, "source_members"), (target_collection, "target_members")]:
        actual = sorted(row[0] for row in db.execute(
            "SELECT note_id FROM collection_notes WHERE collection_id=?", (cid,)))
        expected = sorted(roles[role][0] for role in f[key])
        require(actual == expected, f"unexpected {key}")
    actual = db.execute(
        "SELECT source_note,target_note FROM relation_field_values WHERE field_id=? ORDER BY 1,2",
        (field_id,)).fetchall()
    expected = sorted((roles[s][0], roles[t][0]) for s,t in f["edges"])
    require(actual == expected, "unexpected edge, missing edge or duplicate relationship")
    receipt = dict(notes=roles, attachments=attachments, field=[field_id,source_collection,target_collection])
    if "board" in spec:
        b = spec["board"]
        child, parent, tile, x, y = exactly_one(db, """
            SELECT c.id,p.id,o.id,o.x,o.y FROM board_objects o
            JOIN boards c ON c.id=o.child_id JOIN boards p ON p.id=o.board_id
            WHERE c.title=? AND p.title=?
            """, (b["child"], b["parent"]), "parent tile")
        placements = db.execute("""
            SELECT b.title,c.board_id,c.x,c.y FROM board_cards c
            JOIN boards b ON b.id=c.board_id WHERE c.note_id=? ORDER BY b.title
            """, (roles[b["role"]][0],)).fetchall()
        require(sorted(b["placements"]) == sorted(row[0] for row in placements),
                "canonical note has wrong board placement(s)")
        receipt["board"] = dict(child=child,parent=parent,tile=[tile,x,y],placements=placements)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("prior", type=Path, nargs="?")
    args = parser.parse_args()
    spec = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
    with closing(sqlite3.connect(args.database.resolve(strict=True).as_uri()+"?mode=ro", uri=True)) as db:
        db.execute("PRAGMA query_only=ON")
        db.execute("BEGIN")
        receipt = json.loads(json.dumps(check(db, spec)))
    if args.prior:
        prior = json.loads(args.prior.read_text(encoding="utf-8-sig"))
        for key in prior:
            require(receipt.get(key) == prior[key], f"identity or placement changed: {key}")
    print(json.dumps(receipt, ensure_ascii=True, sort_keys=True))


if __name__ == "__main__":
    main()

