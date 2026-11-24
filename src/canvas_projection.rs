use crate::canvas::{self, Document};
use serde_json::Value;

const MAX_COORDINATE: i64 = 1_000_000_000_000_000;
pub type Rect = [i32; 4];
pub type Geometry = [i64; 4];

#[derive(Clone, Copy)]
pub struct Projection {
    pub area: Rect,
    pub inset: i32,
    origin: [i64; 2],
    scale: f64,
}

pub fn nodes(doc: &Document) -> impl Iterator<Item = (&Value, Geometry)> {
    doc.json()["nodes"].as_array().into_iter().flatten().take(500).filter_map(|node| {
        let (x, y, width, height) = canvas::geometry_for_preview(node)?;
        Some((node, [x as i64, y as i64, width as i64, height as i64]))
    })
}

impl Projection {
    pub fn fit(doc: &Document, area: Rect, inset: i32) -> Option<Self> {
        let mut geometry = nodes(doc).map(|(_, geometry)| geometry);
        let [x, y, width, height] = geometry.next()?;
        let [mut left, mut top, mut right, mut bottom] = [x, y, x + width, y + height];
        for [x, y, width, height] in geometry {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + width);
            bottom = bottom.max(y + height);
        }
        let scale_x = f64::from((area[2] - area[0] - 2 * inset).max(1)) / (right - left) as f64;
        let scale_y = f64::from((area[3] - area[1] - 2 * inset).max(1)) / (bottom - top) as f64;
        Some(Self { area, inset, origin: [left, top], scale: scale_x.min(scale_y).min(1.5) })
    }

    pub fn rectangle(self, [x, y, width, height]: Geometry) -> Rect {
        let left = (self.area[0] + self.inset).saturating_add(((x - self.origin[0]) as f64 * self.scale) as i32);
        let top = (self.area[1] + self.inset).saturating_add(((y - self.origin[1]) as f64 * self.scale) as i32);
        [
            left,
            top,
            left.saturating_add(((width as f64 * self.scale) as i32).max(2)),
            top.saturating_add(((height as f64 * self.scale) as i32).max(2)),
        ]
    }

    pub fn visible_rect(self, [left, top, right, bottom]: Rect) -> Option<Rect> {
        let left = left.max(self.area[0]);
        let top = top.max(self.area[1]);
        let right = right.min(self.area[2]);
        let bottom = bottom.min(self.area[3]);
        (left < right && top < bottom).then_some([left, top, right, bottom])
    }

    pub fn grip(self, rect: Rect, size: i32) -> Option<Rect> {
        let [left, top, right, bottom] = rect;
        let [_, _, board_right, board_bottom] = self.area;
        if right > board_right || bottom > board_bottom
            || right.saturating_sub(left) < size * 2 || bottom.saturating_sub(top) < size * 2
        {
            return None;
        }
        self.visible_rect([right - size, bottom - size, right, bottom])
    }
}

pub fn contains([left, top, right, bottom]: Rect, [x, y]: [i32; 2]) -> bool {
    (left..right).contains(&x) && (top..bottom).contains(&y)
}

pub fn hit_node(
    doc: &Document, projection: Projection, point: [i32; 2], grip_size: i32,
) -> Option<(&str, Geometry, bool)> {
    if !contains(projection.area, point) {
        return None;
    }
    // The last visible hit is the topmost. A group's interior is transparent.
    nodes(doc).filter_map(|(node, geometry)| {
        let rect = projection.rectangle(geometry);
        if !contains(rect, point) {
            return None;
        }
        let resize = projection.grip(rect, grip_size).is_some_and(|grip| contains(grip, point));
        if !resize && node["type"].as_str() == Some("group")
            && contains([
                rect[0].saturating_add(2), rect[1].saturating_add(grip_size * 2),
                rect[2].saturating_sub(2), rect[3].saturating_sub(2),
            ], point)
        {
            return None;
        }
        Some((node["id"].as_str()?, geometry, resize))
    }).last()
}

pub struct Drag {
    pub id: String,
    pub projection: Projection,
    pub original: Geometry,
    pub start: [i32; 2],
    pub current: [i32; 2],
    pub resize: bool,
}

impl Drag {
    pub fn geometry(&self) -> Geometry {
        let dx = self.current[0].saturating_sub(self.start[0]);
        let dy = self.current[1].saturating_sub(self.start[1]);
        let [x, y, width, height] = self.original;
        if dx.unsigned_abs().max(dy.unsigned_abs()) < 3 {
            return self.original;
        }
        let to_world = |amount: i32| (f64::from(amount) / self.projection.scale).round() as i64;
        let (dx, dy) = (to_world(dx), to_world(dy));
        if self.resize {
            [x, y, width.saturating_add(dx).clamp(1, MAX_COORDINATE),
             height.saturating_add(dy).clamp(1, MAX_COORDINATE)]
        } else {
            let move_axis = |origin: i64, delta: i64| {
                origin.saturating_add(delta).clamp(-MAX_COORDINATE, MAX_COORDINATE)
            };
            [move_axis(x, dx), move_axis(y, dy), width, height]
        }
    }
}

// A bounded preview plan. Origin stays on the uncut group when scrolled,
// so a visible slice does not recenter a cover image or restart the tiles.
#[cfg(any(windows, test))]
pub(crate) fn group_background_plan(
    full: Rect, visible: Rect, image: [i32; 2], style: Option<&str>,
) -> Vec<Rect> {
    let [w, h] = image.map(i64::from);
    let [l, t, r, b] = full.map(i64::from);
    let [vl, vt, vr, vb] = visible.map(i64::from);
    if w < 1 || h < 1 || l >= r || t >= b || vl >= vr || vt >= vb { return Vec::new(); }
    let box_rect = |x: i64, y: i64, width: i64, height: i64| -> Option<Rect> {
        let raw = [x, y, x + width, y + height];
        if width > i64::from(i32::MAX) || height > i64::from(i32::MAX) {
            return None;
        }
        Some([
            i32::try_from(raw[0]).ok()?, i32::try_from(raw[1]).ok()?,
            i32::try_from(raw[2]).ok()?, i32::try_from(raw[3]).ok()?,
        ])
    };
    if style == Some("repeat") {
        // Bound per-group painting. A tiny source pattern may be enlarged
        // uniformly to keep each visible axis at no more than 16 tiles.
        let factor = ((vr - vl + w * 15 - 1) / (w * 15)).max((vb - vt + h * 15 - 1) / (h * 15)).max(1);
        let [tw, th] = [w * factor, h * factor];
        let x = l + (vl - l).div_euclid(tw) * tw;
        let y = t + (vt - t).div_euclid(th) * th;
        let mut out = Vec::new();
        for yi in 0..16 {
            let y = y + th * yi;
            if y >= vb { break; }
            for xi in 0..16 {
                let x = x + tw * xi;
                if x >= vr { break; }
                if let Some(rect) = box_rect(x, y, tw, th) { out.push(rect); }
            }
        }
        return out;
    }
    let fx = (r - l) as f64 / w as f64;
    let fy = (b - t) as f64 / h as f64;
    let scale = if style == Some("ratio") { fx.min(fy) } else { fx.max(fy) };
    let width = (w as f64 * scale).round().max(1.0) as i64;
    let height = (h as f64 * scale).round().max(1.0) as i64;
    box_rect(l + (r - l - width).div_euclid(2), t + (b - t - height).div_euclid(2), width, height)
        .into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Result;

    #[test]
    fn background_cover_and_ratio_fit_full_group_without_recentering_clip() {
        let full = [10, 20, 110, 100];
        let cropped = [30, 25, 100, 60];
        assert_eq!(group_background_plan(full, cropped, [40, 20], Some("cover")), [[-20, 20, 140, 100]]);
        assert_eq!(group_background_plan(full, cropped, [40, 20], Some("ratio")), [[10, 35, 110, 85]]);
        assert_eq!(group_background_plan(full, cropped, [40, 20], None), [[-20, 20, 140, 100]]);
    }

    #[test]
    fn repeat_preserves_origin_and_bounds_tile_work() {
        let full = [10, 20, 130, 121];
        let cropped = [19, 35, 70, 66];
        let tiles = group_background_plan(full, cropped, [8, 5], Some("repeat"));
        assert_eq!(tiles.first(), Some(&[18, 35, 26, 40]));
        assert_eq!(tiles.last(), Some(&[66, 65, 74, 70]));
        let huge = [0, 0, 10_002, 8_003];
        let tiles = group_background_plan(huge, huge, [1, 1], Some("repeat"));
        assert!(tiles.len() <= 256 && !tiles.is_empty());
        assert!(tiles.iter().any(|r| r[2] >= huge[2] && r[3] >= huge[3]));
        assert!(group_background_plan(full, cropped, [0, 10], None).is_empty());
    }

    #[test]
    fn hits_topmost_card_below_a_transparent_group_and_preserves_anchor_on_resize() -> Result<()> {
        let doc = canvas::parse(r#"{"nodes":[
            {"id":"bottom","type":"text","text":"a","x":-20,"y":0,"width":100,"height":80},
            {"id":"top","type":"file","file":"乙/鏡頭.md","x":10,"y":30,"width":80,"height":90},
            {"id":"frame","type":"group","x":-30,"y":-20,"width":200,"height":160}
        ]}"#.into())?;
        let p = Projection::fit(&doc, [330, 100, 554, 284], 12).unwrap();
        let [left, top, right, bottom] = p.rectangle([10, 30, 80, 90]);
        let start = [left + 20, top + 20];
        let (id, original, resize) = hit_node(&doc, p, start, 10).unwrap();
        assert_eq!((id, original, resize), ("top", [10, 30, 80, 90], false));
        let mut drag = Drag { id: id.into(), projection: p, original, start, current: start, resize };
        drag.current = [start[0] + 2, start[1] + 2];
        assert_eq!(drag.geometry(), original);
        drag.current = [start[0] - 28, start[1] + 13];
        assert_eq!(drag.geometry(), [-18, 43, 80, 90]);
        assert_eq!(p.rectangle(drag.geometry()), [left - 28, top + 13, right - 28, bottom + 13]);

        let corner = [right - 3, bottom - 3];
        let (_, _, resize) = hit_node(&doc, p, corner, 10).unwrap();
        assert!(resize);
        drag.resize = resize;
        drag.start = corner;
        drag.current = [corner[0] + 23, corner[1] - 150];
        assert_eq!(drag.geometry(), [10, 30, 103, 1]);
        assert!(p.grip([520, 210, 558, 270], 10).is_none());
        assert!(hit_node(&doc, p, [0, 0], 10).is_none());
        Ok(())
    }
}
