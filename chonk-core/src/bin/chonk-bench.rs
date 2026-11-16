//! tiny-skia performance probe: is CPU rasterization fast enough for an
//! interactive editor? Budget: a full recomposite must fit a 16.7 ms frame.

use std::time::Instant;

use chonk_core::document::{Background, Document, LayerKind, Shadow, TextAlign, TextStyle};
use chonk_core::{Renderer, tiny_skia as sk};

fn procedural_image(w: u32, h: u32) -> sk::Pixmap {
    let mut pm = sk::Pixmap::new(w, h).unwrap();
    let mut seed: u32 = 0x12345678;
    let mut rng = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 16) as u8
    };
    for y in 0..h {
        for x in 0..w {
            let r = (x as u32 * 255 / w) as u8;
            let g = (y as u32 * 255 / h) as u8;
            let b = rng();
            pm.pixels_mut()[(y * w + x) as usize] =
                sk::PremultipliedColorU8::from_rgba(r, g, b, 255).unwrap();
        }
    }
    pm
}

fn busy_svg(shapes: usize) -> String {
    let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600">"#);
    let mut seed: u32 = 7;
    let mut rng = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) % 600
    };
    for i in 0..shapes {
        match i % 3 {
            0 => s.push_str(&format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="rgb({},60,120)" opacity="0.7"/>"#,
                rng() % 700, rng() % 500, 40 + rng() % 200, 40 + rng() % 150, rng() % 255
            )),
            1 => s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="rgb(30,{},90)" opacity="0.6"/>"#,
                rng() % 800, rng() % 600, 20 + rng() % 120, rng() % 255
            )),
            _ => s.push_str(&format!(
                r#"<path d="M {},{} C {},{} {},{} {},{}" stroke="rgb(200,{},40)" stroke-width="6" fill="none"/>"#,
                rng() % 800, rng() % 600, rng() % 800, rng() % 600,
                rng() % 800, rng() % 600, rng() % 800, rng() % 600, rng() % 255
            )),
        }
    }
    s.push_str("</svg>");
    s
}

fn timed(label: &str, iters: u32, mut f: impl FnMut()) {
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("{label:<44} {ms:>8.2} ms/iter");
}

fn main() {
    let (w, h) = (1920u32, 1080u32);

    let img_a = procedural_image(w, h);
    let img_b = procedural_image(1200, 900);
    let svg_src = busy_svg(150);
    let tree = chonk_core::svg::parse(svg_src.as_bytes()).expect("parse svg");

    let big_text = TextStyle {
        font_family: String::new(),
        font_size: 160.0,
        fill: sk::Color::from_rgba8(255, 255, 255, 255),
        stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 12.0)),
        shadow: Some(Shadow { dx: 8.0, dy: 12.0, blur: 8.0, color: sk::Color::from_rgba8(0, 0, 0, 160) }),
        align: TextAlign::Center,
        max_width: Some(1700.0),
        fit_box: None,
    };
    let quote_text = TextStyle {
        font_size: 96.0,
        fill: sk::Color::from_rgba8(20, 20, 20, 255),
        stroke: None,
        shadow: Some(Shadow { dx: 2.0, dy: 3.0, blur: 4.0, color: sk::Color::from_rgba8(0, 0, 0, 120) }),
        align: TextAlign::Center,
        max_width: Some(1400.0),
        fit_box: Some((1400.0, 500.0)),
        ..TextStyle::default()
    };

    let mut doc = Document::new(w, h);
    doc.background = Background::LinearGradient {
        x1: 0.0, y1: 0.0, x2: w as f32, y2: h as f32,
        stops: vec![(0.0, sk::Color::from_rgba8(40, 20, 90, 255)), (1.0, sk::Color::from_rgba8(220, 90, 40, 255))],
    };
    doc.add_layer(LayerKind::Image { pixmap: img_a });
    let l = doc.add_layer(LayerKind::Image { pixmap: img_b });
    l.transform = chonk_core::Transform { x: 300.0, y: 100.0, scale: 0.6, rotation_deg: 12.0 };
    l.opacity = 0.8;
    doc.add_layer(LayerKind::Svg { tree: tree.clone() });
    doc.add_layer(LayerKind::Text { text: "CHONK MAXIMUM".into(), style: big_text });
    doc.add_layer(LayerKind::Text {
        text: "the quick brown fox jumps over the lazy dog, twice, for good measure".into(),
        style: quote_text,
    });

    let mut r = Renderer::new();

    println!("== chonk-core raster budget (release build) ==");
    timed("svg parse (150 shapes)", 20, || {
        std::hint::black_box(chonk_core::svg::parse(svg_src.as_bytes()).unwrap());
    });
    timed("svg render (150 shapes, 800x600)", 20, || {
        std::hint::black_box(chonk_core::svg::render(&tree));
    });
    timed("text render (160pt + stroke + shadow)", 20, || {
        std::hint::black_box(r.text.render("CHONK MAXIMUM", &TextStyle {
            font_size: 160.0,
            fill: sk::Color::from_rgba8(255, 255, 255, 255),
            stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 12.0)),
            shadow: Some(Shadow { dx: 8.0, dy: 12.0, blur: 8.0, color: sk::Color::from_rgba8(0, 0, 0, 160) }),
            align: TextAlign::Center,
            max_width: Some(1700.0),
            ..TextStyle::default()
        }));
    });
    timed("text render (quote fit_box + wrap + shadow)", 20, || {
        std::hint::black_box(r.text.render(
            "the quick brown fox jumps over the lazy dog, twice, for good measure",
            &TextStyle {
                font_size: 96.0,
                fill: sk::Color::from_rgba8(20, 20, 20, 255),
                shadow: Some(Shadow { dx: 2.0, dy: 3.0, blur: 4.0, color: sk::Color::from_rgba8(0, 0, 0, 120) }),
                align: TextAlign::Center,
                max_width: Some(1400.0),
                fit_box: Some((1400.0, 500.0)),
                ..TextStyle::default()
            },
        ));
    });
    timed("full composite 1920x1080 (5 layers)", 30, || {
        std::hint::black_box(r.composite(&doc));
    });

    // Blit-only: what a drag frame costs if layer rasters are cached.
    let mut blit_doc = Document::new(w, h);
    blit_doc.background = doc.background.clone();
    for l in &doc.layers {
        match &l.kind {
            LayerKind::Image { pixmap } => {
                let nl = blit_doc.add_layer(LayerKind::Image { pixmap: pixmap.clone() });
                nl.transform = l.transform;
                nl.opacity = l.opacity;
            }
            LayerKind::AnimatedImage { .. } => {}
            LayerKind::Svg { tree } => {
                let pm = chonk_core::svg::render(tree);
                let nl = blit_doc.add_layer(LayerKind::Image { pixmap: pm });
                nl.transform = l.transform;
                nl.opacity = l.opacity;
            }
            LayerKind::Text { text, style } => {
                let rt = r.text.render(text, style).unwrap();
                let nl = blit_doc.add_layer(LayerKind::Image { pixmap: rt.pixmap.clone() });
                nl.transform = l.transform;
                nl.opacity = l.opacity;
            }
        }
    }
    timed("cached composite 1920x1080 (blit only)", 60, || {
        std::hint::black_box(r.composite(&blit_doc));
    });

    // 4K stress.
    let mut doc4k = Document::new(3840, 2160);
    doc4k.background = doc.background.clone();
    for l in &blit_doc.layers {
        let nl = doc4k.add_layer(l.kind.clone());
        nl.transform = l.transform;
        nl.opacity = l.opacity;
    }
    timed("cached composite 3840x2160 (blit only)", 20, || {
        std::hint::black_box(r.composite(&doc4k));
    });

    // The interactive path: a drag frame recomposites only the dragged
    // layer's dirty rect and overlays it on the cached full canvas.
    let mut base = r.composite(&blit_doc);
    timed("drag frame 1080p (400x300 dirty rect + overlay)", 100, || {
        let region = r.render_region(&blit_doc, 300, 100, 400, 300);
        Renderer::overlay_region(&mut base, &region, 300, 100);
        std::hint::black_box(&base);
    });
    timed("drag frame 4K (600x400 dirty rect + overlay)", 60, || {
        let region = r.render_region(&doc4k, 300, 100, 600, 400);
        Renderer::overlay_region(&mut base, &region, 300, 100);
        std::hint::black_box(&base);
    });
}
