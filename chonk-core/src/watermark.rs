//! Fake social-media watermarks — the other half of the deep-fry
//! aesthetic. Adds a caption layer styled like a platform overlay.

use crate::document::{Document, TextAlign, TextStyle};
use crate::tiny_skia as sk;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WatermarkStyle {
    TikTok,
    Instagram,
    YouTube,
    Snapchat,
    Reddit,
}

impl WatermarkStyle {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "tiktok" | "tt" => Some(Self::TikTok),
            "instagram" | "ig" => Some(Self::Instagram),
            "youtube" | "yt" => Some(Self::YouTube),
            "snapchat" | "snap" => Some(Self::Snapchat),
            "reddit" => Some(Self::Reddit),
            _ => None,
        }
    }
}

/// Add a watermark caption layer to `doc` (bottom area, platform-styled).
pub fn add_watermark(doc: &mut Document, style: WatermarkStyle, text: &str) {
    let white = sk::Color::from_rgba8(255, 255, 255, 217);
    let (fill, size, x_frac, y_frac, align, shadow) = match style {
        // TikTok: bold white, bottom-left, hard dark shadow.
        WatermarkStyle::TikTok => (white, 28.0, 0.04, 0.88, TextAlign::Left, true),
        // Instagram: thin-ish white, top-left.
        WatermarkStyle::Instagram => (white, 24.0, 0.04, 0.03, TextAlign::Left, true),
        // YouTube: white on the bottom-right, no stroke.
        WatermarkStyle::YouTube => (white, 26.0, 0.96, 0.9, TextAlign::Right, true),
        // Snapchat: big white centered near the bottom.
        WatermarkStyle::Snapchat => (white, 34.0, 0.5, 0.85, TextAlign::Center, true),
        // Reddit: orange, bottom-left.
        WatermarkStyle::Reddit => (
            sk::Color::from_rgba8(255, 69, 0, 217),
            24.0,
            0.04,
            0.9,
            TextAlign::Left,
            true,
        ),
    };
    let mut st = TextStyle {
        font_family: "Noto Sans".into(),
        font_size: size,
        fill,
        stroke: None,
        shadow: if shadow {
            Some(crate::document::Shadow {
                dx: 1.5,
                dy: 1.5,
                blur: 0.0,
                color: sk::Color::from_rgba8(0, 0, 0, 180),
            })
        } else {
            None
        },
        align,
        max_width: Some(doc.width as f32 * 0.92),
        fit_box: None,
    };
    let (dw, dh) = (doc.width as f32, doc.height as f32);
    let l = doc.add_text(text, st);
    l.transform.x = dw * x_frac;
    l.transform.y = dh * y_frac;
}
