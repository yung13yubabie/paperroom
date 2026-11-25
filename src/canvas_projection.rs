use crate::canvas::{self, Document};
use serde_json::Value;

pub(crate) const MAX_COORDINATE: i64 = 1_000_000_000_000_000;
const MAX_VIEW_ORIGIN: f64 = 4e15;
const MIN_VIEW_SCALE: f64 = 1e-16;
const MAX_VIEW_SCALE: f64 = 12.0;
pub(crate) const MAX_DRAWN_NODES: usize = 500;
pub type Rect = [i32; 4];
pub type Geometry = [i64; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Camera {
    pub origin: [f64; 2],
    pub scale: f64,
}

impl Camera {
    pub(crate) fn valid(self) -> bool {
        self.origin.iter().all(|axis| axis.abs() <= MAX_VIEW_ORIGIN)
            && (MIN_VIEW_SCALE..=MAX_VIEW_SCALE).contains(&self.scale)
    }
}

#[derive(Clone, Copy)]
pub struct Projection {
    pub area: Rect,
    pub inset: i32,
    origin: [f64; 2],
    scale: f64,
}

fn geometry(node: &Value) -> Option<Geometry> {
    let (x, y, width, height) = canvas::geometry_for_preview(node)?;
    Some([x as i64, y as i64, width as i64, height as i64])
}

pub fn nodes(doc: &Document) -> impl Iterator<Item = (&Value, Geometry)> {
    doc.json()["nodes"].as_array().into_iter().flatten().take(MAX_DRAWN_NODES)
        .filter_map(|node| Some((node, geometry(node)?)))
}

pub(crate) fn all_geometry(doc: &Document) -> impl Iterator<Item = (&Value, Geometry)> {
    doc.json()["nodes"].as_array().into_iter().flatten()
        .filter_map(|node| Some((node, geometry(node)?)))
}

pub(crate) fn encloses_geometry([x, y, w, h]: Geometry, [nx, ny, nw, nh]: Geometry) -> bool {
    x <= nx && y <= ny && nx + nw <= x + w && ny + nh <= y + h
}

#[cfg(any(windows, test))]
pub(crate) fn move_nodes(
    doc: &Document, selection: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, Vec<Geometry>) {
    let mut nodes = all_geometry(doc).filter_map(|(node, rect)| {
        Some((node["id"].as_str()?, node["type"].as_str() == Some("group"), rect))
    }).collect::<Vec<_>>();
    let groups = nodes.iter().filter_map(|(id, group, rect)| {
        (selection.contains(*id) && *group).then_some(*rect)
    }).collect::<Vec<_>>();
    nodes.retain(|(id, group, rect)| selection.contains(*id) || groups.iter().any(|frame| {
        // Coincident group frames are peers, not nested groups.
        (!group || frame != rect) && encloses_geometry(*frame, *rect)
    }));
    nodes.into_iter().map(|(id, _, rect)| (id.to_owned(), rect)).unzip()
}

impl Projection {
    pub fn drag_offset(self, start: [i32; 2], end: [i32; 2]) -> [i64; 2] {
        let pixels = [end[0].saturating_sub(start[0]), end[1].saturating_sub(start[1])];
        if pixels.iter().all(|amount| amount.unsigned_abs() < 3) { return [0; 2]; }
        pixels.map(|amount| (f64::from(amount) / self.scale).round() as i64)
    }

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
        Some(Self { area, inset, origin: [left as f64, top as f64], scale: scale_x.min(scale_y).min(1.5) })
    }

    #[cfg(any(windows, test))]
    pub(crate) fn focus_camera(
        area: Rect, inset: i32, dpi: u32, [x, y, width, height]: Geometry,
    ) -> Option<Camera> {
        let available = [area[2] - area[0] - 2 * inset, area[3] - area[1] - 2 * inset];
        if available.iter().any(|size| *size < 4) || width <= 0 || height <= 0 { return None; }
        let dpi_factor = f64::from(dpi.max(1)) / 96.0;
        let physical = (0.8 * f64::from(available[0]) / width as f64)
            .min(0.8 * f64::from(available[1]) / height as f64)
            .min(1.5 * dpi_factor);
        let center = [x as f64 + width as f64 / 2.0, y as f64 + height as f64 / 2.0];
        let camera = Camera {
            origin: std::array::from_fn(|axis| center[axis] - f64::from(available[axis]) / (2.0 * physical)),
            scale: physical / dpi_factor,
        };
        camera.valid().then_some(camera)
    }

    pub(crate) fn camera(area: Rect, inset: i32, dpi: u32, camera: Camera) -> Self {
        Self { area, inset, origin: camera.origin, scale: camera.scale * f64::from(dpi.max(1)) / 96.0 }
    }

    pub(crate) fn pan(self, start: [i32; 2], point: [i32; 2], dpi: u32) -> Camera {
        let origin = std::array::from_fn(|i| {
            let pixels = f64::from(point[i]) - f64::from(start[i]);
            (self.origin[i] - pixels / self.scale).clamp(-MAX_VIEW_ORIGIN, MAX_VIEW_ORIGIN)
        });
        Camera { origin, scale: self.logical_scale(dpi) }
    }

    pub(crate) fn zoom(self, point: [i32; 2], wheel: i16, dpi: u32) -> Camera {
        let previous = self.logical_scale(dpi);
        let scale = (previous * 1.25f64.powf(f64::from(wheel) / 120.0))
            .clamp(MIN_VIEW_SCALE, MAX_VIEW_SCALE);
        let physical = scale * f64::from(dpi.max(1)) / 96.0;
        let origin = std::array::from_fn(|i| {
            let pixels = f64::from(point[i] - self.area[i] - self.inset);
            (self.origin[i] + pixels / self.scale - pixels / physical)
                .clamp(-MAX_VIEW_ORIGIN, MAX_VIEW_ORIGIN)
        });
        Camera { origin, scale }
    }

    fn logical_scale(self, dpi: u32) -> f64 {
        (self.scale * 96.0 / f64::from(dpi.max(1))).clamp(MIN_VIEW_SCALE, MAX_VIEW_SCALE)
    }

    pub fn rectangle(self, [x, y, width, height]: Geometry) -> Rect {
        let left = (self.area[0] + self.inset).saturating_add(((x as f64 - self.origin[0]) * self.scale) as i32);
        let top = (self.area[1] + self.inset).saturating_add(((y as f64 - self.origin[1]) * self.scale) as i32);
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

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn focus_centers_and_fits_a_distant_occurrence_across_dpi() {
        for (area, inset, dpi) in [([303,100,950,680],12,96), ([495,150,1425,1020],18,144)] {
            for geometry in [[8_000_000,-9_000_000,112,80],[-3_000,2_000,80_000,450_000]] {
                let camera = Projection::focus_camera(area, inset, dpi, geometry).unwrap();
                let projected = Projection::camera(area, inset, dpi, camera).rectangle(geometry);
                assert!(encloses(area, projected));
                assert!((projected[0]+projected[2]-area[0]-area[2]).abs() <= 3);
                assert!((projected[1]+projected[3]-area[1]-area[3]).abs() <= 3);
            }
        }
        assert!(Projection::focus_camera([0,0,40,40],24,96,[0,0,1,1]).is_none());
    }

    #[test]
    fn camera_pan_zoom_and_dpi_preserve_the_world_anchor() {
        let area = [20, 40, 620, 460];
        let before = Projection::camera(area, 12, 96, Camera { origin: [0.0, 0.0], scale: 1.5 });
        let node = [100, 120, 80, 50];
        let a = before.rectangle(node);
        let panned = before.pan([200, 200], [250, 220], 96);
        let moved = Projection::camera(area, 12, 96, panned);
        let b = moved.rectangle(node);
        assert_eq!([b[0] - a[0], b[1] - a[1]], [50, 20]);
        let zoom = moved.zoom([b[0], b[1]], 120, 96);
        assert_eq!(zoom.scale, 1.875);
        let c = Projection::camera(area, 12, 96, zoom).rectangle(node);
        assert!((c[0] - b[0]).abs() <= 1 && (c[1] - b[1]).abs() <= 1);
        assert!(c[2] - c[0] > b[2] - b[0] && c[3] - c[1] > b[3] - b[1]);
        let d = Projection::camera(area, 18, 144, zoom).rectangle(node);
        assert!((2 * (d[2] - d[0]) - 3 * (c[2] - c[0])).abs() <= 2);
        assert!(moved.zoom([b[0], b[1]], i16::MAX, 96).valid());
        assert!(moved.zoom([b[0], b[1]], i16::MIN, 96).valid());
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

pub fn enclose(a: [i32; 2], b: [i32; 2]) -> Rect {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

pub fn encloses([left, top, right, bottom]: Rect, [x0, y0, x1, y1]: Rect) -> bool {
    left <= x0 && top <= y0 && x1 <= right && y1 <= bottom
}

pub fn move_offset(mut delta: [i64; 2], geometries: impl Iterator<Item = Geometry>) -> [i64; 2] {
    for [x, y, ..] in geometries {
        delta[0] = delta[0].clamp(-MAX_COORDINATE - x, MAX_COORDINATE - x);
        delta[1] = delta[1].clamp(-MAX_COORDINATE - y, MAX_COORDINATE - y);
    }
    delta
}

pub fn resize([x, y, width, height]: Geometry, [dx, dy]: [i64; 2]) -> Geometry {
    [x, y, width.saturating_add(dx).clamp(1, MAX_COORDINATE),
     height.saturating_add(dy).clamp(1, MAX_COORDINATE)]
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
        assert_eq!(p.drag_offset(start, [start[0] + 2, start[1] + 2]), [0, 0]);
        let [dx, dy] = p.drag_offset(start, [start[0] - 28, start[1] + 13]);
        assert_eq!(p.rectangle([original[0] + dx, original[1] + dy, original[2], original[3]]),
            [left - 28, top + 13, right - 28, bottom + 13]);
        let marquee = enclose([right + 2, bottom + 2], [left - 2, top - 2]);
        assert!(encloses(marquee, [left, top, right, bottom]));
        assert!(!encloses(marquee, [left - 3, top, right, bottom]));
        assert_eq!(move_offset([99, -99], [
            [MAX_COORDINATE - 3, 0, 10, 10], [0, 4 - MAX_COORDINATE, 10, 10],
        ].into_iter()), [3, -4]);

        let corner = [right - 3, bottom - 3];
        assert!(hit_node(&doc, p, corner, 10).unwrap().2);
        assert_eq!(super::resize(original, p.drag_offset(corner, [corner[0] + 23, corner[1] - 150])),
            [10, 30, 103, 1]);
        assert!(p.grip([520, 210, 558, 270], 10).is_none());
        assert!(hit_node(&doc, p, [0, 0], 10).is_none());
        Ok(())
    }
}
