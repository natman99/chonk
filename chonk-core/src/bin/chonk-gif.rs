//! chonk-gif: caption + effects on animated GIFs.
//! Usage: chonk-gif input.gif "caption text" [-o out.gif]
//!        [--filter blur|gray|pixelate] [--top] [--font-size 64]

use chonk_core::codecs::{export_gif, load_gif};
use chonk_core::document::{Document, TextAlign, TextStyle};
use chonk_core::filters::{Filter, apply_filter_animated};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut positional: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    let mut filter: Option<Filter> = None;
    let mut top = false;
    let mut font_size = 64.0f32;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                out = args.get(i + 1).cloned();
                i += 1;
            }
            "--filter" => {
                filter = match args.get(i + 1).map(|s| s.as_str()) {
                    Some("blur") => Some(Filter::Blur(6.0)),
                    Some("gray") | Some("grayscale") => Some(Filter::Grayscale),
                    Some("pixelate") => Some(Filter::Pixelate(12)),
                    _ => {
                        eprintln!("unknown filter (blur|gray|pixelate)");
                        std::process::exit(2);
                    }
                };
                i += 1;
            }
            "--top" => top = true,
            "--font-size" => {
                font_size =
                    args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(font_size);
                i += 1;
            }
            "-?" | "--help" => {
                eprintln!(
                    "usage: chonk-gif input.gif \"caption\" [-o out.gif] [--filter blur|gray|pixelate] [--top] [--font-size N]"
                );
                return;
            }
            a => positional.push(a.to_string()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        eprintln!("need: input.gif \"caption text\" (see --help)");
        std::process::exit(2);
    }
    let input = positional.remove(0);
    let caption = positional.remove(0);
    let out = out.unwrap_or_else(|| {
        input.replace(".gif", "-captioned.gif")
    });

    let bytes = std::fs::read(&input).expect("read input gif");
    let mut anim = load_gif(&bytes).expect("decode gif");
    if let Some(f) = filter {
        apply_filter_animated(&mut anim, f);
    }
    let (w, h) = (anim.frames[0].width(), anim.frames[0].height());

    let mut doc = Document::new(w, h);
    doc.add_animated(anim);
    let style = TextStyle {
        font_family: String::new(),
        font_size,
        fill: sk::Color::from_rgba8(255, 255, 255, 255),
        stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), font_size / 10.0)),
        align: TextAlign::Center,
        max_width: Some(w as f32 * 0.92),
        fit_box: Some((w as f32 * 0.92, h as f32 * 0.35)),
        ..TextStyle::default()
    };
    let layer = doc.add_text(caption, style);
    layer.transform.y = if top { 8.0 } else { h as f32 - font_size * 1.6 };

    let mut renderer = Renderer::new();
    let (frames, delays) = renderer.composite_animated(&doc);
    std::fs::write(&out, export_gif(&frames, &delays).expect("encode gif"))
        .expect("write output");
    println!("wrote {out} ({} frames)", frames.len());
}
