//! Deep-fry engine: authentic JPEG round-trips, scale resampling,
//! contrast/saturation blowout, and sensor-ish noise. Every pass rides
//! the real codec (codecs::export_jpeg -> load_image), so the artifacts
//! are the genuine article, not a shader impression of them.

use crate::codecs;
use crate::filters;
use crate::tiny_skia as sk;

#[derive(Clone, Copy, Debug)]
pub struct FrySpec {
    /// JPEG quality for each compression pass (1..=100).
    pub jpeg_quality: u8,
    /// Number of full downscale -> compress -> upscale passes.
    pub passes: u32,
    /// Resample factor per pass (0.5 = halve, then blow back up).
    pub scale: f32,
    /// Box blur radius applied after compression (0 = none).
    pub blur: f32,
    /// Contrast multiplier around mid-gray (1 = unchanged).
    pub contrast: f32,
    /// Saturation multiplier (1 = unchanged; >1 = deep-fry blowout).
    pub saturation: f32,
    /// Per-channel noise amplitude (0..=255).
    pub noise: u8,
}

/// Preset ladder. `F` walks down it; each rung is meaner than the last.
pub const FRY_MILD: FrySpec = FrySpec {
    jpeg_quality: 50,
    passes: 1,
    scale: 0.75,
    blur: 0.0,
    contrast: 1.08,
    saturation: 1.15,
    noise: 0,
};
pub const FRY_MEDIUM: FrySpec = FrySpec {
    jpeg_quality: 30,
    passes: 1,
    scale: 0.5,
    blur: 0.6,
    contrast: 1.18,
    saturation: 1.35,
    noise: 4,
};
pub const FRY_HEAVY: FrySpec = FrySpec {
    jpeg_quality: 15,
    passes: 2,
    scale: 0.5,
    blur: 0.8,
    contrast: 1.3,
    saturation: 1.6,
    noise: 8,
};
pub const FRY_ULTRA: FrySpec = FrySpec {
    jpeg_quality: 5,
    passes: 3,
    scale: 0.4,
    blur: 1.0,
    contrast: 1.45,
    saturation: 1.9,
    noise: 14,
};

pub const FRY_LADDER: [FrySpec; 4] = [FRY_MILD, FRY_MEDIUM, FRY_HEAVY, FRY_ULTRA];
pub const FRY_NAMES: [&str; 4] = ["mild", "medium", "heavy", "ultra"];

/// Deep-fry `pm` per `spec`. Deterministic (fixed noise seed) so undo
/// replays and tests are stable.
pub fn fry(pm: &sk::Pixmap, spec: &FrySpec) -> sk::Pixmap {
    let (w, h) = (pm.width(), pm.height());
    let mut cur = pm.clone();

    for _ in 0..spec.passes.max(1) {
        // Downscale (the artifact generator: real codec at low quality
        // on a small image, then blown back to full size).
        let sw = ((w as f32 * spec.scale) as u32).max(8);
        let sh = ((h as f32 * spec.scale) as u32).max(8);
        let mut small = match sk::Pixmap::new(sw, sh) {
            Some(p) => p,
            None => return cur,
        };
        let mut paint = sk::PixmapPaint::default();
        paint.quality = sk::FilterQuality::Bilinear;
        // draw_pixmap blits 1:1; the scale lives in the transform.
        let down = sk::Transform::from_scale(sw as f32 / w as f32, sh as f32 / h as f32);
        small.draw_pixmap(0, 0, cur.as_ref(), &paint, down, None);

        // Genuine JPEG round-trip.
        if let Ok(bytes) = codecs::export_jpeg(&small, spec.jpeg_quality) {
            if let Ok(back) = codecs::load_image(&bytes) {
                small = back;
            }
        }

        // Blow back up to the original size — this is where the 8x8
        // blocks become visible scars.
        let mut big = match sk::Pixmap::new(w, h) {
            Some(p) => p,
            None => return cur,
        };
        let up = sk::Transform::from_scale(w as f32 / sw as f32, h as f32 / sh as f32);
        big.draw_pixmap(0, 0, small.as_ref(), &paint, up, None);
        cur = big;
    }

    if spec.blur > 0.0 {
        cur = filters::blur_pixmap(&cur, spec.blur);
    }

    if spec.contrast != 1.0 || spec.saturation != 1.0 || spec.noise > 0 {
        let mut img = codecs::unpremultiply(&cur);
        // Fixed-seed xorshift: deterministic artifacts.
        let mut rng: u32 = 0x9E3779B9;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        for px in img.pixels_mut() {
            let mut r = px[0] as f32;
            let mut g = px[1] as f32;
            let mut b = px[2] as f32;
            // Contrast around mid-gray.
            r = ((r - 127.5) * spec.contrast + 127.5).clamp(0.0, 255.0);
            g = ((g - 127.5) * spec.contrast + 127.5).clamp(0.0, 255.0);
            b = ((b - 127.5) * spec.contrast + 127.5).clamp(0.0, 255.0);
            // Saturation: push away from luma.
            if spec.saturation != 1.0 {
                let l = 0.299 * r + 0.587 * g + 0.114 * b;
                r = (l + (r - l) * spec.saturation).clamp(0.0, 255.0);
                g = (l + (g - l) * spec.saturation).clamp(0.0, 255.0);
                b = (l + (b - l) * spec.saturation).clamp(0.0, 255.0);
            }
            // Noise.
            if spec.noise > 0 {
                let amp = spec.noise as f32;
                r = (r + ((next() >> 16) as f32 / u16::MAX as f32 - 0.5) * 2.0 * amp).clamp(0.0, 255.0);
                g = (g + ((next() >> 16) as f32 / u16::MAX as f32 - 0.5) * 2.0 * amp).clamp(0.0, 255.0);
                b = (b + ((next() >> 16) as f32 / u16::MAX as f32 - 0.5) * 2.0 * amp).clamp(0.0, 255.0);
            }
            px[0] = r as u8;
            px[1] = g as u8;
            px[2] = b as u8;
        }
        cur = codecs::rgba_to_pixmap(&img);
    }
    cur
}
