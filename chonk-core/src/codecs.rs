//! Codec bridge: image crate <-> tiny-skia Pixmap, and export encoders.
//!
//! Convention: `Pixmap` pixels are always PREMULTIPLIED alpha (tiny-skia's
//! requirement); `RgbaImage` buffers crossing this module are also premultiplied
//! unless the function name says otherwise.

use tiny_skia as sk;

pub fn load_image(bytes: &[u8]) -> Result<sk::Pixmap, image::ImageError> {
    let img = image::load_from_memory(bytes)?.to_rgba8();
    Ok(premultiply(&img))
}

/// Straight-alpha RgbaImage -> premultiplied Pixmap.
pub fn premultiply(img: &image::RgbaImage) -> sk::Pixmap {
    let (w, h) = img.dimensions();
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for (i, px) in img.pixels().enumerate() {
        let [r, g, b, a] = px.0;
        let o = i * 4;
        buf[o] = ((r as u32 * a as u32 + 127) / 255) as u8;
        buf[o + 1] = ((g as u32 * a as u32 + 127) / 255) as u8;
        buf[o + 2] = ((b as u32 * a as u32 + 127) / 255) as u8;
        buf[o + 3] = a;
    }
    sk::Pixmap::from_vec(buf, sk::IntSize::from_wh(w, h).expect("non-zero image")).expect("buffer sized to pixmap")
}

/// Premultiplied Pixmap -> straight-alpha RgbaImage.
pub fn unpremultiply(pm: &sk::Pixmap) -> image::RgbaImage {
    let (w, h) = (pm.width(), pm.height());
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (i, px) in pm.pixels().iter().enumerate() {
        let a = px.alpha();
        let o = i * 4;
        if a == 0 {
            out[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
        } else {
            out[o] = ((px.red() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            out[o + 1] = ((px.green() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            out[o + 2] = ((px.blue() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            out[o + 3] = a;
        }
    }
    image::RgbaImage::from_raw(w, h, out).expect("buffer sized to image")
}

/// Premultiplied Pixmap -> premultiplied RgbaImage (for filtering in place).
pub fn pixmap_to_rgba(pm: &sk::Pixmap) -> image::RgbaImage {
    let (w, h) = (pm.width(), pm.height());
    image::RgbaImage::from_raw(w, h, pm.data().to_vec()).expect("buffer sized to image")
}

pub fn rgba_to_pixmap(img: &image::RgbaImage) -> sk::Pixmap {
    let (w, h) = img.dimensions();
    sk::Pixmap::from_vec(img.as_raw().clone(), sk::IntSize::from_wh(w, h).expect("non-zero image"))
        .expect("buffer sized to pixmap")
}

pub fn export_png(pm: &sk::Pixmap) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder as _;
    let straight = unpremultiply(pm);
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(straight.as_raw(), pm.width(), pm.height(), image::ExtendedColorType::Rgba8)?;
    Ok(out)
}

pub fn export_jpeg(pm: &sk::Pixmap, quality: u8) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder as _;
    // Flatten premultiplied onto white: out = premul + white * (1 - a).
    let mut rgb = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
    for px in pm.pixels() {
        let inv = 255 - px.alpha();
        rgb.push(px.red() + inv);
        rgb.push(px.green() + inv);
        rgb.push(px.blue() + inv);
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(&rgb, pm.width(), pm.height(), image::ExtendedColorType::Rgb8)?;
    Ok(out)
}

/// WebP export. NOTE: image-webp 0.2 only encodes *lossless* WebP; a lossy
/// encoder (quality knob) is a future swap once one is available in pure Rust.
pub fn export_webp(pm: &sk::Pixmap) -> Result<Vec<u8>, image::ImageError> {
    let straight = unpremultiply(pm);
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out)
        .encode(straight.as_raw(), pm.width(), pm.height(), image_webp::ColorType::Rgba8)
        .map_err(|e| {
            image::ImageError::from(std::io::Error::other(e.to_string()))
        })?;
    Ok(out)
}
