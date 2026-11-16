//! chonk-quote: headless quote-card generator (the quotes project's use case).
//! Usage: chonk-quote "quote text" "Author" [-o out.png] [-w 1080] [-h 1080]
//!        [--font "Noto Sans"] [--bg1 2b2c5e] [--bg2 e07a2c]

use chonk_core::codecs;
use chonk_core::quote::{QuoteBackground, QuoteSpec, quote_document};
use chonk_core::text::TextEngine;
use chonk_core::tiny_skia as sk;

fn parse_hex(s: &str) -> Option<sk::Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(sk::Color::from_rgba8(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8, 255))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut positional: Vec<String> = Vec::new();
    let mut out = "quote.png".to_string();
    let mut spec = QuoteSpec::new("", "");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                out = args.get(i + 1).cloned().unwrap_or_default();
                i += 1;
            }
            "-w" => {
                spec.width = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(spec.width);
                i += 1;
            }
            "-h" => {
                spec.height = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(spec.height);
                i += 1;
            }
            "--font" => {
                spec.font_family = args.get(i + 1).cloned().unwrap_or_default();
                i += 1;
            }
            "--bg1" => {
                if let Some(c) = args.get(i + 1).and_then(|v| parse_hex(v)) {
                    if let QuoteBackground::Gradient(_, b) = &spec.background {
                        spec.background = QuoteBackground::Gradient(c, *b);
                    }
                }
                i += 1;
            }
            "--bg2" => {
                if let Some(c) = args.get(i + 1).and_then(|v| parse_hex(v)) {
                    if let QuoteBackground::Gradient(a, _) = &spec.background {
                        spec.background = QuoteBackground::Gradient(*a, c);
                    }
                }
                i += 1;
            }
            "-?" | "--help" => {
                eprintln!(
                    "usage: chonk-quote \"quote text\" \"Author\" [-o out.png] [-w W] [-h H] [--font F] [--bg1 HEX] [--bg2 HEX]"
                );
                return;
            }
            a => {
                positional.push(a.to_string());
            }
        }
        i += 1;
    }
    if positional.is_empty() {
        eprintln!("usage: chonk-quote \"quote text\" \"Author\" [-o out.png] ... (see --help)");
        std::process::exit(2);
    }
    spec.text = positional.remove(0);
    spec.author = positional.first().cloned().unwrap_or_default();

    let mut engine = TextEngine::new();
    let doc = quote_document(&mut engine, &spec);
    let mut renderer = chonk_core::Renderer::new();
    // Reuse the engine that already shaped the text (avoid double font load).
    std::mem::swap(&mut renderer.text, &mut engine);
    let pm = renderer.composite(&doc);
    std::fs::write(&out, codecs::export_png(&pm).expect("encode png")).expect("write output");
    println!("wrote {out}");
}
