CREATE TABLE note_revision_state (
    revision_id INTEGER PRIMARY KEY REFERENCES note_revisions(id) ON DELETE CASCADE,
    properties_json TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE note_revision_attachments (
    revision_id INTEGER NOT NULL REFERENCES note_revision_state(revision_id) ON DELETE CASCADE,
    original_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('pdf','png','jpeg','text','markdown')),
    content BLOB NOT NULL CHECK(length(content) BETWEEN 1 AND 8388608),
    origin TEXT,
    PRIMARY KEY(revision_id,original_id)
) WITHOUT ROWID;

-- A view gives text saves and explicit checkpoint calls one capture operation.
CREATE VIEW capture_note_history AS SELECT id AS note_id,version,title,body FROM notes WHERE 0;
CREATE TRIGGER write_note_history INSTEAD OF INSERT ON capture_note_history
BEGIN
    INSERT INTO note_revisions(note_id,prior_version,title,body,snapshot_bytes)
    VALUES(NEW.note_id,NEW.version,NEW.title,NEW.body,
           length(CAST(NEW.title AS BLOB))+length(CAST(NEW.body AS BLOB)));

    INSERT INTO note_revision_state(revision_id,properties_json)
    SELECT last_insert_rowid(), json_group_object(key,value)
    FROM note_properties WHERE note_id=NEW.note_id;

    INSERT INTO note_revision_attachments(revision_id,original_id,name,kind,content,origin)
    SELECT last_insert_rowid(),a.id,a.name,a.kind,a.content,o.relative_path
    FROM attachments a LEFT JOIN attachment_origins o ON o.attachment_id=a.id
    WHERE a.note_id=NEW.note_id;

    UPDATE note_revisions SET snapshot_bytes=snapshot_bytes
        +(SELECT length(CAST(properties_json AS BLOB)) FROM note_revision_state WHERE revision_id=last_insert_rowid())
        +(SELECT coalesce(sum(length(content)+length(CAST(name AS BLOB))+length(kind)
            +coalesce(length(CAST(origin AS BLOB)),0)),0)
          FROM note_revision_attachments WHERE revision_id=last_insert_rowid())
    WHERE id=last_insert_rowid();

    DELETE FROM note_revisions WHERE note_id=NEW.note_id AND id < (
        SELECT id FROM note_revisions WHERE note_id=NEW.note_id ORDER BY id DESC LIMIT 1 OFFSET 29
    );
    DELETE FROM note_revisions WHERE id < (
        SELECT id FROM note_revisions ORDER BY id DESC LIMIT 1 OFFSET 2047
    );
    -- Allow one maximum-sized note, plus some recent history. Charge stored
    -- BLOBs, JSON and raw text to this budget even when editions repeat bytes.
    DELETE FROM note_revisions WHERE id IN (
        SELECT id FROM (
            SELECT id,sum(snapshot_bytes) OVER (ORDER BY id DESC ROWS UNBOUNDED PRECEDING) AS total
            FROM note_revisions
        ) WHERE total>201326592
    );
END;

DROP TRIGGER record_prior_note_revision;
CREATE TRIGGER record_prior_note_revision AFTER UPDATE OF title,body ON notes
WHEN OLD.title <> NEW.title OR OLD.body <> NEW.body
BEGIN
    INSERT INTO capture_note_history(note_id,version,title,body)
    VALUES(OLD.id,OLD.version,OLD.title,OLD.body);
END;
