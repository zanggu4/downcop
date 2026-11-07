//! Procedurally drawn application / tray icon (no binary assets needed).
//!
//! This file is also compiled into `build.rs` (via `#[path]`) to produce the
//! `.ico` that is embedded into the Windows executable, so it must stay
//! self-contained (std only).

#![allow(dead_code)]

/// Icon background colors (RGB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconStyle {
    /// Application icon (brand blue).
    App,
    /// Tray: traffic passes normally.
    Normal,
    /// Tray: at least one direction is speed limited.
    Limited,
    /// Tray: at least one direction is blocked.
    Blocked,
}

impl IconStyle {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            IconStyle::App => (0x25, 0x63, 0xEB),
            IconStyle::Normal => (0x16, 0xA3, 0x4A),
            IconStyle::Limited => (0xF5, 0x9E, 0x0B),
            IconStyle::Blocked => (0xDC, 0x26, 0x26),
        }
    }
}

fn in_rounded_rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// Download arrow dropping into a tray, in unit coordinates.
fn in_glyph(x: f32, y: f32, blocked: bool) -> bool {
    let shaft = (0.42..=0.58).contains(&x) && (0.17..=0.50).contains(&y);
    let head = (0.44..=0.72).contains(&y) && (x - 0.5).abs() <= (0.72 - y);
    let tray = in_rounded_rect(x, y, 0.22, 0.78, 0.78, 0.87, 0.04);
    let glyph = shaft || head || tray;
    if blocked {
        // carve a diagonal gap so the "blocked" state is readable without color
        let d = (x - y).abs() / std::f32::consts::SQRT_2;
        glyph && d > 0.06
    } else {
        glyph
    }
}

/// Renders the icon as straight (non premultiplied) RGBA8, row-major, top-down.
pub fn render_rgba(size: u32, style: IconStyle) -> Vec<u8> {
    const SS: u32 = 4; // supersampling per axis
    let (br, bg, bb) = style.rgb();
    let blocked = style == IconStyle::Blocked;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let inv = 1.0 / (size * SS) as f32;
    for py in 0..size {
        for px in 0..size {
            let (mut cov_bg, mut cov_fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = ((px * SS + sx) as f32 + 0.5) * inv;
                    let y = ((py * SS + sy) as f32 + 0.5) * inv;
                    if in_rounded_rect(x, y, 0.03, 0.03, 0.97, 0.97, 0.22) {
                        cov_bg += 1;
                        if in_glyph(x, y, blocked) {
                            cov_fg += 1;
                        }
                    }
                }
            }
            let n = (SS * SS) as f32;
            let a = cov_bg as f32 / n;
            if cov_bg == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // white glyph blended over the background, then overall coverage
            let f = cov_fg as f32 / cov_bg as f32;
            let mix = |c: u8| (c as f32 * (1.0 - f) + 255.0 * f).round() as u8;
            out.extend_from_slice(&[mix(br), mix(bg), mix(bb), (a * 255.0).round() as u8]);
        }
    }
    out
}

/// Encodes several sizes into a Windows `.ico` file (32-bit BMP entries).
pub fn encode_ico(sizes: &[u32], style: IconStyle) -> Vec<u8> {
    let mut images = Vec::new();
    for &s in sizes {
        let rgba = render_rgba(s, style);
        let mut bmp = Vec::new();
        // BITMAPINFOHEADER
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(s as i32).to_le_bytes());
        bmp.extend_from_slice(&((s * 2) as i32).to_le_bytes()); // XOR + AND masks
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        let mask_stride = s.div_ceil(32) * 4;
        let img_size = s * s * 4 + mask_stride * s;
        bmp.extend_from_slice(&img_size.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 16]);
        // pixels, bottom-up BGRA
        for row in (0..s).rev() {
            for col in 0..s {
                let i = ((row * s + col) * 4) as usize;
                bmp.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        bmp.extend(std::iter::repeat_n(0u8, (mask_stride * s) as usize));
        images.push((s, bmp));
    }

    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, data) in &images {
        let dim = if *s >= 256 { 0u8 } else { *s as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        ico.extend_from_slice(&data);
    }
    ico
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_has_expected_size_and_transparent_corners() {
        let px = render_rgba(32, IconStyle::Normal);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "top-left corner should be transparent");
        // center column near the top of the shaft is white glyph
        let i = ((8 * 32 + 16) * 4) as usize;
        assert_eq!(&px[i..i + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn styles_differ() {
        assert_ne!(render_rgba(16, IconStyle::Normal), render_rgba(16, IconStyle::Blocked));
    }

    #[test]
    fn ico_header_is_valid() {
        let ico = encode_ico(&[16, 32, 256], IconStyle::App);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 3, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[6 + 32], 0); // 256 encoded as 0
        let first_offset = u32::from_le_bytes(ico[18..22].try_into().unwrap());
        assert_eq!(first_offset, 6 + 16 * 3);
    }
}
