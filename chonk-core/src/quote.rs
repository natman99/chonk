//! Quote cards: quote text + attribution, auto-fit and centered over a
//! gradient/solid/image background. Built on the same Document model as memes,
//! so the GUI edits them identically.

use tiny_skia as sk;

use crate::document::{Background, Document, TextAlign, TextStyle, Transform};
use crate::text::TextEngine;

#[derive(Clone, Debug)]
pub enum QuoteBackground {
    Solid(sk::Color),
    /// Diagonal corner-to-corner gradient.
    Gradient(sk::Color, sk::Color),
    /// Image scaled to cover the canvas (added as the bottom layer).
    Image(sk::Pixmap),
}

#[derive(Clone, Debug)]
pub struct QuoteSpec {
    pub text: String,
    pub author: String,
    pub width: u32,
    pub height: u32,
    pub font_family: String,
    pub text_color: sk::Color,
    pub background: QuoteBackground,
    /// Fraction of the canvas the text block may cover (0.0..=1.0).
    pub coverage: f32,
}

impl QuoteSpec {
    pub fn new(text: impl Into<String>, author: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            author: author.into(),
            width: 1080,
            height: 1080,
            font_family: String::new(),
            text_color: sk::Color::from_rgba8(255, 255, 255, 255),
            background: QuoteBackground::Gradient(
                sk::Color::from_rgba8(43, 44, 94, 255),
                sk::Color::from_rgba8(224, 122, 44, 255),
            ),
            coverage: 0.8,
        }
    }
}

/// Build a quote-card Document. Text is measured once (deterministic layout)
/// so the quote + attribution block can be centered as a unit.
pub fn quote_document(engine: &mut TextEngine, spec: &QuoteSpec) -> Document {
    let (w, h) = (spec.width as f32, spec.height as f32);
    let mut doc = Document::new(spec.width, spec.height);

    doc.background = match &spec.background {
        QuoteBackground::Solid(c) => Background::Solid(*c),
        QuoteBackground::Gradient(a, b) => Background::LinearGradient {
            x1: 0.0,
            y1: 0.0,
            x2: w,
            y2: h,
            stops: vec![(0.0, *a), (1.0, *b)],
        },
        QuoteBackground::Image(_) => Background::Transparent,
    };
    if let QuoteBackground::Image(pm) = &spec.background {
        let s = (w / pm.width() as f32).max(h / pm.height() as f32);
        let l = doc.add_image(pm.clone());
        l.transform = Transform {
            x: (w - pm.width() as f32 * s) / 2.0,
            y: (h - pm.height() as f32 * s) / 2.0,
            scale: s,
            ..Default::default()
        };
    }

    let box_w = w * spec.coverage;
    let box_h = h * spec.coverage;

    let quote_style = TextStyle {
        font_family: spec.font_family.clone(),
        font_size: 200.0,
        fill: spec.text_color,
        align: TextAlign::Center,
        max_width: Some(box_w),
        fit_box: Some((box_w, box_h * 0.72)), // leave room for the attribution
        ..Default::default()
    };
    let author_style = TextStyle {
        font_family: spec.font_family.clone(),
        font_size: 100.0,
        fill: sk::Color::from_rgba8(255, 255, 255, 217),
        align: TextAlign::Center,
        max_width: Some(box_w),
        fit_box: Some((box_w, box_h * 0.25)),
        ..Default::default()
    };

    let q = engine.render(&spec.text, &quote_style).expect("quote text renders");
    let author = if spec.author.trim().is_empty() {
        None
    } else {
        engine.render(&format!("\u{2014} {}", spec.author), &author_style)
    };

    let gap = q.font_size * 0.5;
    let total_h = q.pixmap.height() as f32
        + author.as_ref().map_or(0.0, |a| gap + a.pixmap.height() as f32);
    let top = (h - total_h) / 2.0;

    let l = doc.add_text(spec.text.clone(), quote_style);
    l.transform =
        Transform { x: (w - q.pixmap.width() as f32) / 2.0, y: top, ..Default::default() };
    if let Some(a) = author {
        let l = doc.add_text(format!("\u{2014} {}", spec.author), author_style);
        l.transform = Transform {
            x: (w - a.pixmap.width() as f32) / 2.0,
            y: top + q.pixmap.height() as f32 + gap,
            ..Default::default()
        };
    }
    doc
}
