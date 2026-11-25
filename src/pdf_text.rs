use hayro::hayro_interpret::RectExt;
use crate::store::PdfRegion;
use hayro::hayro_interpret::font::{Glyph, GlyphId, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::{BfString, WritingMode};
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect, Shape};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, Paint, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::page::Page;

const MAX_RUNS: usize = 512;
const MAX_GLYPHS: usize = 12_000;
const MAX_MARKS: usize = 1024;
const MAX_QUOTE: usize = 2048;
const MAX_PIXEL_CHECKS: usize = 8_000_000;

#[derive(Default)]
pub struct TextLayer {
    runs: Vec<Run>,
    blockers: Vec<Rect>,
}

struct Run {
    bounds: Rect,
    origin: f64,
    baseline: f64,
    em_width: f64,
    em_height: f64,
    glyphs: Vec<GlyphBox>,
}

struct GlyphBox {
    text: String,
    ink: Option<Rect>,
}

#[derive(Clone)]
pub struct TextSelection {
    pub quote: String,
    pub regions: Vec<PdfRegion>,
}

impl TextLayer {
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Endpoints use whole-page top-left fractions. Cross-column selection
    /// additionally requires geometry and content order to agree.
    pub fn select(&self, from: [f64; 2], to: [f64; 2]) -> Option<TextSelection> {
        let endpoints = self.endpoint(from).zip(self.endpoint(to));
        if let Some((start, end)) = endpoints
            && !start.aligned_with(end)
        {
            return self.select_across_columns((start, from), (end, to));
        }
        let left = from[0].min(to[0]);
        let right = from[0].max(to[0]);
        let mut same_line = self.runs.iter().filter(|run| {
            left <= run.bounds.x1 && right >= run.bounds.x0
                && run.on_line(from[1]) && run.on_line(to[1])
        });
        if let Some(run) = same_line.next() {
            if same_line.next().is_some() || right - left < 0.001 {
                return None;
            }
            let (quote, region) = run.select(left, right)?;
            return finish_selection(vec![(quote, region)]);
        }

        let (start, end) = endpoints?;
        let (start, end, top, bottom) = if from[1] <= to[1] {
            (start, end, from, to)
        } else {
            (end, start, to, from)
        };
        if start.baseline >= end.baseline || !start.aligned_with(end) {
            return None;
        }
        let mut lines: Vec<&Run> = self.runs.iter().filter(|run| {
            (start.baseline..=end.baseline).contains(&run.baseline) && start.aligned_with(run)
        }).collect();
        lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
        if lines.len() > PdfRegion::MAX_TEXT_SEGMENTS
            || lines.windows(2).any(|pair| !pair[0].before_adjacent(pair[1]))
        {
            return None;
        }
        let corridor = lines.iter().map(|line| line.bounds).reduce(|a, b| a.union(b))?;
        // Reject an intersecting run from a competing column or split line.
        if self.runs.iter().any(|run| {
            corridor.overlaps(run.bounds)
                && !lines.iter().any(|line| std::ptr::eq(*line, run))
        }) {
            return None;
        }
        let selected = lines.into_iter().map(|run| {
            let left = if std::ptr::eq(run, start) { top[0] } else { 0.0 };
            let right = if std::ptr::eq(run, end) { bottom[0] } else { 1.0 };
            run.select(left, right)
        }).collect::<Option<_>>()?;
        finish_selection(selected)
    }

    fn select_across_columns(
        &self, from: (&Run, [f64; 2]), to: (&Run, [f64; 2]),
    ) -> Option<TextSelection> {
        let ((start, point_start), (end, point_end)) =
            if from.0.origin < to.0.origin { (from, to) } else { (to, from) };
        if !point_start.into_iter().chain(point_end).all(|v| (0.0..=1.0).contains(&v)) {
            return None;
        }
        let left = self.column_at(start)?;
        let right = self.column_at(end)?;
        let (left_first, left_last) = (left.first()?.1, left.last()?.1);
        let (right_first, right_last) = (right.first()?.1, right.last()?.1);
        let em = start.em_height.max(end.em_height);
        if em > start.em_height.min(end.em_height) * 1.2
            || (left_first.baseline - right_first.baseline).abs() > em * 0.45
            || (left_last.baseline - right_last.baseline).abs() > em * 0.45
            || left.last()?.0 >= right.first()?.0
        {
            return None;
        }
        let left_box = left.iter().map(|(_, line)| line.bounds).reduce(|a, b| a.union(b))?;
        let right_box = right.iter().map(|(_, line)| line.bounds).reduce(|a, b| a.union(b))?;
        if right_box.x0 - left_box.x1 < start.em_width.max(end.em_width) {
            return None;
        }
        // Inspect the entire horizontal band. A third lane, dropped glyphs,
        // illustrations and table rules must not be silently skipped.
        let band = left_box.union(right_box);
        let band = Rect::new(0.0, band.y0, 1.0, band.y1);
        if self.blockers.iter().any(|bounds| band.overlaps(*bounds)) ||
            self.runs.iter().enumerate().any(|(index, line)| {
                band.overlaps(line.bounds) && !left.iter().chain(&right).any(|(i, _)| *i == index)
            })
        {
            return None;
        }

        let begin = left.iter().position(|(_, line)| std::ptr::eq(*line, start))?;
        let finish = right.iter().position(|(_, line)| std::ptr::eq(*line, end))?;
        if left.len() - begin + finish + 1 > PdfRegion::MAX_TEXT_SEGMENTS {
            return None;
        }
        let lines = left[begin..].iter().chain(&right[..=finish]).map(|(_, line)| {
            let x0 = if std::ptr::eq(*line, start) { point_start[0] } else { 0.0 };
            let x1 = if std::ptr::eq(*line, end) { point_end[0] } else { 1.0 };
            line.select(x0, x1)
        }).collect::<Option<_>>()?;
        finish_selection(lines)
    }

    fn column_at(&self, anchor: &Run) -> Option<Vec<(usize, &Run)>> {
        let mut lines = self.runs.iter().enumerate().filter(|(_, line)| anchor.aligned_with(line))
            .collect::<Vec<_>>();
        lines.sort_by(|a, b| a.1.baseline.total_cmp(&b.1.baseline));
        let at = lines.iter().position(|(_, line)| std::ptr::eq(*line, anchor))?;
        let connected = |pair: &[(usize, &Run)]| pair[0].1.before_adjacent(pair[1].1);
        let begin = lines[..=at].windows(2).rposition(|pair| !connected(pair)).map_or(0, |i| i + 1);
        let finish = lines[at..].windows(2).position(|pair| !connected(pair)).map_or(lines.len(), |i| at + i + 1);
        let column = &lines[begin..finish];
        // A lone run could be a label. A different drawing sequence could be
        // a table, so do not invent a cross-column reading order from geometry alone.
        (column.len() >= 2 && column.windows(2).all(|pair| pair[0].0 < pair[1].0))
            .then(|| column.to_vec())
    }

    fn endpoint(&self, point: [f64; 2]) -> Option<&Run> {
        let mut runs: Vec<_> = self.runs.iter().filter_map(|run| {
            let distance = (run.bounds.x0 - point[0]).max(point[0] - run.bounds.x1).max(0.0);
            (run.on_line(point[1]) && distance <= run.em_width * 1.5).then_some((distance, run))
        }).collect();
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let &(distance, first) = runs.first()?;
        if runs.get(1).is_some_and(|(next, other)| {
            next - distance < first.em_width.min(other.em_width) * 0.15
        }) {
            return None;
        }
        Some(first)
    }
}

impl Run {
    fn on_line(&self, y: f64) -> bool {
        let tolerance = self.bounds.height() * 0.35;
        (self.bounds.y0 - tolerance..=self.bounds.y1 + tolerance).contains(&y)
    }

    fn aligned_with(&self, other: &Self) -> bool {
        (self.origin - other.origin).abs() <= self.em_width.min(other.em_width) * 0.4
            && self.em_height.max(other.em_height) <= self.em_height.min(other.em_height) * 1.2
    }

    fn before_adjacent(&self, other: &Self) -> bool {
        let gap = other.baseline - self.baseline;
        let em = self.em_height.max(other.em_height);
        (em * 0.65..=em * 2.05).contains(&gap) && self.bounds.y1 < other.bounds.y0
    }

    fn select(&self, left: f64, right: f64) -> Option<(String, PdfRegion)> {
        let included = |entry: &GlyphBox| entry.ink.is_some_and(|rect| {
            (left..=right).contains(&rect.center().x)
        });
        let first = self.glyphs.iter().position(included)?;
        let last = self.glyphs.iter().rposition(included)?;
        let glyphs = &self.glyphs[first..=last];
        let quote: String = glyphs.iter().map(|glyph| glyph.text.as_str()).collect();
        let rect = glyphs.iter().filter_map(|glyph| glyph.ink).reduce(|a, b| a.union(b))?;
        Some((quote, PdfRegion {
            x: rect.x0,
            y: rect.y0,
            width: rect.width().min(1.0 - rect.x0),
            height: rect.height().min(1.0 - rect.y0),
        }))
    }
}

fn finish_selection(lines: Vec<(String, PdfRegion)>) -> Option<TextSelection> {
    let (quotes, regions): (Vec<_>, Vec<_>) = lines.into_iter().unzip();
    let quote = quotes.join("\n");
    if quote.encode_utf16().count() > MAX_QUOTE {
        return None;
    }
    Some(TextSelection { quote, regions })
}

pub(super) fn extract(
    page: &Page<'_>, settings: &InterpreterSettings, scale: f32,
    width: u16, height: u16, bgra: &[u8],
) -> TextLayer {
    let transform = Affine::scale(f64::from(scale)) * page.initial_transform(true).to_kurbo();
    let target = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let crop = transform.transform_rect_bbox(page.intersected_crop_box().to_kurbo());
    let cache = InterpreterCache::new();
    let mut context = Context::new(transform, target, &cache, page.xref(), settings.clone());
    let mut collector = Collector {
        crop: crop.intersect(target),
        width: usize::from(width),
        bgra,
        runs: Vec::new(),
        marks: Vec::new(),
        glyph_count: 0,
        pixel_checks: 0,
        clips: 0,
        groups: 0,
        unusable: false,
    };
    interpret_page(page, &mut context, &mut collector);
    if collector.unusable {
        return TextLayer::default();
    }
    let dimensions = Affine::scale_non_uniform(1.0 / f64::from(width), 1.0 / f64::from(height));
    let mut runs = Vec::new();
    let mut blockers = collector.marks.iter().map(|mark| dimensions.transform_rect_bbox(*mark))
        .collect::<Vec<_>>();
    for (i, run) in collector.runs.iter().enumerate() {
        let bounds = run.bounds.inflate(1.0, 1.0);
        if collector.marks.iter().any(|mark| mark.overlaps(bounds))
            || collector.runs.iter().enumerate().any(|(j, other)| {
                i != j && other.bounds.overlaps(bounds)
            })
        {
            blockers.push(dimensions.transform_rect_bbox(bounds));
            continue;
        }
        let glyphs = run.glyphs.iter().map(|glyph| GlyphBox {
            text: glyph.text.clone(),
            ink: glyph.ink.map(|rect| dimensions.transform_rect_bbox(rect)),
        }).collect();
        runs.push(Run {
            bounds: dimensions.transform_rect_bbox(run.bounds),
            origin: run.origin / f64::from(width),
            baseline: run.baseline / f64::from(height),
            em_width: run.em_width / f64::from(width),
            em_height: run.em_height / f64::from(height),
            glyphs,
        });
    }
    TextLayer { runs, blockers }
}

struct Collector<'a> {
    crop: Rect,
    width: usize,
    bgra: &'a [u8],
    runs: Vec<Run>,
    marks: Vec<Rect>,
    glyph_count: usize,
    pixel_checks: usize,
    clips: usize,
    groups: usize,
    unusable: bool,
}

impl Collector<'_> {
    fn mark(&mut self, bounds: Rect) {
        if self.marks.len() == MAX_MARKS || !bounds.is_finite() {
            self.unusable = true;
        } else {
            self.marks.push(bounds.inflate(2.0, 2.0));
        }
    }

    fn collect_line(
        &mut self, run: &GlyphRun<'_, '_>, props: &DrawProps<'_>, mode: &DrawMode,
    ) -> Option<Run> {
        if self.clips != 0 || self.groups != 0 || props.soft_mask.is_some()
            || props.blend_mode != BlendMode::Normal || !matches!(mode, DrawMode::Fill(_))
            || run.writing_mode() != WritingMode::Horizontal
        {
            return None;
        }
        let Paint::Color(color) = &props.paint else { return None; };
        let [r, g, b, alpha] = color.to_rgba().to_rgba8();
        if alpha != 255 || r.min(g).min(b) > 210 {
            return None;
        }
        let mut glyphs = Vec::with_capacity(run.glyphs().len());
        let mut edge = f64::NEG_INFINITY;
        let mut metrics: Option<(f64, f64, f64)> = None;
        let mut bounds: Option<Rect> = None;
        for glyph in run.glyphs() {
            let Glyph::Outline(outline) = &**glyph else { return None; };
            let transform = props.transform * glyph.transform();
            let [sx, shear_y, shear_x, sy, x, y] = transform.as_coeffs();
            if !transform.is_finite() || sx <= 0.0 || sy >= 0.0
                || shear_x.abs() > 1e-8 || shear_y.abs() > 1e-8 || x <= edge
                || metrics.is_some_and(|(_, baseline, _)| (y - baseline).abs() > 1.0)
            {
                return None;
            }
            edge = x;
            metrics.get_or_insert((x, y, -sy * 1000.0));
            let text = match glyph.as_unicode()? {
                BfString::Char(ch) => ch.to_string(),
                BfString::String(text) => text,
            };
            if text.is_empty() || text.chars().count() > 12 || !text.chars().all(selectable_char) {
                return None;
            }
            let path = outline.outline();
            if text == " " && path.is_empty() {
                glyphs.push(GlyphBox { text, ink: None });
                continue;
            }
            let ink = transform.transform_rect_bbox(path.bounding_box());
            if outline.glyph_id() == GlyphId::NOTDEF || path.is_empty() || !ink.is_finite()
                || ink.width() < 0.75 || ink.height() < 0.75
                || !self.crop.contains_rect(ink.inflate(1.0, 1.0))
                || !self.has_ink(ink, [b, g, r])
            {
                return None;
            }
            bounds = Some(bounds.map_or(ink, |bounds| bounds.union(ink)));
            glyphs.push(GlyphBox { text, ink: Some(ink) });
        }
        let (origin, baseline, em) = metrics?;
        Some(Run { bounds: bounds?, origin, baseline, em_width: em, em_height: em, glyphs })
    }

    fn has_ink(&mut self, ink: Rect, color: [u8; 3]) -> bool {
        let x_span = ink.x0.floor() as usize..ink.x1.ceil() as usize;
        let y_span = ink.y0.floor() as usize..ink.y1.ceil() as usize;
        self.pixel_checks += x_span.len() * y_span.len();
        if self.pixel_checks > MAX_PIXEL_CHECKS {
            self.unusable = true;
            return false;
        }
        for y in y_span {
            for x in x_span.clone() {
                let pixel = &self.bgra[(y * self.width + x) * 4..][..3];
                if pixel.iter().zip(color).all(|(actual, expected)| actual.abs_diff(expected) <= 35) {
                    return true;
                }
            }
        }
        false
    }
}

impl<'a> Device<'a> for Collector<'_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        if self.unusable || matches!(mode, DrawMode::Invisible) {
            return;
        }
        let padding = match mode {
            DrawMode::Stroke(stroke) | DrawMode::FillAndStroke(_, stroke) => {
                f64::from(stroke.line_width.abs().max(1.0) * stroke.miter_limit.max(2.0))
            }
            _ => 0.0,
        };
        let bounds = props.transform.transform_rect_bbox(path.bounding_box().inflate(padding, padding));
        self.mark(bounds.inflate(1.0, 1.0));
    }

    fn push_clip_path(&mut self, _: &ClipPath) { self.clips += 1; }
    fn pop_clip(&mut self) { self.clips = self.clips.saturating_sub(1); }
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.groups += 1;
    }
    fn pop_transparency_group(&mut self) { self.groups = self.groups.saturating_sub(1); }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        let bounds = Rect::new(0.0, 0.0, f64::from(image.width()), f64::from(image.height()));
        self.mark(props.transform.transform_rect_bbox(bounds));
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        if self.unusable || matches!(mode, DrawMode::Invisible) || run.glyphs().is_empty() {
            return;
        }
        if matches!(&props.paint, Paint::Color(color) if color.to_rgba().to_rgba8()[3] == 0) {
            return;
        }
        self.glyph_count += run.glyphs().len();
        if self.glyph_count > MAX_GLYPHS || self.runs.len() == MAX_RUNS {
            self.unusable = true;
            return;
        }
        if let Some(line) = self.collect_line(run, &props, mode) {
            self.runs.push(line);
            return;
        }
        // Unsupported paint can cover other text. Keep its outline bounds as
        // blockers; Type 3 and stroked glyph programs lack safe bounds here.
        if !matches!(mode, DrawMode::Fill(_)) {
            self.unusable = true;
            return;
        }
        for glyph in run.glyphs() {
            let Glyph::Outline(outline) = &**glyph else {
                self.unusable = true;
                return;
            };
            let path = outline.outline();
            if !path.is_empty() {
                let transform = props.transform * glyph.transform();
                self.mark(transform.transform_rect_bbox(path.bounding_box()));
                if self.unusable { return; }
            }
        }
    }
}

// Permit common horizontal left-to-right text. Avoid guessing direction,
// reordering shaping/format controls, or substituting Unicode replacement marks.
fn selectable_char(ch: char) -> bool {
    matches!(ch,
        ' '..='~' | '\u{00a1}'..='\u{00ac}' | '\u{00ae}'..='\u{024f}'
        | '\u{2010}'..='\u{2027}' | '\u{2e80}'..='\u{2fdf}'
        | '\u{3000}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7a3}' | '\u{f900}'..='\u{faff}' | '\u{ff01}'..='\u{ff60}'
    )
}
