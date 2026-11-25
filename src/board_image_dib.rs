//! Bounded decoders for ordinary packed Win32 clipboard formats.
use crate::store::Result;

pub(crate) const MAX_DIB_BYTES: usize = 34 * 1024 * 1024;
pub(crate) const MAX_DROP_BYTES: usize = 256 * 1024;
const MAX_PIXELS: usize = 8 * 1024 * 1024;
const MAX_SIDE: usize = 4096;
const MAX_DROP_FILES: usize = 8;

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
}

/// 24-bit BGR and 32-bit BGRX/BGRA, including native bottom-up scanlines.
/// Reject color tables, unusual masks and embedded/linked color profiles.
pub(crate) fn decode_clipboard_dib(data: &[u8], v5_format: bool) -> Result<image::RgbaImage> {
    let unsupported = "剪貼簿點陣圖格式或尺寸不支援。";
    let bad = "剪貼簿點陣圖不完整。";
    if data.len() > MAX_DIB_BYTES { return Err(unsupported.into()) }
    let header = read_u32(data, 0).ok_or(bad)? as usize;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) || (v5_format && header != 124) {
        return Err(unsupported.into());
    }
    if data.len() < header { return Err(bad.into()) }
    let w = read_u32(data, 4).ok_or(bad)? as i32;
    let h = read_u32(data, 8).ok_or(bad)? as i32;
    let width = usize::try_from(w).map_err(|_| unsupported)?;
    let height = h.checked_abs().ok_or(unsupported)? as usize;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE ||
        width.checked_mul(height).is_none_or(|pixels| pixels > MAX_PIXELS)
    {
        return Err(unsupported.into());
    }
    let bits = read_u16(data, 14).ok_or(bad)?;
    let compression = read_u32(data, 16).ok_or(bad)?;
    if read_u16(data, 12) != Some(1) || read_u32(data, 32) != Some(0) ||
        (header == 124 && (read_u32(data, 112) != Some(0) || read_u32(data, 116) != Some(0) ||
            matches!(read_u32(data, 56), Some(0x4c494e4b | 0x4d424544))))
    {
        return Err(unsupported.into());
    }
    let (offset, alpha) = match (bits, compression) {
        (24 | 32, 0) => (header, false), // BI_RGB's unused byte has no alpha contract.
        (32, 3 | 6) => {
            let (offset, alpha) = if header == 40 {
                if compression == 6 { (56, read_u32(data, 52).ok_or(bad)?) } else { (52, 0) }
            } else if header >= 56 {
                (header, read_u32(data, 52).ok_or(bad)?)
            } else { (header, 0) };
            let masks = [read_u32(data, 40), read_u32(data, 44), read_u32(data, 48)];
            if masks != [Some(0x00ff0000), Some(0x0000ff00), Some(0x000000ff)] ||
                !matches!(alpha, 0 | 0xff000000) || (compression == 6 && alpha == 0)
            {
                return Err(unsupported.into());
            }
            (offset, alpha != 0)
        }
        _ => return Err(unsupported.into()),
    };
    let bytes_per_pixel = usize::from(bits / 8);
    let stride = (width * bytes_per_pixel + 3) & !3;
    let pixel_bytes = stride * height;
    let declared = read_u32(data, 20).ok_or(bad)? as usize;
    if declared != 0 && (declared < pixel_bytes || declared > data.len().saturating_sub(offset)) {
        return Err(bad.into());
    }
    let pixels = data.get(offset..offset + pixel_bytes).ok_or(bad)?;
    let mut rgba = vec![0; width * height * 4];
    for (y, destination) in rgba.chunks_exact_mut(width * 4).enumerate() {
        let row = if h < 0 { y } else { height - 1 - y };
        let source = &pixels[row * stride..row * stride + width * bytes_per_pixel];
        for (out, bgr) in destination.chunks_exact_mut(4).zip(source.chunks_exact(bytes_per_pixel)) {
            out.copy_from_slice(&[bgr[2], bgr[1], bgr[0], if alpha { bgr[3] } else { 255 }]);
        }
    }
    image::RgbaImage::from_raw(width as u32, height as u32, rgba).ok_or_else(|| bad.into())
}

/// CF_HDROP contains a DROPFILES header followed by UTF-16 paths and a double nul.
/// Only ordinary Unicode Explorer transfers enter the local-path validator.
pub(crate) fn decode_dropfiles(data: &[u8]) -> Result<Vec<String>> {
    let bad = "剪貼簿圖片檔案清單無效，請從檔案總管重新複製。";
    if data.len() > MAX_DROP_BYTES || read_u32(data, 16).is_none_or(|wide| wide == 0) {
        return Err(bad.into());
    }
    let start = read_u32(data, 0).ok_or(bad)? as usize;
    if start < 20 || start % 2 != 0 {
        return Err(bad.into());
    }
    let mut paths = Vec::new();
    let mut path = Vec::new();
    for word in data.get(start..).ok_or(bad)?.chunks_exact(2) {
        let unit = u16::from_le_bytes([word[0], word[1]]);
        if unit == 0 {
            if path.is_empty() {
                return if paths.is_empty() { Err(bad.into()) } else { Ok(paths) };
            }
            if paths.len() == MAX_DROP_FILES {
                return Err("一次最多貼上 8 個本機圖片檔案。".into());
            }
            paths.push(String::from_utf16(&path).map_err(|_| bad)?);
            path.clear();
        } else {
            if path.len() == 32767 { return Err(bad.into()) }
            path.push(unit);
        }
    }
    Err(bad.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(bytes: usize, width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut data = vec![0; bytes];
        data[..4].copy_from_slice(&(bytes as u32).to_le_bytes());
        data[4..8].copy_from_slice(&width.to_le_bytes());
        data[8..12].copy_from_slice(&height.to_le_bytes());
        data[12..14].copy_from_slice(&1_u16.to_le_bytes());
        data[14..16].copy_from_slice(&bits.to_le_bytes());
        data[16..20].copy_from_slice(&compression.to_le_bytes());
        data
    }

    #[test]
    fn reads_dib_padding_orientation_and_explicit_v5_alpha() -> Result<()> {
        let mut dib = header(40, 1, 2, 24, 0);
        dib.extend_from_slice(&[255, 0, 0, 17, 0, 0, 255, 99]);
        assert_eq!(decode_clipboard_dib(&dib, false)?.as_raw(), &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(decode_clipboard_dib(&dib, true).is_err());
        let mut v5 = header(124, 1, -1, 32, 3);
        for (index, mask) in [0x00ff0000_u32, 0x0000ff00, 0x000000ff, 0xff000000].into_iter().enumerate() {
            v5[40 + 4 * index..44 + 4 * index].copy_from_slice(&mask.to_le_bytes());
        }
        v5.extend_from_slice(&[30, 20, 10, 80]);
        assert_eq!(decode_clipboard_dib(&v5, true)?.as_raw(), &[10, 20, 30, 80]);
        v5[16..20].fill(0);
        assert_eq!(decode_clipboard_dib(&v5, true)?.as_raw(), &[10, 20, 30, 255]);
        v5[112] = 1;
        assert!(decode_clipboard_dib(&v5, true).is_err());
        assert!(decode_clipboard_dib(&dib[..dib.len() - 1], false).is_err());
        assert!(decode_clipboard_dib(&header(40, 8192, 8192, 32, 0), false).is_err());
        assert!(decode_clipboard_dib(&header(40, 1, i32::MIN, 32, 0), false).is_err());
        Ok(())
    }

    #[test]
    fn hdrop_preserves_unicode_requires_termination_and_rejects_invalid_offsets() -> Result<()> {
        let mut drop = vec![0; 20];
        drop[0] = 20;
        drop[16] = 1;
        for unit in "C:\\合成圖.png\0D:\\第二張.jpg\0\0".encode_utf16() {
            drop.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_dropfiles(&drop)?, ["C:\\合成圖.png", "D:\\第二張.jpg"]);
        assert!(decode_dropfiles(&drop[..drop.len() - 2]).is_err());
        drop[0] = 21;
        assert!(decode_dropfiles(&drop).is_err());
        Ok(())
    }
}
