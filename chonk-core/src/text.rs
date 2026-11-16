//! Text rendering: cosmic-text for shaping/layout, swash for glyph outlines,
//! tiny-skia for fill/stroke/shadow rasterization. Vector-crisp at any size.

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, Wrap};
use std::collections::HashMap;
use tiny_skia as sk;

use crate::document::{Shadow, TextAlign, TextStyle};

pub struct TextEngine {
    font_system: FontSystem,
    scale: swash::scale::ScaleContext,
    /// Rendered text cache keyed on (text, style): composites become pure
    /// blits for unchanged layers (drags, toggles, animation frames).
    cache: HashMap<String, std::sync::Arc<RenderedText>>,
}

pub struct RenderedText {
    pub pixmap: sk::Pixmap,
    /// Font size actually used (may be smaller than requested with `fit_box`).
    pub font_size: f32,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            scale: swash::scale::ScaleContext::new(),
            cache: HashMap::new(),
        }
    }

    pub fn font_system(&self) -> &FontSystem {
        &self.font_system
    }

    /// Shape and lay out `text` at `font_size`, collecting every glyph outline
    /// into one path in text-block coordinates.
    fn layout(
        &mut self,
        text: &str,
        style: &TextStyle,
        font_size: f32,
    ) -> Option<(sk::Path, f32, f32)> {
        let attrs = Attrs::new();
        let attrs = match style.font_family.as_str() {
            "" | "sans-serif" => attrs.family(cosmic_text::Family::SansSerif),
            "serif" => attrs.family(cosmic_text::Family::Serif),
            "monospace" => attrs.family(cosmic_text::Family::Monospace),
            name => attrs.family(cosmic_text::Family::Name(name)),
        };
        let align = match style.align {
            TextAlign::Left => cosmic_text::Align::Left,
            TextAlign::Center => cosmic_text::Align::Center,
            TextAlign::Right => cosmic_text::Align::Right,
        };
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::relative(font_size, 1.2));
        if let Some(w) = style.max_width {
            buffer.set_size(Some(w), None);
            buffer.set_wrap(Wrap::WordOrGlyph);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut pb = sk::PathBuilder::new();
        let (mut max_w, mut max_h) = (0.0f32, 0.0f32);
        for run in buffer.layout_runs() {
            max_w = max_w.max(run.line_w);
            max_h = max_h.max(run.line_top + run.line_height);
            for g in run.glyphs {
                let Some(font) = self.font_system.get_font(g.font_id, g.font_weight) else {
                    continue;
                };
                let mut scaler =
                    self.scale.builder(font.as_swash()).size(g.font_size).hint(false).build();
                let Some(outline) = scaler.scale_outline(g.glyph_id) else { continue };
                let (ox, oy) = (g.x, run.line_y + g.y);
                // swash outlines are y-up (font convention); tiny-skia is y-down.
                let (pts, verbs) = (outline.points(), outline.verbs());
                let mut pi = 0usize;
                for v in verbs {
                    use swash::zeno::Verb;
                    match v {
                        Verb::MoveTo => {
                            let p = &pts[pi];
                            pi += 1;
                            pb.move_to(p.x + ox, oy - p.y);
                        }
                        Verb::LineTo => {
                            let p = &pts[pi];
                            pi += 1;
                            pb.line_to(p.x + ox, oy - p.y);
                        }
                        Verb::QuadTo => {
                            // zeno encoding: (control, endpoint)
                            let (c, p) = (&pts[pi], &pts[pi + 1]);
                            pi += 2;
                            pb.quad_to(c.x + ox, oy - c.y, p.x + ox, oy - p.y);
                        }
                        Verb::CurveTo => {
                            // zeno encoding: (c1, c2, endpoint)
                            let (c1, c2, p) = (&pts[pi], &pts[pi + 1], &pts[pi + 2]);
                            pi += 3;
                            pb.cubic_to(
                                c1.x + ox,
                                oy - c1.y,
                                c2.x + ox,
                                oy - c2.y,
                                p.x + ox,
                                oy - p.y,
                            );
                        }
                        Verb::Close => pb.close(),
                    }
                }
            }
        }
        Some((pb.finish()?, max_w, max_h))
    }

    /// Render styled text into a tight pixmap (with room for stroke + shadow).
    /// Cached on (text, style): unchanged layers never re-rasterize.
    pub fn render(&mut self, text: &str, style: &TextStyle) -> Option<std::sync::Arc<RenderedText>> {
        let key = cache_key(text, style);
        if let Some(hit) = self.cache.get(&key) {
            return Some(hit.clone());
        }
        let rt = std::sync::Arc::new(self.render_uncached(text, style)?);
        self.cache.insert(key, rt.clone());
        Some(rt)
    }

    fn render_uncached(&mut self, text: &str, style: &TextStyle) -> Option<RenderedText> {
        let mut font_size = style.font_size;
        let (mut path, _w, _h) = self.layout(text, style, font_size)?;

        if let Some((bw, bh)) = style.fit_box {
            let b = path.bounds();
            if b.width() > bw || b.height() > bh {
                // Binary-search the largest font size whose wrapped block fits.
                let (mut lo, mut hi) = (4.0f32, font_size);
                let mut best: Option<(sk::Path, f32)> = None;
                while hi - lo > 0.5 {
                    let mid = (lo + hi) * 0.5;
                    match self.layout(text, style, mid) {
                        Some((p, _, _)) => {
                            let pb = p.bounds();
                            if pb.width() <= bw && pb.height() <= bh {
                                best = Some((p, mid));
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        None => hi = mid,
                    }
                }
                if let Some((p, s)) = best {
                    path = p;
                    font_size = s;
                }
            }
        }

        let b = path.bounds();
        let stroke_w = style.stroke.map(|(_, w)| w).unwrap_or(0.0);
        let shadow = style.shadow.unwrap_or(Shadow {
            dx: 0.0,
            dy: 0.0,
            blur: 0.0,
            color: sk::Color::TRANSPARENT,
        });
        let has_shadow = style.shadow.is_some();
        let blur_pad = if has_shadow { shadow.blur * 2.0 } else { 0.0 };
        let pad = stroke_w + 2.0;

        let left = (b.x() - pad + shadow.dx.min(0.0) - blur_pad).min(0.0);
        let top = (b.y() - pad + shadow.dy.min(0.0) - blur_pad).min(0.0);
        let right = b.x() + b.width() + pad + shadow.dx.max(0.0) + blur_pad;
        let bottom = b.y() + b.height() + pad + shadow.dy.max(0.0) + blur_pad;

        let align_pad = 0.0; // alignment is handled by cosmic-text layout

        let pw = right.ceil().max(1.0) as u32;
        let ph = bottom.ceil().max(1.0) as u32;
        let mut pm = sk::Pixmap::new(pw, ph)?;
        let tx = sk::Transform::from_translate(-left + align_pad, -top);

        if has_shadow {
            let mut spm = sk::Pixmap::new(pw, ph)?;
            let stx = tx.pre_translate(shadow.dx, shadow.dy);
            let mut paint = sk::Paint::default();
            paint.set_color(shadow.color);
            spm.fill_path(&path, &paint, sk::FillRule::Winding, stx, None);
            if shadow.blur > 0.0 {
                spm = crate::filters::blur_pixmap(&spm, shadow.blur);
            }
            pm.draw_pixmap(
                0,
                0,
                spm.as_ref(),
                &sk::PixmapPaint::default(),
                sk::Transform::from_translate(align_pad, 0.0),
                None,
            );
        }

        if let Some((c, sw)) = style.stroke {
            let mut paint = sk::Paint::default();
            paint.set_color(c);
            pm.stroke_path(
                &path,
                &paint,
                &sk::Stroke {
                    width: sw,
                    miter_limit: 4.0,
                    line_cap: sk::LineCap::Butt,
                    line_join: sk::LineJoin::Round,
                    dash: None,
                },
                tx,
                None,
            );
        }

        let mut paint = sk::Paint::default();
        paint.set_color(style.fill);
        pm.fill_path(&path, &paint, sk::FillRule::Winding, tx, None);

        Some(RenderedText { pixmap: pm, font_size })
    }
}

/// Exact-match cache key for (text, style). Debug-formatting the style is
/// cheap next to rasterization and covers every visual field.
fn cache_key(text: &str, style: &TextStyle) -> String {
    format!(
        "{text}\u{1f}{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        style.font_family,
        style.font_size,
        style.fill,
        style.stroke,
        style.shadow,
        style.align,
        style.max_width,
        style.fit_box,
    )
}
