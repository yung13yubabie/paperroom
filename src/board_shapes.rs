//! Native board shapes. This file is an integration candidate; declare it under store.rs.
use super::{Notebook, Result};
use rusqlite::{Connection, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE board_shapes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('rectangle','ellipse')),
 x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
 y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000),
 width INTEGER NOT NULL CHECK(width BETWEEN 24 AND 2048),
 height INTEGER NOT NULL CHECK(height BETWEEN 24 AND 2048),
 version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0)
);
CREATE INDEX board_shapes_parent ON board_shapes(board_id,id);";
const MAX_SHAPES: i64 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShapeKind { Rectangle, Ellipse }

impl ShapeKind {
    fn key(self) -> &'static str {
        match self { Self::Rectangle => "rectangle", Self::Ellipse => "ellipse" }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "rectangle" => Ok(Self::Rectangle),
            "ellipse" => Ok(Self::Ellipse),
            _ => Err("無法辨識白板形狀。".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShapeRect {
    pub x: i32, pub y: i32, pub width: i32, pub height: i32,
}

impl ShapeRect {
    fn validate(self) -> Result<()> {
        if (0..=10000).contains(&self.x) && (0..=50000).contains(&self.y)
            && (24..=2048).contains(&self.width) && (24..=2048).contains(&self.height) {
            Ok(())
        } else {
            Err("白板形狀位置或尺寸超出範圍。".into())
        }
    }
    /// Convert pointer displacement at the starting DPI without changing size.
    pub(crate) fn moved_by_pointer(self, dx: i32, dy: i32, dpi: u32, max_x: i32) -> Self {
        let unit = |pixels| i64::from(pixels) * 96 / i64::from(dpi.max(1));
        let right = i64::from(max_x.max(self.x).clamp(0, 10_000));
        Self {
            x: (i64::from(self.x) + unit(dx)).clamp(0, right) as i32,
            y: (i64::from(self.y) + unit(dy)).clamp(0, 50_000) as i32,
            ..self
        }
    }

    pub fn hit(self, kind: ShapeKind, x: i32, y: i32) -> bool {
        let dx = i64::from(x) - i64::from(self.x);
        let dy = i64::from(y) - i64::from(self.y);
        let w = i64::from(self.width);
        let h = i64::from(self.height);
        if dx < 0 || dy < 0 || dx >= w || dy >= h { return false; }
        if kind == ShapeKind::Rectangle { return true; }
        // Use widened intermediates even for unvalidated caller geometry.
        let (x, y, w, h) = (i128::from(2*dx-w), i128::from(2*dy-h), i128::from(w), i128::from(h));
        x*x*h*h + y*y*w*w <= w*w*h*h
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shape {
    pub id: i64, pub kind: ShapeKind, pub rect: ShapeRect, pub version: i64,
}

fn create_shape_on(db: &Connection, board: i64, kind: ShapeKind, rect: ShapeRect) -> Result<i64> {
    rect.validate()?;
    let affected = db.execute(
        "INSERT INTO board_shapes(board_id,kind,x,y,width,height)
         SELECT ?1,?2,?3,?4,?5,?6 WHERE EXISTS(SELECT 1 FROM boards WHERE id=?1)
          AND (SELECT COUNT(*) FROM board_shapes WHERE board_id=?1) < ?7",
        params![board,kind.key(),rect.x,rect.y,rect.width,rect.height,MAX_SHAPES],
    )?;
    if affected != 1 { return Err("白板不存在或形狀已達上限。".into()); }
    Ok(db.last_insert_rowid())
}

fn update_shape_on(db: &Connection, board: i64, id: i64, version: i64, rect: ShapeRect) -> Result<()> {
    rect.validate()?;
    let changed = db.execute(
        "UPDATE board_shapes SET x=?4,y=?5,width=?6,height=?7,version=version+1
         WHERE board_id=?1 AND id=?2 AND version=?3",
        params![board,id,version,rect.x,rect.y,rect.width,rect.height],
    )?;
    if changed != 1 { return Err("形狀已變更或不在此白板，請重新選取。".into()); }
    Ok(())
}

fn list_shapes_on(db: &Connection, board: i64) -> Result<Vec<Shape>> {
    let mut stmt = db.prepare(
        "SELECT id,kind,x,y,width,height,version FROM board_shapes WHERE board_id=?1 ORDER BY id"
    )?;
    let mut rows = stmt.query([board])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(Shape {
            id: r.get(0)?, kind: ShapeKind::parse(&r.get::<_,String>(1)?)?,
            rect: ShapeRect { x:r.get(2)?,y:r.get(3)?,width:r.get(4)?,height:r.get(5)? },
            version:r.get(6)?,
        });
    }
    Ok(out)
}

impl Notebook {
    pub(crate) fn board_shapes(&self, board:i64) -> Result<Vec<Shape>> {
        list_shapes_on(&self.connection,board)
    }
    pub(crate) fn create_board_shape(&self, board:i64, kind:ShapeKind, rect:ShapeRect) -> Result<i64> {
        create_shape_on(&self.connection,board,kind,rect)
    }
    pub(crate) fn update_board_shape(&self, board:i64, id:i64, version:i64, rect:ShapeRect) -> Result<()> {
        update_shape_on(&self.connection,board,id,version,rect)
    }
    pub(crate) fn update_board_shapes(&mut self, board:i64, changes:&[(Shape,ShapeRect)]) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (prior,rect) in changes { update_shape_on(&tx,board,prior.id,prior.version,*rect)?; }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn delete_board_shape(&self, board:i64, id:i64, version:i64) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM board_shapes WHERE board_id=?1 AND id=?2 AND version=?3",
            params![board,id,version],
        )? != 1 { return Err("形狀已變更或不在此白板，請重新選取。".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_move_preserves_geometry_through_reopen_and_rejects_stale_drag() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("shape-drag.sqlite3");
        let original = ShapeRect { x: 55, y: 220, width: 140, height: 80 };
        let (board, id, sibling, moving_version, target) = {
            let db = Notebook::open(&path, false)?;
            let board = db.create_board("拖曳形狀")?;
            let id = db.create_board_shape(board, ShapeKind::Ellipse, original)?;
            let sibling = db.create_board_shape(board, ShapeKind::Rectangle, original)?;
            let expected = db.board_shapes(board)?[0];
            let target = expected.rect.moved_by_pointer(90, -60, 144, 400);
            assert_eq!(target, ShapeRect { x: 115, y: 180, ..original });
            assert_eq!(original.moved_by_pointer(i32::MAX, i32::MIN, 96, 400),
                ShapeRect { x: 400, y: 0, ..original });
            assert_eq!(original.moved_by_pointer(0, 0, 192, 20), original);
            // Motion is preview-only until the app calls the versioned update.
            assert_eq!(db.board_shapes(board)?[0], expected);
            db.update_board_shape(board, id, expected.version, target)?;
            (board, id, sibling, expected.version, target)
        };
        {
            let reopened = Notebook::open(&path, false)?;
            assert!(reopened.update_board_shape(board, id, moving_version, original).is_err());
            let shapes = reopened.board_shapes(board)?;
            assert_eq!(shapes.len(), 2);
            assert_eq!((shapes[0].id, shapes[0].kind, shapes[0].rect),
                (id, ShapeKind::Ellipse, target));
            assert_eq!((shapes[1].id, shapes[1].rect), (sibling, original));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn shape_geometry_and_compare_and_swap_are_scoped_to_board() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE boards(id INTEGER PRIMARY KEY);
            INSERT INTO boards VALUES(1),(2);").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let rect = ShapeRect { x:520,y:140,width:140,height:80 };
        let id = create_shape_on(&db,1,ShapeKind::Ellipse,rect).unwrap();
        assert!(rect.hit(ShapeKind::Ellipse,650,200));
        assert!(!rect.hit(ShapeKind::Ellipse,520,140));
        assert!(rect.hit(ShapeKind::Rectangle,520,140));
        let moved = ShapeRect { x:450, ..rect };
        assert!(update_shape_on(&db,2,id,1,moved).is_err());
        assert!(update_shape_on(&db,1,id,1,ShapeRect { width:0,..rect }).is_err());
        update_shape_on(&db,1,id,1,moved).unwrap();
        assert!(update_shape_on(&db,1,id,1,rect).is_err());
        assert_eq!(list_shapes_on(&db,1).unwrap(),vec![Shape {
            id,kind:ShapeKind::Ellipse,rect:moved,version:2,
        }]);
        db.execute("DELETE FROM boards WHERE id=1",[]).unwrap();
        assert!(list_shapes_on(&db,1).unwrap().is_empty());
    }
}
