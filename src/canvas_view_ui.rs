use super::*;
use crate::canvas_projection::{self as projection, Geometry, Projection, Rect};
use crate::store::canvas_edit::CanvasEdit;
use crate::store::canvas_bundle::{CanvasAssetState, CanvasBundleImage};
use serde_json::Value;

pub(super) struct Pan {
    projection: Projection,
    start: [i32; 2],
    current: [i32; 2],
}

pub(super) struct Drag {
    projection: Projection,
    start: [i32; 2],
    current: [i32; 2],
    action: DragAction,
}

enum DragAction {
    Marquee,
    Move,
    Resize { id: String, original: Geometry },
}

fn selected_offset(view: &Canvas, drag: &Drag) -> [i64; 2] {
    projection::move_offset(drag.projection.drag_offset(drag.start, drag.current),
        projection::nodes(&view.doc).filter_map(|(node, geometry)| {
            node["id"].as_str().filter(|id| view.selection.contains(*id)).map(|_| geometry)
        }))
}

unsafe fn frame(dc: HDC, rect: RECT, accent: u32) {
    unsafe {
        let brush = CreateSolidBrush(accent);
        if !brush.is_null() {
            FrameRect(dc, &rect, brush);
            DeleteObject(brush);
        }
    }
}

fn coordinates(rect: RECT) -> Rect {
    [rect.left, rect.top, rect.right, rect.bottom]
}

fn rectangle([left, top, right, bottom]: Rect) -> RECT {
    RECT { left, top, right, bottom }
}

fn canvas_color(value: &Value, default: u32) -> u32 {
    match value.as_str() {
        Some("1") => color(185, 65, 64),
        Some("2") => color(180, 95, 43),
        Some("3") => color(155, 122, 38),
        Some("4") => TEAL,
        Some("5") => color(32, 122, 139),
        Some("6") => color(115, 79, 167),
        Some(hex) => hex_color(hex).unwrap_or(default),
        _ => default,
    }
}

fn hex_color(code: &str) -> Option<u32> {
    let hex = code.strip_prefix('#')?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    let [r, g, b, alpha] = match hex.len() {
        3 => [value >> 8, (value >> 4) & 15, value & 15, 15].map(|digit| digit * 17),
        4 => [value >> 12, (value >> 8) & 15, (value >> 4) & 15, value & 15].map(|digit| digit * 17),
        6 => [value >> 16, (value >> 8) & 255, value & 255, 255],
        8 => [value >> 24, (value >> 16) & 255, (value >> 8) & 255, value & 255],
        _ => return None,
    };
    // GDI strokes are opaque, so composite the optional alpha over the board.
    let composite = |channel, board| (channel * alpha + board * (255 - alpha) + 127) / 255;
    Some(color(composite(r, 246), composite(g, 248), composite(b, 243)))
}

fn center(rect: Rect) -> [i32; 2] {
    [(rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2]
}

fn anchor(rect: Rect, requested: Option<&str>, other: [i32; 2]) -> ([i32; 2], [i32; 2]) {
    let [x, y] = center(rect);
    let [dx, dy] = [other[0] - x, other[1] - y];
    let side = requested.unwrap_or(if dx.abs() >= dy.abs() {
        if dx >= 0 { "right" } else { "left" }
    } else if dy >= 0 { "bottom" } else { "top" });
    match side {
        "left" => ([rect[0], y], [-1, 0]),
        "bottom" => ([x, rect[3]], [0, 1]),
        "top" => ([x, rect[1]], [0, -1]),
        _ => ([rect[2], y], [1, 0]),
    }
}

unsafe fn paint_edge(dc: HDC, font: HFONT, edge: &Value, from: Rect, to: Rect, space: i32) {
    unsafe {
        let (start, outward_start) = anchor(from, edge["fromSide"].as_str(), center(to));
        let (end, outward_end) = anchor(to, edge["toSide"].as_str(), center(from));
        let extend = |[x, y]: [i32; 2], [dx, dy]: [i32; 2]| [x + dx * space, y + dy * space];
        let a = extend(start, outward_start);
        let b = extend(end, outward_end);
        let stroke = canvas_color(&edge["color"], TEAL);
        let pen = CreatePen(PS_SOLID, 1, stroke);
        if pen.is_null() { return; }
        let previous = SelectObject(dc, pen);
        MoveToEx(dc, start[0], start[1], null_mut());
        for [x, y] in [a, b, end] {
            LineTo(dc, x, y);
        }
        let arrow = |[x, y]: [i32; 2], [dx, dy]: [i32; 2]| {
            let base = (space / 2).max(2);
            let wing = (space / 3).max(2);
            MoveToEx(dc, x + dx * base - dy * wing, y + dy * base + dx * wing, null_mut());
            LineTo(dc, x, y);
            LineTo(dc, x + dx * base + dy * wing, y + dy * base - dx * wing);
        };
        if edge["fromEnd"].as_str() == Some("arrow") { arrow(start, outward_start); }
        if edge["toEnd"].as_str() != Some("none") { arrow(end, outward_end); }
        SelectObject(dc, previous);
        DeleteObject(pen);

        if let Some(label) = edge["label"].as_str().filter(|label| !label.is_empty()) {
            let label = label.chars().take(80).collect::<String>();
            let x = (a[0] + b[0]) / 2;
            let y = (a[1] + b[1]) / 2;
            let mut rect = RECT { left: x - space * 7, top: y - space - 2,
                                  right: x + space * 7, bottom: y + 1 };
            draw_text(dc, font, stroke, &label, &mut rect, DT_CENTER | DT_BOTTOM | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
    }
}

unsafe fn paint_bundled_image(dc: HDC, area: RECT, preview: &CanvasBundleImage) {
    let width = area.right - area.left;
    let height = area.bottom - area.top;
    if width < 2 || height < 2 { return; }
    let scale = (f64::from(width) / f64::from(preview.width))
        .min(f64::from(height) / f64::from(preview.height));
    let drawn_width = (f64::from(preview.width) * scale).round().clamp(1.0, f64::from(width)) as i32;
    let drawn_height = (f64::from(preview.height) * scale).round().clamp(1.0, f64::from(height)) as i32;
    let left = area.left + (width - drawn_width) / 2;
    let top = area.top + (height - drawn_height) / 2;
    unsafe { blit_bundled_image(dc, [left, top, left + drawn_width, top + drawn_height], preview) }
}

unsafe fn blit_bundled_image(dc: HDC, dst: Rect, preview: &CanvasBundleImage) {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: preview.width, biHeight: -preview.height,
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB,
            biSizeImage: 0, biXPelsPerMeter: 0, biYPelsPerMeter: 0,
            biClrUsed: 0, biClrImportant: 0,
        },
        bmiColors: [RGBQUAD { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbReserved: 0 }],
    };
    unsafe {
        let previous = SetStretchBltMode(dc, COLORONCOLOR);
        StretchDIBits(dc, dst[0], dst[1], dst[2] - dst[0], dst[3] - dst[1], 0, 0, preview.width, preview.height, preview.bgra.as_ptr().cast(),
            &info, DIB_RGB_COLORS, SRCCOPY);
        if previous != 0 { SetStretchBltMode(dc, previous); }
    }
}

impl App {
    pub(super) unsafe fn cancel_canvas_drag(&mut self) -> bool {
        unsafe {
            let Some(WorkspaceView::Canvas(view)) = self.extras.workspace.as_mut() else { return false; };
            let had_drag = view.drag.take().is_some();
            let had_pan = view.pan.take().is_some();
            if !had_drag && !had_pan { return false; }
            if GetCapture() == self.hwnd { ReleaseCapture(); }
            InvalidateRect(self.hwnd, &self.board_area(), 0);
            true
        }
    }

    pub(super) unsafe fn canvas_input(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Result<()> {
        unsafe {
            if matches!(msg, WM_CAPTURECHANGED | WM_CANCELMODE) {
                self.cancel_canvas_drag();
                return Ok(());
            }
            let client = self.board_area();
            let area = coordinates(client);
            let grip = self.scaled(10);
            let inset = self.scaled(12);
            let mut cursor = POINT { x: lp as u16 as i16 as i32, y: (lp >> 16) as u16 as i16 as i32 };
            if msg == WM_MOUSEWHEEL && ScreenToClient(self.hwnd, &mut cursor) == 0 {
                return Ok(());
            }
            let point = [cursor.x, cursor.y];
            let dpi = self.dpi;
            let Some(WorkspaceView::Canvas(view)) = self.extras.workspace.as_mut() else { return Ok(()); };
            let visible = view.projection.get().filter(|p| p.area == area && p.inset == inset);
            match msg {
                WM_MOUSEWHEEL if view.drag.is_none() && view.pan.is_none() => {
                    let wheel = (wp >> 16) as u16 as i16;
                    if let Some(projection) = visible.filter(|_| projection::contains(area, point) && wheel != 0) {
                        let camera = projection.zoom(point, wheel, dpi);
                        self.notebook.set_canvas_camera(view.id, Some(camera))?;
                        view.camera = Some(camera);
                        view.projection.set(Some(Projection::camera(area, inset, dpi, camera)));
                        InvalidateRect(self.hwnd, &client, 0);
                    }
                }
                WM_MBUTTONDOWN if view.drag.is_none() && view.pan.is_none() => {
                    if let Some(projection) = visible.filter(|_| projection::contains(area, point)) {
                        SetCapture(self.hwnd);
                        if GetCapture() == self.hwnd {
                            view.pan = Some(Pan { projection, start: point, current: point });
                        }
                    }
                }
                WM_MBUTTONUP => {
                    if let Some(pan) = view.pan.take() {
                        let captured = GetCapture() == self.hwnd;
                        if captured { ReleaseCapture(); }
                        InvalidateRect(self.hwnd, &client, 0);
                        if captured && pan.projection.area == area && pan.projection.inset == inset && point != pan.start {
                            let camera = pan.projection.pan(pan.start, point, dpi);
                            self.notebook.set_canvas_camera(view.id, Some(camera))?;
                            view.camera = Some(camera);
                            view.projection.set(Some(Projection::camera(area, inset, dpi, camera)));
                        }
                    }
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if view.drag.is_none() && view.pan.is_none() => {
                    if let Some(projection) = visible.filter(|_| projection::contains(area, point)) {
                        SetFocus(self.hwnd);
                        SetCapture(self.hwnd);
                        if GetCapture() == self.hwnd {
                            let action = match projection::hit_node(&view.doc, projection, point, grip) {
                                Some((id, original, true)) => {
                                    DragAction::Resize { id: id.to_owned(), original }
                                }
                                Some((id, _, false)) => {
                                    if !view.selection.contains(id) {
                                        view.selection.clear();
                                        view.selection.insert(id.to_owned());
                                    }
                                    DragAction::Move
                                }
                                None => DragAction::Marquee,
                            };
                            view.drag = Some(Drag { projection, start: point, current: point, action });
                            InvalidateRect(self.hwnd, &client, 0);
                        }
                    }
                }
                WM_MOUSEMOVE => {
                    if let Some(pan) = &mut view.pan {
                        pan.current = point;
                        InvalidateRect(self.hwnd, &client, 0);
                    } else if let Some(drag) = &mut view.drag {
                        drag.current = point;
                        InvalidateRect(self.hwnd, &client, 0);
                    }
                }
                WM_LBUTTONUP => {
                    if let Some(mut drag) = view.drag.take() {
                        let captured = GetCapture() == self.hwnd;
                        if captured { ReleaseCapture(); }
                        InvalidateRect(self.hwnd, &client, 0);
                        if !captured || drag.projection.area != area || drag.projection.inset != inset {
                            return Ok(());
                        }
                        drag.current = point;
                        let edit = match &drag.action {
                            DragAction::Marquee => {
                                let bounds = projection::enclose(drag.start, point);
                                view.selection = projection::nodes(&view.doc).filter_map(|(node, geometry)| {
                                    let rect = drag.projection.rectangle(geometry);
                                    if projection::encloses(bounds, rect) && projection::encloses(area, rect) {
                                        node["id"].as_str().map(str::to_owned)
                                    } else { None }
                                }).collect();
                                label(self.widgets.status, &format!("已選取 {} 個畫布節點。", view.selection.len()));
                                return Ok(());
                            }
                            DragAction::Move => {
                                let offset = selected_offset(view, &drag);
                                if offset == [0, 0] { return Ok(()); }
                                CanvasEdit::Translate { ids: &view.selection, offset }
                            }
                            DragAction::Resize { id, original } => {
                                let offset = drag.projection.drag_offset(drag.start, point);
                                if offset == [0, 0] { return Ok(()); }
                                CanvasEdit::Geometry { id, geometry: projection::resize(*original, offset) }
                            }
                        };
                        view.doc = self.notebook.edit_canvas(view.id, &view.doc, edit)?;
                        label(self.widgets.status, "已儲存畫布修改。");
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    pub(super) unsafe fn paint_canvas(&self, dc: HDC, area: RECT, view: &Canvas) {
        unsafe {
            let area = coordinates(area);
            let inset = self.scaled(12);
            let projection = view.projection.get()
                .filter(|p| p.area == area && p.inset == inset)
                .or_else(|| view.camera.map(|camera| Projection::camera(area, inset, self.dpi, camera)))
                .or_else(|| Projection::fit(&view.doc, area, inset));
            view.projection.set(projection);
            let Some(projection) = projection else { return; };
            let projection = view.pan.as_ref().filter(|pan| pan.projection.area == area && pan.projection.inset == inset)
                .map_or(projection, |pan| Projection::camera(area, inset, self.dpi,
                    pan.projection.pan(pan.start, pan.current, self.dpi)));
            let drag = view.drag.as_ref()
                .filter(|drag| drag.projection.area == area && drag.projection.inset == inset);
            let offset = drag.filter(|drag| matches!(drag.action, DragAction::Move))
                .map_or([0, 0], |drag| selected_offset(view, drag));
            let grip_size = self.scaled(10);
            let mut places = HashMap::new();
            let mut cards = Vec::new();
            for (node, mut geometry) in projection::nodes(&view.doc) {
                let id = node["id"].as_str().unwrap_or_default();
                if let Some(drag) = drag
                    && let DragAction::Resize { id: resized, original } = &drag.action
                    && id == resized
                {
                    let delta = drag.projection.drag_offset(drag.start, drag.current);
                    if delta != [0, 0] { geometry = projection::resize(*original, delta); }
                }
                if view.selection.contains(id) {
                    geometry[0] += offset[0];
                    geometry[1] += offset[1];
                }
                let rect = projection.rectangle(geometry);
                let Some(visible) = projection.visible_rect(rect) else { continue; };
                places.insert(id, visible);
                cards.push((node, rectangle(visible), projection.grip(rect, grip_size), rect));
            }
            // Draw backgrounds for each group in their array order. Keep
            // the group surfaces under lines so opaque backgrounds never
            // conceal edges crossing or terminating within the group.
            if let Some(bundle) = &view.bundle {
                for (node, visible, _, full) in &cards {
                    if node["type"].as_str() != Some("group") { continue; }
                    let Some(name) = node["background"].as_str() else { continue; };
                    let Some(CanvasAssetState::Stored(path)) = bundle.references.get(name) else { continue; };
                    let Some(image) = bundle.previews.get(path) else { continue; };
                    let clipped = coordinates(*visible);
                    let plan = projection::group_background_plan(
                        *full, clipped, [image.width, image.height], node["backgroundStyle"].as_str(),
                    );
                    if plan.is_empty() { continue; }
                    let saved = SaveDC(dc);
                    if saved == 0 { continue; }
                    if IntersectClipRect(dc, clipped[0], clipped[1], clipped[2], clipped[3]) != 0 {
                        for dst in plan { blit_bundled_image(dc, dst, image); }
                    }
                    RestoreDC(dc, saved);
                }
            }
            for edge in view.doc.json()["edges"].as_array().into_iter().flatten().take(1000) {
                let from = edge["fromNode"].as_str().and_then(|id| places.get(id));
                let to = edge["toNode"].as_str().and_then(|id| places.get(id));
                if let (Some(from), Some(to)) = (from, to) {
                    paint_edge(dc, self.fonts[4], edge, *from, *to, self.scaled(13));
                }
            }
            for (node, mut rect, grip, _) in cards {
                let kind = node["type"].as_str().unwrap_or_default();
                let accent = canvas_color(&node["color"], TEAL);
                if kind != "group" { fill(dc, &rect, PAPER); }
                frame(dc, rect, accent);
                if let Some(grip) = grip { fill(dc, &rectangle(grip), accent); }
                let field = match kind { "text" => "text", "file" => "file", "link" => "url", _ => "label" };
                let caption = node[field].as_str().unwrap_or(kind).chars().take(140)
                    .map(|c| if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { c })
                    .collect::<String>();
                let unknown = !matches!(kind, "text" | "file" | "link" | "group");
                let margin = self.scaled(4);
                rect.left = rect.left.saturating_add(margin);
                rect.right = rect.right.saturating_sub(margin);
                let board_file = view.bundle.as_ref().and_then(|bundle| {
                    let key = match kind {
                        "file" => node["file"].as_str(),
                        "group" => node["background"].as_str(),
                        _ => None,
                    }?;
                    bundle.references.get(key).map(|state| (bundle, state))
                });
                let mut line = rect;
                line.bottom = rect.bottom.min(rect.top.saturating_add(self.scaled(20)));
                let title = if unknown { format!("未知節點｜{caption}") } else { caption };
                draw_text(dc, self.fonts[4], INK, &title, &mut line, DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
                if let Some((bundle, state)) = board_file {
                    let mut body = RECT { top: line.bottom, bottom: rect.bottom.saturating_sub(margin), ..rect };
                    let badge = match state {
                        CanvasAssetState::Stored(path) => {
                            if let Some(image) = bundle.previews.get(path) {
                                if kind == "file" { paint_bundled_image(dc, body, image); }
                                continue;
                            }
                            "附件已保存"
                        }
                        CanvasAssetState::Missing => "缺少附件",
                        CanvasAssetState::Ambiguous => "附件路徑有歧義",
                        CanvasAssetState::Unsafe => "附件路徑無效或不在封裝內",
                    };
                    if body.bottom > body.top {
                        draw_text(dc, self.fonts[4], SOFT, badge, &mut body, DT_SINGLELINE | DT_END_ELLIPSIS);
                    }
                }
            }
            // Keep the selection visible even if a later node overlaps it.
            for id in &view.selection {
                if let Some(&[left, top, right, bottom]) = places.get(id.as_str()) {
                    let border = self.scaled(2);
                    let inset = RECT { left: left + border, top: top + border,
                                       right: right - border, bottom: bottom - border };
                    if inset.left < inset.right && inset.top < inset.bottom { frame(dc, inset, INK); }
                }
            }
            if let Some(drag) = drag.filter(|drag| matches!(drag.action, DragAction::Marquee))
                && let Some(bounds) = projection.visible_rect(projection::enclose(drag.start, drag.current))
            {
                frame(dc, rectangle(bounds), INK);
            }
        }
    }
}
