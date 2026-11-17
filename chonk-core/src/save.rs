//! The .chonk document format: pretty JSON with base64 PNG payloads for
//! raster data and raw SVG source for vector layers. Pretty-printed so
//! image-free documents (templates!) are human-readable and git-diffable.

use crate::document::{
    AnimatedImage, Background, Document, Layer, LayerKind, Shadow, TextAlign, TextStyle, Transform,
};
use crate::tiny_skia as sk;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

// ---------------------------------------------------------------- base64
// Minimal standard-alphabet base64 (no deps; payloads are PNG bytes only).

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut vals = [u8::MAX; 128];
    for (i, c) in B64.iter().enumerate() {
        vals[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = *vals.get(c as usize).ok_or("bad base64 char")?;
        if v == u8::MAX {
            return Err("bad base64 char".into());
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- colors

type Rgba = [u8; 4];

fn color_pack(c: sk::Color) -> Rgba {
    let c8 = c.to_color_u8();
    [c8.red(), c8.green(), c8.blue(), c8.alpha()]
}

fn color_unpack(c: Rgba) -> sk::Color {
    sk::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn pm_png(pm: &sk::Pixmap) -> Result<String, String> {
    let png = pm.encode_png().map_err(|e| format!("png encode: {e}"))?;
    Ok(b64_encode(&png))
}

fn pm_from_png(s: &str) -> Result<sk::Pixmap, String> {
    let bytes = b64_decode(s)?;
    sk::Pixmap::decode_png(&bytes).map_err(|e| format!("png decode: {e}"))
}

// ---------------------------------------------------------------- model

#[derive(Serialize, Deserialize)]
struct SavedDoc {
    version: u32,
    width: u32,
    height: u32,
    background: SavedBg,
    layers: Vec<SavedLayer>,
}

#[derive(Serialize, Deserialize)]
enum SavedBg {
    Transparent,
    Solid(Rgba),
    LinearGradient { x1: f32, y1: f32, x2: f32, y2: f32, stops: Vec<(f32, Rgba)> },
}

#[derive(Serialize, Deserialize)]
struct SavedLayer {
    kind: SavedKind,
    x: f32,
    y: f32,
    scale: f32,
    rotation_deg: f32,
    opacity: f32,
    visible: bool,
}

#[derive(Serialize, Deserialize)]
enum SavedKind {
    Image { png: String },
    Animated { pngs: Vec<String>, delays_ms: Vec<u32> },
    Svg { source: String },
    Text { text: String, style: SavedStyle },
}

#[derive(Serialize, Deserialize)]
struct SavedStyle {
    font_family: String,
    font_size: f32,
    fill: Rgba,
    stroke: Option<(Rgba, f32)>,
    shadow: Option<SavedShadow>,
    align: u8,
    max_width: Option<f32>,
    fit_box: Option<(f32, f32)>,
}

#[derive(Serialize, Deserialize)]
struct SavedShadow {
    dx: f32,
    dy: f32,
    blur: f32,
    color: Rgba,
}

// ---------------------------------------------------------------- to/from

fn save_style(s: &TextStyle) -> SavedStyle {
    SavedStyle {
        font_family: s.font_family.clone(),
        font_size: s.font_size,
        fill: color_pack(s.fill),
        stroke: s.stroke.map(|(c, w)| (color_pack(c), w)),
        shadow: s.shadow.map(|sh| SavedShadow {
            dx: sh.dx,
            dy: sh.dy,
            blur: sh.blur,
            color: color_pack(sh.color),
        }),
        align: match s.align {
            TextAlign::Left => 0,
            TextAlign::Center => 1,
            TextAlign::Right => 2,
        },
        max_width: s.max_width,
        fit_box: s.fit_box,
    }
}

fn load_style(s: SavedStyle) -> TextStyle {
    TextStyle {
        font_family: s.font_family,
        font_size: s.font_size,
        fill: color_unpack(s.fill),
        stroke: s.stroke.map(|(c, w)| (color_unpack(c), w)),
        shadow: s.shadow.map(|sh| Shadow {
            dx: sh.dx,
            dy: sh.dy,
            blur: sh.blur,
            color: color_unpack(sh.color),
        }),
        align: match s.align {
            0 => TextAlign::Left,
            2 => TextAlign::Right,
            _ => TextAlign::Center,
        },
        max_width: s.max_width,
        fit_box: s.fit_box,
    }
}

fn save_layer(l: &Layer) -> Result<SavedLayer, String> {
    let kind = match &l.kind {
        LayerKind::Image { pixmap } => SavedKind::Image { png: pm_png(pixmap)? },
        LayerKind::AnimatedImage { image } => SavedKind::Animated {
            pngs: image.frames.iter().map(pm_png).collect::<Result<_, _>>()?,
            delays_ms: image.delays_ms.clone(),
        },
        LayerKind::Svg { tree } => {
            let opts = usvg::WriteOptions::default();
            SavedKind::Svg { source: tree.to_string(&opts) }
        }
        LayerKind::Text { text, style } => SavedKind::Text { text: text.clone(), style: save_style(style) },
    };
    Ok(SavedLayer {
        kind,
        x: l.transform.x,
        y: l.transform.y,
        scale: l.transform.scale,
        rotation_deg: l.transform.rotation_deg,
        opacity: l.opacity,
        visible: l.visible,
    })
}

fn load_layer(l: SavedLayer) -> Result<Layer, String> {
    let kind = match l.kind {
        SavedKind::Image { png } => LayerKind::Image { pixmap: pm_from_png(&png)? },
        SavedKind::Animated { pngs, delays_ms } => LayerKind::AnimatedImage {
            image: AnimatedImage {
                frames: pngs.iter().map(|s| pm_from_png(s)).collect::<Result<_, _>>()?,
                delays_ms,
            },
        },
        SavedKind::Svg { source } => {
            let tree = crate::svg::parse(source.as_bytes()).map_err(|e| e.to_string())?;
            LayerKind::Svg { tree }
        }
        SavedKind::Text { text, style } => LayerKind::Text { text, style: load_style(style) },
    };
    Ok(Layer {
        kind,
        transform: Transform { x: l.x, y: l.y, scale: l.scale, rotation_deg: l.rotation_deg },
        opacity: l.opacity,
        visible: l.visible,
    })
}

/// Serialize a document to .chonk JSON text.
pub fn to_json(doc: &Document) -> Result<String, String> {
    let saved = SavedDoc {
        version: VERSION,
        width: doc.width,
        height: doc.height,
        background: match &doc.background {
            Background::Transparent => SavedBg::Transparent,
            Background::Solid(c) => SavedBg::Solid(color_pack(*c)),
            Background::LinearGradient { x1, y1, x2, y2, stops } => SavedBg::LinearGradient {
                x1: *x1,
                y1: *y1,
                x2: *x2,
                y2: *y2,
                stops: stops.iter().map(|(p, c)| (*p, color_pack(*c))).collect(),
            },
        },
        layers: doc.layers.iter().map(save_layer).collect::<Result<_, _>>()?,
    };
    serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())
}

/// Parse .chonk JSON text back into a document.
pub fn from_json(text: &str) -> Result<Document, String> {
    let saved: SavedDoc = serde_json::from_str(text).map_err(|e| format!("bad .chonk: {e}"))?;
    if saved.version != VERSION {
        return Err(format!("unsupported .chonk version {}", saved.version));
    }
    Ok(Document {
        width: saved.width,
        height: saved.height,
        background: match saved.background {
            SavedBg::Transparent => Background::Transparent,
            SavedBg::Solid(c) => Background::Solid(color_unpack(c)),
            SavedBg::LinearGradient { x1, y1, x2, y2, stops } => Background::LinearGradient {
                x1,
                y1,
                x2,
                y2,
                stops: stops.into_iter().map(|(p, c)| (p, color_unpack(c))).collect(),
            },
        },
        layers: saved.layers.into_iter().map(load_layer).collect::<Result<_, _>>()?,
    })
}

pub fn save_chonk(doc: &Document, path: &std::path::Path) -> Result<(), String> {
    let json = to_json(doc)?;
    std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn load_chonk(path: &std::path::Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    from_json(&text)
}
