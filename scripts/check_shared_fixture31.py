"""Read the exact synthetic workspace through one SQLite snapshot. Never writes the database."""
from contextlib import closing
from pathlib import Path
import json
import sqlite3
import sys

from check_unified_workspace import check, require


def witness(db, spec, scenario, stage):
    result = check(db, spec)
    ids = {role: identity[0] for role, identity in result["notes"].items()}
    sql = "SELECT n.version,n.body,(SELECT count(*) FROM note_revisions h WHERE h.note_id=n.id) FROM notes n WHERE n.id=?"
    result["rows"] = {key: db.execute(sql, (identifier,)).fetchone() for key, identifier in ids.items()}
    if scenario == "collection-bulk-edit":
        values = {
            key: db.execute("SELECT value FROM note_properties WHERE note_id=? AND key='Duration'",
                            (identifier,)).fetchall() for key, identifier in ids.items()
        }
        expected = {"exact": "8", "decoy": "1"} if stage == "before" else {"exact": "21", "decoy": "21"}
        require(all(values[k] == [(value,)] for k, value in expected.items()), "Selected rows have wrong typed values")
        require(all(not values[k] for k in ("other", "reserve", "canonical")), "An unselected or excluded row changed")
        mode = db.execute("SELECT id,keyword FROM collection_views WHERE collection_id=? AND name='批次場景'",
                          (result["field"][2],)).fetchall()
        require(len(mode) == 1 and mode[0][1] == "場景", "Named collection view lost its filter")
        result.update(values=values, view=mode)
    elif scenario == "native-board-transfer":
        boards = {}
        for key, title in (("source", "CopySource32"), ("destination", "CopyDestination32")):
            board_id, = db.execute("SELECT id FROM boards WHERE title=?", (title,)).fetchone()
            cards = db.execute("SELECT note_id,x,y FROM board_cards WHERE board_id=? ORDER BY note_id", (board_id,)).fetchall()
            edges = db.execute("SELECT from_note,to_note FROM board_edges WHERE board_id=? ORDER BY 1,2", (board_id,)).fetchall()
            boards[key] = dict(id=board_id, cards=cards, edges=edges)
        src, dst = boards["source"], boards["destination"]
        pair = tuple(sorted((ids["canonical"], ids["exact"])))
        require([n for n, _, _ in src["cards"]] == list(pair) and src["edges"] == [pair], "Wrong original layout")
        require(dst["edges"] == ([] if stage == "before" else [pair]), "Destination edges differ")
        require(dst["cards"] == ([] if stage == "before" else [(n,x,y+135) for n,x,y in src["cards"]]), "Wrong copied position")
        result["boards"] = boards
    elif scenario == "native-board-edge-navigation":
        all_boards = {}
        for name, role, members in [("EdgeFar", "exact", ["canonical","exact","decoy"]),
                                    ("EdgeOther", "decoy", ["canonical","decoy"])]:
            board, = db.execute("SELECT id FROM boards WHERE title=?", (name,)).fetchone()
            cards = db.execute("SELECT note_id,x,y FROM board_cards WHERE board_id=? ORDER BY note_id", (board,)).fetchall()
            edges = db.execute("SELECT from_note,to_note FROM board_edges WHERE board_id=? ORDER BY 1,2", (board,)).fetchall()
            require([row[0] for row in cards] == sorted(ids[r] for r in members), "Unexpected board members")
            require(edges == [tuple(sorted((ids["canonical"], ids[role])))], "Wrong edge endpoints")
            if name == "EdgeFar":
                require(any(note == ids["exact"] and x >= 4200 and y >= 3600 for note,x,y in cards),
                        "Target card did not move beyond the viewport")
            all_boards[name] = dict(id=board,cards=cards,edges=edges)
        result["boards"] = all_boards
    else:
        raise ValueError("Unexpected scenario")
    return result


if __name__ == "__main__":
    database, manifest, scenario, stage = sys.argv[1:]
    with closing(sqlite3.connect(Path(database).resolve(strict=True).as_uri()+"?mode=ro", uri=True)) as conn:
        conn.execute("PRAGMA query_only=ON")
        conn.execute("BEGIN")
        print(json.dumps(witness(conn, json.loads(Path(manifest).read_text(encoding="utf-8-sig")), scenario, stage),
                         sort_keys=True, ensure_ascii=True))
