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

#[test]
fn chonk_roundtrip() {
    use chonk_core::save;
    let mut doc = Document::new(120, 80);
    doc.background = chonk_core::document::Background::LinearGradient {
        x1: 0.0,
        y1: 0.0,
        x2: 120.0,
        y2: 80.0,
        stops: vec![(0.0, sk::Color::from_rgba8(20, 20, 40, 255)), (1.0, sk::Color::from_rgba8(200, 60, 20, 255))],
    };
    doc.add_image(solid(40, 30, sk::Color::from_rgba8(0, 255, 0, 255)));
    doc.add_text(
        "hello chonk",
        TextStyle {
            font_family: "Noto Sans".into(),
            font_size: 24.0,
            fill: sk::Color::from_rgba8(255, 255, 0, 255),
            stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 3.0)),
            shadow: Some(chonk_core::document::Shadow {
                dx: 2.0,
                dy: 2.0,
                blur: 0.0,
                color: sk::Color::from_rgba8(0, 0, 0, 128),
            }),
            align: chonk_core::document::TextAlign::Center,
            max_width: Some(100.0),
            fit_box: None,
        },
    );
    doc.layers[1].opacity = 0.5;
    doc.layers[1].visible = false;

    let json = save::to_json(&doc).expect("serialize");
    let doc2 = save::from_json(&json).expect("deserialize");
    assert_eq!(doc2.width, 120);
    assert_eq!(doc2.layers.len(), 2);
    assert!(!doc2.layers[1].visible);
    assert!((doc2.layers[1].opacity - 0.5).abs() < f32::EPSILON);

    // Composite both and compare pixels: the roundtrip must be lossless
    // at the raster level (PNG is lossless; colors go through u8).
    let mut r = chonk_core::render::Renderer::new();
    let a = r.composite_at(&doc, 0);
    let b = r.composite_at(&doc2, 0);
    assert_eq!(a.pixels(), b.pixels(), "roundtrip composite differs");
}

#[test]
fn palette_extraction_finds_dominant() {
    use chonk_core::palette;
    // 3/4 green, 1/4 magenta: the dominant palette color must be green.
    let mut pm = solid(64, 64, sk::Color::TRANSPARENT);
    for y in 0..64 {
        for x in 0..64 {
            let c = if x < 48 {
                sk::Color::from_rgba8(0, 200, 0, 255)
            } else {
                sk::Color::from_rgba8(220, 0, 220, 255)
            };
            let c8 = c.to_color_u8();
            pm.pixels_mut()[y * 64 + x] =
                sk::PremultipliedColorU8::from_rgba(c8.red(), c8.green(), c8.blue(), 255)
                    .unwrap();
        }
    }
    let pal = palette::extract(&pm, 4);
    assert!(!pal.is_empty());
    let u8 = |c: sk::Color| {
        (
            (c.red() * 255.0) as u8,
            (c.green() * 255.0) as u8,
            (c.blue() * 255.0) as u8,
        )
    };
    let (gr, gg, gb) = u8(pal[0]);
    let dom = pal[0];
    assert!(gg > 150 && gr < 100, "dominant should be green, got {dom:?}");
    let _ = gb;
    assert!(pal.iter().any(|c| { let (r, _, b) = u8(*c); r > 150 && b > 150 }), "magenta bucket present");
    // best_contrast against a dark bg picks a bright color.
    let bg = sk::Color::from_rgba8(10, 10, 10, 255);
    let best = palette::best_contrast(&pal, bg).unwrap();
    let (br, bg2, _) = u8(best);
    assert!(bg2 > 150 || br > 150);
}

#[test]
fn template_slots_roundtrip_and_fill() {
    use chonk_core::template;
    use chonk_core::{LayerKind, Slot, SlotKind, TextStyle};
    let mut doc = Document::new(300, 200);
    doc.add_image(solid(300, 200, sk::Color::from_rgba8(30, 30, 30, 255)));
    let t = doc.add_text("placeholder", TextStyle::default());
    t.slot = Some(Slot { name: "caption".into(), kind: SlotKind::Text });
    let img = doc.add_image(solid(100, 80, sk::Color::from_rgba8(255, 0, 0, 255)));
    img.slot = Some(Slot { name: "avatar".into(), kind: SlotKind::Image });

    // Slots survive a .chonk roundtrip.
    let text = chonk_core::save::to_json(&doc).unwrap();
    let mut doc2 = chonk_core::save::from_json(&text).unwrap();
    assert_eq!(template::slot_names(&doc2).len(), 2);

    // Fill by name.
    assert!(template::fill_text(&mut doc2, "caption", "real words"));
    assert!(template::fill_image(&mut doc2, "avatar", &solid(50, 40, sk::Color::from_rgba8(0, 255, 0, 255))));
    assert!(!template::fill_text(&mut doc2, "nope", "x"));
    // Image slot kept its display size (scaled, not swapped raw).
    for l in &doc2.layers {
        if let (Some(s), LayerKind::Image { pixmap }) = (&l.slot, &l.kind) {
            if s.name == "avatar" {
                assert_eq!((pixmap.width(), pixmap.height()), (100, 80));
                // Scaled, not clipped: bottom-right corner must be green.
                let p = pixmap.pixels()[79 * 100 + 99];
                assert!(p.green() > 150, "fill must scale to the slot box, corner={p:?}");
            }
        }
    }
    // Renders fine.
    let pm = chonk_core::render::Renderer::new().composite(&doc2);
    assert_eq!(pm.width(), 300);
}

#[test]
fn deep_fry_changes_pixels_and_keeps_size() {
    use chonk_core::fry::{self, FRY_HEAVY, FRY_LADDER};
    // A gradient so JPEG blocking has real signal to chew on.
    let mut pm = sk::Pixmap::new(128, 128).unwrap();
    for y in 0..128usize {
        for x in 0..128usize {
            let c = sk::Color::from_rgba8((x * 2) as u8, (y * 2) as u8, ((x + y) * 2) as u8, 255);
            let c8 = c.to_color_u8();
            pm.pixels_mut()[y * 128 + x] =
                sk::PremultipliedColorU8::from_rgba(c8.red(), c8.green(), c8.blue(), 255)
                    .unwrap();
        }
    }
    let fried = fry::fry(&pm, &FRY_HEAVY);
    assert_eq!((fried.width(), fried.height()), (128, 128));
    // Deterministic.
    let fried2 = fry::fry(&pm, &FRY_HEAVY);
    assert_eq!(fried.pixels(), fried2.pixels());
    // Actually fried: at least one pixel moved.
    assert_ne!(fried.pixels(), pm.pixels());
    assert_eq!(FRY_LADDER.len(), 4);
}

#[test]
fn fry_depth_survives_roundtrip() {
    let mut doc = Document::new(64, 64);
    doc.add_image(solid(64, 64, sk::Color::from_rgba8(10, 20, 30, 255)));
    doc.layers[0].fry_depth = 3;
    let text = chonk_core::save::to_json(&doc).unwrap();
    let doc2 = chonk_core::save::from_json(&text).unwrap();
    assert_eq!(doc2.layers[0].fry_depth, 3);
    assert_eq!(doc.layers[0].fry_depth, 3);
}
