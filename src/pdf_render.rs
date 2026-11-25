use crate::store::Result;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{Pdf, PdfData};
use hayro::{PixmapSettings, RenderCache, RenderSettings};

#[path = "pdf_text.rs"]
mod pdf_text;
pub use pdf_text::TextSelection;

const MAX_PDF_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIXELS_PER_SIDE: f32 = 1400.0;

pub struct RenderedPage {
    pub width: u16,
    pub height: u16,
    pub page_count: usize,
    /// Opaque BGRA, with the first four bytes representing the top-left pixel.
    pub bgra: Vec<u8>,
    pub text: pdf_text::TextLayer,
}

impl RenderedPage {
    pub fn highlighted_bgra(&self, regions: &[crate::store::PdfRegion]) -> Option<Vec<u8>> {
        if regions.is_empty() {
            return None;
        }
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let mut coverage = vec![false; width * height];
        for region in regions {
            let x = |value: f64| (value.clamp(0.0, 1.0) * f64::from(self.width)).round() as usize;
            let y = |value: f64| (value.clamp(0.0, 1.0) * f64::from(self.height)).round() as usize;
            for row in y(region.y)..y(region.y + region.height) {
                let start = width * row + x(region.x);
                let end = width * row + x(region.x + region.width);
                coverage[start..end].fill(true);
            }
        }
        let mut marked = self.bgra.clone();
        for (covered, pixel) in coverage.into_iter().zip(marked.chunks_exact_mut(4)) {
            if covered {
                // Yellow tint affects the paper much more than dark glyphs.
                pixel[0] = (u16::from(pixel[0]) * 150 / 255) as u8;
                pixel[1] = (u16::from(pixel[1]) * 231 / 255) as u8;
            }
        }
        Some(marked)
    }
}

/// Parse and rasterize one 1-based page. Call this off the UI thread.
pub fn render_page(
    bytes: impl Into<PdfData> + std::panic::UnwindSafe, page: u32,
) -> Result<RenderedPage> {
    std::panic::catch_unwind(move || {
        let bytes = bytes.into();
        if bytes.as_ref().is_empty() || bytes.as_ref().len() > MAX_PDF_BYTES {
            return Err("PDF 預覽限制為 8 MiB。".into());
        }
        let index = page.checked_sub(1).ok_or("PDF 頁碼須從 1 起算。")? as usize;
        render(bytes, index)
    })
    .map_err(|_| "PDF 預覽因無法解析的頁面內容而中止。")?
}

fn render(bytes: PdfData, index: usize) -> Result<RenderedPage> {
    let pdf = Pdf::new(bytes).map_err(|_| "無法解析 PDF，或檔案使用了不支援的加密。")?;
    let count = pdf.pages().len();
    let page = pdf.pages().get(index).ok_or("PDF 沒有這一頁。")?;
    let (width, height) = page.render_dimensions();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0
        || width > 100_000.0 || height > 100_000.0
    {
        return Err("此 PDF 頁面的尺寸超出可預覽範圍。".into());
    }
    let scale = (MAX_PIXELS_PER_SIDE / width.max(height)).min(2.0);
    if (width * scale).min(height * scale) < 1.0 {
        return Err("此 PDF 頁面的長寬比例超出可預覽範圍。".into());
    }
    let settings = InterpreterSettings::default();
    let bitmap = hayro::render(
        page,
        &RenderCache::new(),
        &settings,
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            ..Default::default()
        },
    );
    let source = bitmap.data_as_u8_slice();
    let mut bgra = Vec::with_capacity(source.len());
    for pixel in source.chunks_exact(4) {
        // The rasterizer returns premultiplied RGBA over transparency. Composite
        // it over white before using the resulting pixels in a top-down GDI DIB.
        let white = 255 - pixel[3];
        bgra.extend_from_slice(&[
            pixel[2].saturating_add(white),
            pixel[1].saturating_add(white),
            pixel[0].saturating_add(white),
            255,
        ]);
    }
    let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_text::extract(page, &settings, scale, bitmap.width(), bitmap.height(), &bgra)
    })).unwrap_or_default();
    Ok(RenderedPage {
        width: bitmap.width(),
        height: bitmap.height(),
        page_count: count,
        bgra,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &[u8] = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");

    #[test]
    fn renders_two_distinct_storyboard_pages_as_opaque_bgra() -> Result<()> {
        let first = render_page(PDF.to_vec(), 1)?;
        let second = render_page(PDF.to_vec(), 2)?;
        assert_eq!(first.page_count, 2);
        assert_eq!(second.page_count, 2);
        assert_eq!((first.width, first.height), (second.width, second.height));
        assert!(first.width > 1000);
        assert!(first.height > 1000);
        assert!(f32::from(first.width) <= MAX_PIXELS_PER_SIDE && f32::from(first.height) <= MAX_PIXELS_PER_SIDE);
        for page in [&first, &second] {
            assert_eq!(page.bgra.len(), usize::from(page.width) * usize::from(page.height) * 4);
            assert!(page.bgra.chunks_exact(4).all(|pixel| pixel[3] == 255));
            assert!(page.bgra.chunks_exact(4).any(|pixel| pixel[..3] != [255, 255, 255]));
        }
        assert_ne!(first.bgra, second.bgra);
        Ok(())
    }

    #[test]
    fn selects_actual_decoded_glyphs_at_their_rendered_positions() -> Result<()> {
        let page = render_page(PDF.to_vec(), 2)?;
        assert!(!page.text.is_empty());
        let selected = page.text.select([0.43, 0.639], [0.89, 0.639]).ok_or("text missing")?;
        assert_eq!(selected.quote, "Synthetic shot; selection B");
        assert!((0.43..0.46).contains(&selected.regions[0].x));
        assert!((0.62..0.65).contains(&selected.regions[0].y));
        let reversed = page.text.select([0.89, 0.639], [0.43, 0.639]).ok_or("reverse selection missing")?;
        assert_eq!(reversed.quote, selected.quote);
        assert_eq!(reversed.regions[0], selected.regions[0]);
        let prefix = page.text.select([0.43, 0.639], [0.567, 0.639]).ok_or("prefix missing")?;
        assert_eq!(prefix.quote, "Synthetic");
        assert!(prefix.regions[0].width < selected.regions[0].width);
        assert!(page.text.select([0.43, 0.3], [0.89, 0.3]).is_none());
        assert!(page.text.select([0.43, 0.639], [0.89, 0.3]).is_none());
        Ok(())
    }

    #[test]
    fn refuses_invisible_covered_clipped_and_rotated_words() -> Result<()> {
        let contents = "BT /F1 18 Tf 30 250 Td (Keen text) Tj ET\n\
                        BT /F1 18 Tf 3 Tr 30 205 Td (Hidden text) Tj ET\n\
                        BT /F1 18 Tf 0 Tr 30 160 Td (Covered text) Tj ET\n\
                        1 g 24 140 200 35 re f 0 g\n\
                        q 20 20 2 2 re W n BT /F1 18 Tf 30 115 Td (Clipped text) Tj ET Q\n\
                        BT /F1 18 Tf 0 1 -1 0 50 25 Tm (Rotated) Tj ET";
        let page = render_page(text_pdf(contents), 1)?;
        assert_eq!(page.text.select([0.08, 0.15], [0.8, 0.15]).ok_or("visible text missing")?.quote, "Keen text");
        for y in [0.30, 0.45, 0.60, 0.85] {
            assert!(page.text.select([0.08, y], [0.8, y]).is_none());
        }
        Ok(())
    }

    #[test]
    fn selects_aligned_adjacent_lines_without_crossing_columns_or_tinting_the_gap() -> Result<()> {
        let pdf = text_pdf("BT /F1 18 Tf 30 250 Td (First visible line) Tj ET\n\
                            BT /F1 18 Tf 30 222 Td (Second visible line) Tj ET\n\
                            BT /F1 18 Tf 30 194 Td (Third visible line) Tj ET\n\
                            BT /F1 18 Tf 192 250 Td (Right one) Tj ET\n\
                            BT /F1 18 Tf 192 222 Td (Right two) Tj ET");
        let page = render_page(pdf, 1)?;
        let start = [0.08, 0.15];
        let end = [0.59, 0.24];
        let selected = page.text.select(start, end).ok_or("adjacent text missing")?;
        assert_eq!(selected.quote, "First visible line\nSecond visible line");
        assert_eq!(selected.regions.len(), 2);
        let first = selected.regions[0];
        let second = selected.regions[1];
        assert!(first.y + first.height < second.y);
        assert_eq!(page.text.select(end, start).unwrap().regions, selected.regions);
        assert_eq!(page.text.select(end, start).unwrap().quote, selected.quote);
        let partial = page.text.select([0.235, 0.15], [0.33, 0.333]).ok_or("three lines missing")?;
        assert_eq!(partial.quote.lines().count(), 3);
        assert_eq!(partial.quote.lines().nth(1), Some("Second visible line"));
        assert!(partial.quote.lines().next().unwrap().starts_with("visible"));
        assert!(page.text.select(start, [0.8, 0.24]).is_none());

        let marked = page.highlighted_bgra(&selected.regions).unwrap();
        let offset = |x: f64, y: f64| {
            let (width, height) = (usize::from(page.width), usize::from(page.height));
            4 * (width * (y * height as f64).round() as usize + (x * width as f64).round() as usize)
        };
        let gap = offset(0.2, (first.y + first.height + second.y) / 2.0);
        assert_eq!(&marked[gap..gap + 4], &page.bgra[gap..gap + 4]);
        for region in selected.regions {
            let upper = offset(region.x, region.y);
            let lower = offset(region.x + region.width, region.y + region.height);
            assert_ne!(&marked[upper..lower], &page.bgra[upper..lower]);
        }
        Ok(())
    }

    #[test]
    fn selects_two_unambiguous_columns_in_reading_order_without_tinting_the_gutter() -> Result<()> {
        let page = render_page(columns_pdf([0, 1, 2, 3, 4, 5], ""), 1)?;
        let from = [0.08, 0.24];
        let to = [0.95, 0.24];
        let selection = page.text.select(from, to).ok_or("two columns not selected")?;
        assert_eq!(selection.quote, "Second left\nLast left\nFirst right\nSecond right");
        assert_eq!(selection.regions.len(), 4);
        let [left_second, left_last, right_first, right_second] = selection.regions.as_slice()
            else { unreachable!() };
        assert!(left_second.y < left_last.y && right_first.y < right_second.y);
        assert!(right_first.y < left_last.y);
        for left in [left_second, left_last] {
            assert!(left.x + left.width < 0.5);
        }
        for right in [right_first, right_second] {
            assert!(right.x > 0.5);
        }
        let reverse = page.text.select(to, from).ok_or("reverse drag not selected")?;
        assert_eq!(reverse.quote, selection.quote);
        assert_eq!(reverse.regions, selection.regions);
        let marked = page.highlighted_bgra(&selection.regions).ok_or("missing highlight")?;
        let x = usize::from(page.width) / 2;
        for y in 0..usize::from(page.height) {
            let offset = (y * usize::from(page.width) + x) * 4;
            assert_eq!(&marked[offset..offset + 4], &page.bgra[offset..offset + 4]);
        }
        Ok(())
    }

    #[test]
    fn refuses_cross_column_selection_when_paint_order_or_layout_is_ambiguous() -> Result<()> {
        for pdf in [
            columns_pdf([0, 3, 1, 4, 2, 5], ""),
            columns_pdf([3, 4, 5, 0, 1, 2], ""),
            columns_pdf([0, 1, 2, 3, 4, 5], "BT /F1 18 Tf 136 222 Td (Side) Tj ET"),
            columns_pdf([0, 1, 2, 3, 4, 5], "142 206 9 30 re f"),
        ] {
            let page = render_page(pdf, 1)?;
            assert!(page.text.select([0.08, 0.24], [0.95, 0.24]).is_none());
            let single_column = page.text.select([0.08, 0.24], [0.40, 0.24])
                .ok_or("one-column selection was affected")?;
            assert_eq!(single_column.quote, "Second left");
        }
        Ok(())
    }

    fn columns_pdf(order: [usize; 6], extra: &str) -> Vec<u8> {
        use std::fmt::Write;
        let lines = ["First left", "Second left", "Last left", "First right", "Second right", "Last right"];
        let mut contents = String::new();
        for index in order {
            let x = if index < 3 { 30 } else { 177 };
            let y = 250 - 28 * (index % 3);
            writeln!(contents, "BT /F1 18 Tf {x} {y} Td ({}) Tj ET", lines[index]).unwrap();
        }
        contents.push_str(extra);
        text_pdf(&contents)
    }

    fn text_pdf(contents: &str) -> Vec<u8> {
        use std::fmt::Write;
        let mut pdf = String::from("%PDF-1.4\n");
        let pages = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{contents}\nendstream", contents.len()),
        ];
        let mut offsets = Vec::new();
        for (i, object) in pages.iter().enumerate() {
            offsets.push(pdf.len());
            writeln!(pdf, "{} 0 obj\n{object}\nendobj", i + 1).unwrap();
        }
        let xref = pdf.len();
        pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            writeln!(pdf, "{offset:010} 00000 n ").unwrap();
        }
        write!(pdf, "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").unwrap();
        pdf.into_bytes()
    }

    #[test]
    fn refuses_missing_pages_and_invalid_or_oversized_input() {
        assert!(render_page(PDF.to_vec(), 0).is_err());
        assert!(render_page(PDF.to_vec(), 3).is_err());
        assert!(render_page(b"%PDF-1.7\n".to_vec(), 1).is_err());
        assert!(render_page(vec![0; MAX_PDF_BYTES + 1], 1).is_err());
    }
}
