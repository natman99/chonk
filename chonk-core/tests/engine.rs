use chonk_core::document::{Background, Document, LayerKind, TextAlign, TextStyle};
use chonk_core::text::TextEngine;
use chonk_core::{Renderer, Transform, tiny_skia as sk};

fn solid(w: u32, h: u32, c: sk::Color) -> sk::Pixmap {
    let mut pm = sk::Pixmap::new(w, h).unwrap();
    pm.fill(c);
    pm
}

#[test]
fn solid_background_and_alpha_blend() {
    let mut doc = Document::new(10, 10);
    doc.background = Background::Solid(sk::Color::from_rgba8(0, 0, 255, 255));
    let l = doc.add_image(solid(10, 10, sk::Color::from_rgba8(255, 0, 0, 128)));
    l.opacity = 0.5;
    let out = Renderer::new().composite(&doc);
    let p = out.pixels()[5 * 10 + 5];
    // Layer is 50%-alpha red (premul r=128) drawn at 50% opacity over opaque blue:
    // r = 128*0.5 = 64; blue = 255 * (1 - 0.5*0.5) ≈ 191.
    assert!(p.red() > 55 && p.red() < 75, "red {} (premul over blue)", p.red());
    assert!(p.blue() > 180 && p.blue() < 200, "blue {}", p.blue());
    assert_eq!(p.alpha(), 255);
}

#[test]
fn translate_transform_positions_layer() {
    let mut doc = Document::new(20, 20);
    let l = doc.add_image(solid(10, 10, sk::Color::from_rgba8(255, 0, 0, 255)));
    l.transform = Transform { x: 5.0, y: 5.0, scale: 1.0, rotation_deg: 0.0 };
    let out = Renderer::new().composite(&doc);
    assert_eq!(out.pixels()[4 * 20 + 4].alpha(), 0, "outside must be clear");
    assert_eq!(out.pixels()[10 * 20 + 10].red(), 255, "inside must be red");
    assert_eq!(out.pixels()[10 * 20 + 10].alpha(), 255);
}

#[test]
fn scale_transform_resizes_layer() {
    let mut doc = Document::new(20, 20);
    let l = doc.add_image(solid(10, 10, sk::Color::from_rgba8(0, 255, 0, 255)));
    l.transform = Transform { x: 0.0, y: 0.0, scale: 0.5, rotation_deg: 0.0 };
    let out = Renderer::new().composite(&doc);
    assert_eq!(out.pixels()[2 * 20 + 2].green(), 255, "scaled 5x5 covers (2,2)");
    assert_eq!(out.pixels()[8 * 20 + 8].alpha(), 0, "but not (8,8)");
}

#[test]
fn text_layer_renders_ink() {
    let mut doc = Document::new(400, 120);
    doc.add_text(
        "Chonk",
        TextStyle { font_size: 64.0, fill: sk::Color::from_rgba8(0, 0, 0, 255), ..TextStyle::default() },
    );
    let out = Renderer::new().composite(&doc);
    let inked = out.pixels().iter().filter(|p| p.alpha() > 128).count();
    assert!(inked > 1000, "expected substantial glyph coverage, got {inked}");
}

#[test]
fn fit_box_shrinks_font() {
    let mut r = Renderer::new();
    let style = TextStyle {
        font_size: 200.0,
        max_width: Some(300.0),
        fit_box: Some((300.0, 100.0)),
        ..TextStyle::default()
    };
    let rt = r.text.render("a very long quote that will not fit at 200pt", &style).unwrap();
    assert!(rt.font_size < 200.0, "fit_box must shrink: {}", rt.font_size);
    assert!(rt.pixmap.width() <= 310, "width {}", rt.pixmap.width());
    assert!(rt.pixmap.height() <= 110, "height {}", rt.pixmap.height());
}

#[test]
fn svg_triangle_rasterizes() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
        <polygon points="50,10 90,90 10,90" fill="rgb(255,0,0)"/></svg>"#;
    let tree = chonk_core::svg::parse(svg.as_bytes()).unwrap();
    let pm = chonk_core::svg::render(&tree);
    assert_eq!(pm.width(), 100);
    assert_eq!(pm.pixels()[50 * 100 + 50].red(), 255, "inside triangle red");
    assert_eq!(pm.pixels()[5 * 100 + 5].alpha(), 0, "corner clear");
}

#[test]
fn quote_card_builds_and_renders() {
    let mut engine = TextEngine::new();
    let spec = chonk_core::quote::QuoteSpec::new(
        "The palest ink is better than the best memory.",
        "Chinese proverb",
    );
    let doc = chonk_core::quote::quote_document(&mut engine, &spec);
    assert_eq!(doc.width, 1080);
    assert_eq!(doc.height, 1080);
    // background gradient + quote + author layers
    assert_eq!(doc.layers.len(), 2);
    let mut renderer = Renderer::new();
    std::mem::swap(&mut renderer.text, &mut engine);
    let pm = renderer.composite(&doc);
    // Non-transparent pixels everywhere (gradient bg), and the text is white:
    let center = pm.pixels()[(540 * 1080 + 540) as usize];
    assert!(center.alpha() > 200, "center pixel should be opaque");
    // Author line sits below the quote block.
    if let chonk_core::LayerKind::Text { text, .. } = &doc.layers[1].kind {
        assert!(text.starts_with('\u{2014}'), "author line should start with em dash");
    } else {
        panic!("layer 1 should be text");
    }
    assert!(doc.layers[1].transform.y > doc.layers[0].transform.y);
}

#[test]
fn gif_roundtrip_and_animated_caption() {
    use chonk_core::filters::{Filter, apply_filter_animated};

    // Build a 3-frame gif: red, green, blue.
    let frames: Vec<sk::Pixmap> =
        [0u8, 1, 2].map(|i| solid(64, 64, sk::Color::from_rgba8(i * 100, 1, 1, 255))).to_vec();
    let delays = vec![80u32, 90, 100];
    let bytes = chonk_core::codecs::export_gif(&frames, &delays).unwrap();
    let mut anim = chonk_core::codecs::load_gif(&bytes).unwrap();
    assert_eq!(anim.frame_count(), 3);
    assert_eq!(anim.delays_ms, delays);

    // Grayscale baked into every frame.
    apply_filter_animated(&mut anim, Filter::Grayscale);
    for f in &anim.frames {
        let p = f.pixels()[10 * 64 + 10];
        assert_eq!(p.red(), p.green(), "frame should be gray");
        assert_eq!(p.green(), p.blue());
    }

    // Caption overlay repeats across all frames; gif content animates.
    let mut doc = Document::new(64, 64);
    doc.add_animated(anim);
    let mut renderer = Renderer::new();
    let (out_frames, out_delays) = renderer.composite_animated(&doc);
    assert_eq!(out_frames.len(), 3);
    assert_eq!(out_delays.len(), 3);
    let px = |pm: &sk::Pixmap| pm.pixels()[32 * 64 + 32];
    assert_ne!(px(&out_frames[0]), px(&out_frames[1]), "frames should differ");
    // Re-encode must still be a valid 3-frame gif.
    let bytes2 = chonk_core::codecs::export_gif(&out_frames, &out_delays).unwrap();
    assert_eq!(chonk_core::codecs::load_gif(&bytes2).unwrap().frame_count(), 3);
}

#[test]
fn png_roundtrip() {
    let mut doc = Document::new(64, 64);
    doc.background = Background::Solid(sk::Color::from_rgba8(10, 200, 30, 255));
    let out = Renderer::new().composite(&doc);
    let bytes = chonk_core::codecs::export_png(&out).unwrap();
    let back = chonk_core::codecs::load_image(&bytes).unwrap();
    assert_eq!((back.width(), back.height()), (64, 64));
    let p = back.pixels()[32 * 64 + 32];
    assert!(p.red() < 30 && p.green() > 180 && p.blue() < 50, "{p:?}");
}

#[test]
fn jpeg_and_webp_export() {
    let mut doc = Document::new(64, 64);
    doc.background = Background::Solid(sk::Color::from_rgba8(128, 64, 192, 255));
    let out = Renderer::new().composite(&doc);
    let jpg = chonk_core::codecs::export_jpeg(&out, 90).unwrap();
    let webp = chonk_core::codecs::export_webp(&out).unwrap();
    assert!(!jpg.is_empty() && !webp.is_empty());
    let back = chonk_core::codecs::load_image(&jpg).unwrap();
    let p = back.pixels()[32 * 64 + 32];
    assert!((p.red() as i32 - 128).abs() < 12, "jpeg red {}", p.red());
}

#[test]
fn gradient_background_matches_stops() {
    let mut doc = Document::new(100, 10);
    doc.background = Background::LinearGradient {
        x1: 0.0, y1: 0.0, x2: 99.0, y2: 0.0,
        stops: vec![
            (0.0, sk::Color::from_rgba8(0, 0, 0, 255)),
            (1.0, sk::Color::from_rgba8(255, 255, 255, 255)),
        ],
    };
    let out = Renderer::new().composite(&doc);
    assert!(out.pixels()[5 * 100].red() < 10, "left stop black");
    assert!(out.pixels()[5 * 100 + 99].red() > 245, "right stop white");
}

#[test]
fn render_region_matches_full_composite_crop() {
    let mut doc = Document::new(100, 100);
    doc.background = Background::Solid(sk::Color::from_rgba8(10, 10, 10, 255));
    let l = doc.add_image(solid(40, 40, sk::Color::from_rgba8(255, 0, 0, 255)));
    l.transform = Transform { x: 20.0, y: 20.0, ..Default::default() };
    let mut r = Renderer::new();
    let full = r.composite(&doc);
    let region = r.render_region(&doc, 10, 10, 60, 60);
    assert_eq!((region.width(), region.height()), (60, 60));
    for y in 0..60 {
        for x in 0..60 {
            assert_eq!(
                region.pixels()[y * 60 + x],
                full.pixels()[(y + 10) * 100 + (x + 10)],
                "region pixel ({x},{y}) differs from full crop"
            );
        }
    }
}

#[test]
fn blur_filter_reduces_contrast() {
    let pm = solid(64, 64, sk::Color::from_rgba8(255, 0, 0, 255));
    let mut half = solid(64, 64, sk::Color::TRANSPARENT);
    for y in 0..32 {
        for x in 0..64 {
            half.pixels_mut()[(y * 64 + x) as usize] =
                sk::PremultipliedColorU8::from_rgba(255, 0, 0, 255).unwrap();
        }
    }
    let blurred = chonk_core::filters::blur_pixmap(&half, 8.0);
    // Hard edge at y=31/32 must become a gradient.
    assert!(blurred.pixels()[31 * 64 + 32].alpha() < 255);
    assert!(blurred.pixels()[32 * 64 + 32].alpha() > 0);
    let _ = pm;
}

#[test]
fn composite_scaled_preview() {
    use chonk_core::render::Renderer;
    let mut doc = Document::new(200, 100);
    doc.background = chonk_core::document::Background::Solid(sk::Color::from_rgba8(10, 200, 10, 255));
    let mut r = Renderer::new();
    let pm = r.composite_scaled(&doc, 0, 0.5);
    assert_eq!((pm.width(), pm.height()), (100, 50));
    let c = pm.pixels()[25 * 100 + 50];
    assert!(c.red() < 30 && c.green() > 180, "bg downsampled: {c:?}");
    // scale >= 1 falls through to full resolution.
    let full = r.composite_scaled(&doc, 0, 1.0);
    assert_eq!((full.width(), full.height()), (200, 100));
}
