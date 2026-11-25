use crate::store::PdfRegion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PageBox {
    pub fn fit(bounds: [i32; 4], raster: [u16; 2]) -> Option<Self> {
        let [left, top, right, bottom] = bounds;
        let available_width = right.checked_sub(left)?;
        let available_height = bottom.checked_sub(top)?;
        let [width, height] = raster;
        if available_width <= 0 || available_height <= 0 || width == 0 || height == 0 {
            return None;
        }
        let scale = (f64::from(available_width) / f64::from(width))
            .min(f64::from(available_height) / f64::from(height));
        let width = ((f64::from(width) * scale).round() as i32).clamp(1, available_width);
        let height = ((f64::from(height) * scale).round() as i32).clamp(1, available_height);
        Some(Self {
            x: left + (available_width - width) / 2,
            y: top + (available_height - height) / 2,
            width, height,
        })
    }

    pub fn contains(self, point: [i32; 2]) -> bool {
        (self.x..self.x+self.width).contains(&point[0])
            && (self.y..self.y+self.height).contains(&point[1])
    }

    pub fn selection(self, from: [i32; 2], to: [i32; 2]) -> Option<PdfRegion> {
        let [ax, ay] = self.offset(from);
        let [bx, by] = self.offset(to);
        let width = (ax - bx).abs();
        let height = (ay - by).abs();
        if width < 3 || height < 3 {
            return None;
        }
        // Divide the bounding corners first to keep both endpoints within 0..1.
        let left = f64::from(ax.min(bx)) / f64::from(self.width);
        let top = f64::from(ay.min(by)) / f64::from(self.height);
        let right = f64::from(ax.max(bx)) / f64::from(self.width);
        let bottom = f64::from(ay.max(by)) / f64::from(self.height);
        Some(PdfRegion {
            x: left, y: top,
            width: (right - left).min(1. - left),
            height: (bottom - top).min(1. - top),
        })
    }

    pub fn rectangle(self, region: PdfRegion) -> [i32; 4] {
        let x = |value: f64| self.x + (value * f64::from(self.width)).round() as i32;
        let y = |value: f64| self.y + (value * f64::from(self.height)).round() as i32;
        [x(region.x), y(region.y), x(region.x + region.width), y(region.y + region.height)]
    }

    fn offset(self, point: [i32; 2]) -> [i32; 2] {
        [point[0].saturating_sub(self.x).clamp(0, self.width),
         point[1].saturating_sub(self.y).clamp(0, self.height)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_aligns_after_viewport_changes() {
        let first = PageBox::fit([0, 0, 500, 400], [500, 400]).unwrap();
        assert!(first.contains([450, 360]));
        let region = first.selection([450, 360], [50, 40]).unwrap();
        assert_eq!(first.rectangle(region), [50, 40, 450, 360]);
        let enlarged = PageBox::fit([40, 14, 1140, 814], [500, 400]).unwrap();
        assert_eq!(enlarged.rectangle(region), [190, 94, 990, 734]);
        assert!(first.selection([50, 40], [50, 43]).is_none());
        let clamped = first.selection([50, 40], [900, 900]).unwrap();
        assert_eq!(first.rectangle(clamped), [50, 40, 500, 400]);
    }
}
