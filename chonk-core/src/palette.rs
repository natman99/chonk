//! Palette extraction: median-cut over the composite, for "recolor to
//! match the image" workflows. Pure Rust, runs in microseconds on a
//! downsampled sample.

use crate::tiny_skia as sk;

/// Extract up to `n` dominant colors (largest bucket first).
pub fn extract(pm: &sk::Pixmap, n: usize) -> Vec<sk::Color> {
    let n = n.clamp(1, 16);
    // Sample at most ~4096 opaque-ish pixels.
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let step = {
        let total = (w * h) as f64;
        (total.sqrt() / 64.0).max(1.0) as usize
    };
    let mut samples: Vec<[u16; 3]> = Vec::with_capacity(4096);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let p = match pm.pixels().get(y * w + x) {
                Some(p) => p,
                None => continue,
            };
            let a = p.alpha() as u32;
            if a < 32 {
                continue;
            }
            // Undo premultiplication for honest hue buckets.
            let r = (p.red() as u32 * 255 / a).min(255) as u16;
            let g = (p.green() as u32 * 255 / a).min(255) as u16;
            let b = (p.blue() as u32 * 255 / a).min(255) as u16;
            samples.push([r, g, b]);
        }
    }
    if samples.is_empty() {
        return Vec::new();
    }

    // Median cut: repeatedly split the bucket with the widest channel
    // range at that channel's median.
    let mut buckets = vec![samples];
    while buckets.len() < n {
        let (idx, channel) = match buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let mut ranges = [0u16; 3];
                for c in 0..3 {
                    let lo = b.iter().map(|p| p[c]).min().unwrap();
                    let hi = b.iter().map(|p| p[c]).max().unwrap();
                    ranges[c] = hi - lo;
                }
                let ch = (0..3).max_by_key(|&c| ranges[c]).unwrap();
                (i, ch, ranges[ch], b.len())
            })
            .max_by_key(|&(_, ch, range, len)| (range, len))
        {
            Some((i, ch, _, _)) => (i, ch),
            None => break,
        };
        let mut b = buckets.swap_remove(idx);
        b.sort_by_key(|p| p[channel]);
        let mid = b.len() / 2;
        buckets.push(b[..mid].to_vec());
        buckets.push(b[mid..].to_vec());
    }

    // Average each bucket, merging buckets that collapse to the same
    // color (median cut equalizes sizes, so dominance needs the merge).
    let mut out: Vec<(usize, sk::Color)> = Vec::new();
    for b in buckets.iter().filter(|b| !b.is_empty()) {
        let len = b.len();
        let r = (b.iter().map(|p| p[0] as u64).sum::<u64>() as f64 / len as f64).round() as u8;
        let g = (b.iter().map(|p| p[1] as u64).sum::<u64>() as f64 / len as f64).round() as u8;
        let bl = (b.iter().map(|p| p[2] as u64).sum::<u64>() as f64 / len as f64).round() as u8;
        let c = sk::Color::from_rgba8(r, g, bl, 255);
        if let Some(e) = out.iter_mut().find(|(_, oc)| {
            oc.to_color_u8().red() == r
                && oc.to_color_u8().green() == g
                && oc.to_color_u8().blue() == bl
        }) {
            e.0 += len;
        } else {
            out.push((len, c));
        }
    }
    out.sort_by_key(|(len, _)| std::cmp::Reverse(*len));
    out.into_iter().map(|(_, c)| c).collect()
}

/// Pick the palette color with the highest contrast against `bg`
/// (relative-luminance difference), for auto text fill.
pub fn best_contrast(palette: &[sk::Color], bg: sk::Color) -> Option<sk::Color> {
    let lum = |c: sk::Color| {
        let r = c.red();
        let g = c.green();
        let b = c.blue();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let lbg = lum(bg);
    palette
        .iter()
        .max_by(|a, b| (lum(**a) - lbg).abs().partial_cmp(&(&lum(**b) - lbg).abs()).unwrap())
        .copied()
}
