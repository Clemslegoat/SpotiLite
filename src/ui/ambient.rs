//! Background of the player bar: a blurred gradient made from the colors of the
//! current cover. The cover is reduced to 6 × 2 colors, turned into a tiny
//! darkened image, and stretched over the bar with linear filtering: a smooth
//! blur that costs a few hundred bytes and no image processing at draw time.

use egui::{Color32, ColorImage};

const TINT_COLUMNS: usize = 6;
const TINT_ROWS: usize = 2;
const WIDTH: usize = 32;
const HEIGHT: usize = 8;

/// The cover reduced to 6 × 2 average colors (row by row).
pub type Tint = [Color32; TINT_COLUMNS * TINT_ROWS];

/// Key of the liked tracks banner, which has no picture: a fixed deep violet.
pub const LIKED: &str = "builtin:liked";

pub fn liked_tint() -> Tint {
    let mut tint = [Color32::BLACK; TINT_COLUMNS * TINT_ROWS];
    for (i, color) in tint.iter_mut().enumerate() {
        let t = (i % TINT_COLUMNS) as f32 / (TINT_COLUMNS - 1) as f32;
        let lift = if i < TINT_COLUMNS { 1.0 } else { 0.85 };
        let channel = |from: f32, to: f32| ((from + (to - from) * t) * lift) as u8;
        *color = Color32::from_rgb(channel(118.0, 40.0), channel(84.0, 52.0), channel(235.0, 150.0));
    }
    tint
}

pub fn tint_of(image: &ColorImage) -> Tint {
    let [w, h] = image.size;
    let mut tint = [Color32::BLACK; TINT_COLUMNS * TINT_ROWS];
    if w == 0 || h == 0 {
        return tint;
    }
    for row in 0..TINT_ROWS {
        for column in 0..TINT_COLUMNS {
            let (x0, x1) = (
                column * w / TINT_COLUMNS,
                ((column + 1) * w / TINT_COLUMNS).max(column * w / TINT_COLUMNS + 1),
            );
            let (y0, y1) = (row * h / TINT_ROWS, ((row + 1) * h / TINT_ROWS).max(row * h / TINT_ROWS + 1));
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let c = image.pixels[y * w + x];
                    sum[0] += u32::from(c.r());
                    sum[1] += u32::from(c.g());
                    sum[2] += u32::from(c.b());
                    count += 1;
                }
            }
            let count = count.max(1);
            tint[row * TINT_COLUMNS + column] =
                Color32::from_rgb((sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8);
        }
    }
    tint
}

fn sample(tint: &Tint, u: f32, v: f32) -> [f32; 3] {
    let x = u * (TINT_COLUMNS - 1) as f32;
    let y = v * (TINT_ROWS - 1) as f32;
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(TINT_COLUMNS - 1), (y0 + 1).min(TINT_ROWS - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |cx: usize, cy: usize| {
        let c = tint[cy * TINT_COLUMNS + cx];
        [f32::from(c.r()), f32::from(c.g()), f32::from(c.b())]
    };
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    mix(mix(at(x0, y0), at(x1, y0), fx), mix(at(x0, y1), at(x1, y1), fx), fy)
}

/// The tiny gradient image: the cover's colors, a little more saturated, bright
/// near the cover (left) and fading to almost black on the right so that white
/// text and buttons stay readable everywhere.
pub fn ambient_image(tint: &Tint) -> ColorImage {
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT);
    for y in 0..HEIGHT {
        let v = y as f32 / (HEIGHT - 1) as f32;
        for x in 0..WIDTH {
            let u = x as f32 / (WIDTH - 1) as f32;
            let [r, g, b] = sample(tint, u, v);
            let gray = (r + g + b) / 3.0;
            let saturate = |c: f32| gray + (c - gray) * 1.4;
            // Brightest behind the cover, then a long fade.
            let fade = 0.55 - 0.40 * u.powf(0.8);
            let vignette = 0.88 + 0.12 * (1.0 - (v - 0.5).abs() * 2.0);
            let k = fade * vignette;
            let channel = |c: f32| (saturate(c) * k).clamp(0.0, 255.0) as u8;
            pixels.push(Color32::from_rgb(channel(r), channel(g), channel(b)));
        }
    }
    ColorImage::new([WIDTH, HEIGHT], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_averages_regions_and_gradient_fades_right() {
        // Left half red, right half blue.
        let (w, h) = (12, 4);
        let pixels = (0..w * h)
            .map(|i| if i % w < w / 2 { Color32::from_rgb(200, 0, 0) } else { Color32::from_rgb(0, 0, 200) })
            .collect();
        let tint = tint_of(&ColorImage::new([w, h], pixels));
        assert_eq!(tint[0], Color32::from_rgb(200, 0, 0));
        assert_eq!(tint[TINT_COLUMNS - 1], Color32::from_rgb(0, 0, 200));

        let image = ambient_image(&tint);
        assert_eq!(image.size, [WIDTH, HEIGHT]);
        let left = image.pixels[HEIGHT / 2 * WIDTH];
        let right = image.pixels[HEIGHT / 2 * WIDTH + WIDTH - 1];
        assert!(left.r() > 80 && left.b() < 30, "red, bright, near the cover: {left:?}");
        assert!(right.b() < 60 && right.r() < 10, "dark blue far from it: {right:?}");
    }
}
