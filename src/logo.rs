//! The SpotiLite logo (a feather and sound waves), embedded as a 128 px
//! coverage mask compressed with zlib (4 KB): drawn in white on any background
//! by the interface, and turned into the window icon.

use std::io::Read;

const SIZE: usize = 128;
static MASK: &[u8] = include_bytes!("../assets/logo-mask.zz");

/// Coverage of each pixel (0 to 255), row by row.
fn mask() -> Vec<u8> {
    let mut out = Vec::with_capacity(SIZE * SIZE);
    match flate2::read::ZlibDecoder::new(MASK).read_to_end(&mut out) {
        Ok(_) if out.len() == SIZE * SIZE => out,
        _ => vec![0; SIZE * SIZE],
    }
}

/// The logo in white on transparent, for the interface.
pub fn image() -> egui::ColorImage {
    let pixels = mask().into_iter().map(egui::Color32::from_white_alpha).collect();
    egui::ColorImage::new([SIZE, SIZE], pixels)
}

/// Window icon: the logo in white on a black rounded square.
pub fn window_icon() -> egui::IconData {
    const ICON: usize = 64;
    const LOGO: usize = 46;
    let mask = mask();
    let offset = (ICON - LOGO) / 2;
    let scale = SIZE as f32 / LOGO as f32;
    let radius = 14.0f32;
    let mut rgba = vec![0u8; ICON * ICON * 4];
    for y in 0..ICON {
        for x in 0..ICON {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Signed distance to the rounded square (negative inside).
            let dx = (radius - fx).max(fx - (ICON as f32 - radius)).max(0.0);
            let dy = (radius - fy).max(fy - (ICON as f32 - radius)).max(0.0);
            let alpha = (0.5 - ((dx * dx + dy * dy).sqrt() - radius)).clamp(0.0, 1.0);
            // Average of the mask pixels under this icon pixel.
            let ink = match (x.checked_sub(offset), y.checked_sub(offset)) {
                (Some(lx), Some(ly)) if lx < LOGO && ly < LOGO => {
                    let (x0, x1) = ((lx as f32 * scale) as usize, ((lx + 1) as f32 * scale) as usize);
                    let (y0, y1) = ((ly as f32 * scale) as usize, ((ly + 1) as f32 * scale) as usize);
                    let (mut sum, mut count) = (0u32, 0u32);
                    for my in y0..y1.min(SIZE) {
                        for mx in x0..x1.min(SIZE) {
                            sum += u32::from(mask[my * SIZE + mx]);
                            count += 1;
                        }
                    }
                    (sum / count.max(1)) as u8
                }
                _ => 0,
            };
            let i = (y * ICON + x) * 4;
            rgba[i..i + 3].fill(ink);
            rgba[i + 3] = (alpha * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: ICON as u32, height: ICON as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_decodes_and_has_ink() {
        let mask = mask();
        assert_eq!(mask.len(), SIZE * SIZE);
        let inked = mask.iter().filter(|&&a| a > 128).count();
        assert!(inked > SIZE * SIZE / 20 && inked < SIZE * SIZE / 2, "{inked}");
        let icon = window_icon();
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        // Transparent corner, white ink somewhere.
        assert_eq!(icon.rgba[3], 0);
        assert!(icon.rgba.chunks(4).any(|p| p[0] > 200 && p[3] == 255));
    }
}
