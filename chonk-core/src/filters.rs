//! CPU filters over premultiplied pixmaps (blur is also used for text shadows).

use tiny_skia as sk;

use crate::codecs::{pixmap_to_rgba, rgba_to_pixmap};

pub fn blur_pixmap(pm: &sk::Pixmap, radius: f32) -> sk::Pixmap {
    let img = pixmap_to_rgba(pm);
    let blurred = imageproc::filter::gaussian_blur_f32(&img, radius);
    rgba_to_pixmap(&blurred)
}

pub fn grayscale(pm: &sk::Pixmap) -> sk::Pixmap {
    // Luminance on premultiplied components stays compositer-correct.
    let mut img = pixmap_to_rgba(pm);
    for px in img.pixels_mut() {
        let [r, g, b, a] = px.0;
        let l = (19595 * r as u32 + 38470 * g as u32 + 7471 * b as u32) >> 16;
        *px = image::Rgba([l as u8, l as u8, l as u8, a]);
    }
    rgba_to_pixmap(&img)
}

pub fn pixelate(pm: &sk::Pixmap, factor: u32) -> sk::Pixmap {
    let factor = factor.max(2);
    let small = image::imageops::resize(
        &pixmap_to_rgba(pm),
        (pm.width() / factor).max(1),
        (pm.height() / factor).max(1),
        image::imageops::FilterType::Nearest,
    );
    let big = image::imageops::resize(&small, pm.width(), pm.height(), image::imageops::FilterType::Nearest);
    rgba_to_pixmap(&big)
}
