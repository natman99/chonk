//! Export the app's sample document to a PNG for headless verification.

use chonk_core::document::{Background, Document, Shadow, TextAlign, TextStyle};
use chonk_core::{codecs, Renderer};
use chonk_core::tiny_skia as sk;

fn main() {
    let (w, h) = (1080u32, 1080u32);
    let mut doc = Document::new(w, h);
    doc.background = Background::LinearGradient {
        x1: 0.0,
        y1: 0.0,
        x2: w as f32,
        y2: h as f32,
        stops: vec![
            (0.0, sk::Color::from_rgba8(30, 30, 90, 255)),
            (1.0, sk::Color::from_rgba8(230, 120, 40, 255)),
        ],
    };
    doc.add_text(
        "CHONK",
        TextStyle {
            font_family: "Noto Sans".into(),
            font_size: 260.0,
            fill: sk::Color::from_rgba8(255, 255, 255, 255),
            stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 16.0)),
            shadow: Some(Shadow {
                dx: 10.0,
                dy: 14.0,
                blur: 10.0,
                color: sk::Color::from_rgba8(0, 0, 0, 170),
            }),
            align: TextAlign::Center,
            max_width: Some(1000.0),
            ..TextStyle::default()
        },
    );
    doc.layers[0].transform = chonk_core::Transform { x: 0.0, y: 140.0, ..Default::default() };
    doc.add_text(
        "when the meme opens in a single frame",
        TextStyle {
            font_family: "Noto Sans".into(),
            font_size: 64.0,
            fill: sk::Color::from_rgba8(255, 255, 255, 255),
            stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 6.0)),
            align: TextAlign::Center,
            max_width: Some(900.0),
            fit_box: Some((900.0, 400.0)),
            ..TextStyle::default()
        },
    );
    doc.layers[1].transform = chonk_core::Transform { x: 0.0, y: 640.0, ..Default::default() };

    let mut r = Renderer::new();
    let pm = r.composite(&doc);
    let out = std::env::args().nth(1).unwrap_or_else(|| "chonk-sample.png".into());
    std::fs::write(&out, codecs::export_png(&pm).unwrap()).unwrap();
    println!("wrote {out}");
}
