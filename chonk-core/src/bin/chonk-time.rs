//! chonk-time: where does the >1s quote toggle go?

use chonk_core::document::{Background, Document, Shadow, TextAlign, TextStyle};
use chonk_core::quote::{QuoteSpec, quote_document};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;
use std::time::Instant;

fn meme_doc() -> Document {
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
            fit_box: None,
            ..TextStyle::default()
        },
    );
    doc
}

fn main() {
    let t = Instant::now();
    let mut r = Renderer::new();
    println!("Renderer::new (FontSystem load): {:?}", t.elapsed());

    let t = Instant::now();
    let qdoc = quote_document(
        &mut r.text,
        &QuoteSpec::new("It is not enough to succeed; others must fail.", "Caligula"),
    );
    println!("quote_document (measure+fit): {:?}", t.elapsed());

    let t = Instant::now();
    let _ = r.composite(&qdoc);
    println!("composite quote (re-renders text): {:?}", t.elapsed());

    let t = Instant::now();
    let _ = r.composite(&qdoc);
    println!("composite quote again (cold cache? there is none): {:?}", t.elapsed());

    let t = Instant::now();
    let _ = r.text.render("It is not enough to succeed; others must fail.", &default_style());
    println!("text render alone (cache MISS): {:?}", t.elapsed());
    let t = Instant::now();
    let _ = r.text.render("It is not enough to succeed; others must fail.", &default_style());
    println!("text render alone (cache HIT): {:?}", t.elapsed());

    let bgonly = {
        let mut d = Document::new(1080, 1080);
        d.background = Background::Solid(sk::Color::from_rgba8(10, 10, 10, 255));
        d
    };
    let t = Instant::now();
    let _ = r.composite(&bgonly);
    println!("composite bg-only 1080x1080 (blit floor): {:?}", t.elapsed());

    let mdoc = meme_doc();
    let t = Instant::now();
    let _ = r.composite(&mdoc);
    println!("composite meme FIRST (cache miss): {:?}", t.elapsed());
    let t = Instant::now();
    let _ = r.composite(&mdoc);
    println!("composite meme SECOND time (cache hit): {:?}", t.elapsed());
}

fn default_style() -> TextStyle {
    TextStyle {
        font_family: String::new(),
        font_size: 100.0,
        fill: sk::Color::from_rgba8(255, 255, 255, 255),
        align: TextAlign::Center,
        max_width: Some(800.0),
        fit_box: Some((800.0, 600.0)),
        ..TextStyle::default()
    }
}
