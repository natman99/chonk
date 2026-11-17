//! Text rendering: cosmic-text for shaping/layout, swash for glyph outlines,
//! tiny-skia for fill/stroke/shadow rasterization. Vector-crisp at any size.

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, Wrap};
use std::collections::HashMap;
use tiny_skia as sk;

use crate::document::{Shadow, TextAlign, TextStyle};

/// Font files shipped with the app (SIL OFL 1.1 — see fonts/LICENSES.md).
pub const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../fonts/Anton-Regular.ttf"),
    include_bytes!("../fonts/ArchivoBlack-Regular.ttf"),
    include_bytes!("../fonts/NotoSans-Regular.ttf"),
    include_bytes!("../fonts/NotoSans-Bold.ttf"),
    include_bytes!("../fonts/NotoSerif-Regular.ttf"),
    include_bytes!("../fonts/NotoSerif-Bold.ttf"),
    include_bytes!("../fonts/SpaceMono-Regular.ttf"),
    include_bytes!("../fonts/SpaceMono-Bold.ttf"),
];

pub struct TextEngine {
    font_system: FontSystem,
    scale: swash::scale::ScaleContext,
    /// Rendered text cache keyed on (text, style): composites become pure
    /// blits for unchanged layers (drags, toggles, animation frames).
    /// Editing churns one entry per keystroke; at large font sizes each
    /// entry is megabytes, so it's FIFO-evicted on a 64 MB byte budget.
    cache: HashMap<String, std::sync::Arc<RenderedText>>,
    cache_order: std::collections::VecDeque<String>,
    cache_bytes: usize,
}

pub struct RenderedText {
    pub pixmap: sk::Pixmap,
    /// Font size actually used (may be smaller than requested with `fit_box`).
    pub font_size: f32,
    /// Where the text block's (0,0) sits inside the pixmap (stroke/shadow
    /// padding): pixmap point = block point - block_offset.
    pub block_offset: (f32, f32),
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        Self {
            font_system: Self::bundled_font_system(),
            scale: swash::scale::ScaleContext::new(),
            cache: HashMap::new(),
            cache_order: std::collections::VecDeque::new(),
            cache_bytes: 0,
        }
    }

    /// Bundled fonts (all SIL OFL 1.1, in `fonts/`) load first so they are
    /// always available regardless of the host system; system fonts follow.
    fn bundled_font_system() -> FontSystem {
        let mut db = cosmic_text::fontdb::Database::new();
        for bytes in BUNDLED_FONTS {
            db.load_font_data(bytes.to_vec());
        }
        db.load_system_fonts();
        FontSystem::new_with_locale_and_db(String::new(), db)
    }

    /// Family names of the bundled set (for a font picker).
    pub fn bundled_families() -> &'static [&'static str] {
        &["Anton", "Archivo Black", "Noto Sans", "Noto Serif", "Space Mono"]
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
        let bytes = rt.pixmap.width() as usize * rt.pixmap.height() as usize * 4;
        self.cache.insert(key.clone(), rt.clone());
        self.cache_order.push_back(key);
        self.cache_bytes += bytes;
        const CACHE_BUDGET: usize = 64 << 20;
        while self.cache_bytes > CACHE_BUDGET {
            let Some(old_key) = self.cache_order.pop_front() else { break };
            if let Some(old) = self.cache.remove(&old_key) {
                self.cache_bytes -= old.pixmap.width() as usize * old.pixmap.height() as usize * 4;
            }
        }
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

        Some(RenderedText { pixmap: pm, font_size, block_offset: (left, top) })
    }

    /// Build a shaped cosmic-text Buffer for caret/hit-test geometry.
    /// `font_size` must match the size the text was rendered at (use
    /// RenderedText::font_size — `fit_box` may have shrunk it).
    fn buffer_for(&mut self, text: &str, style: &TextStyle, font_size: f32) -> Buffer {
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
        buffer
    }

    /// Layout-only text metrics (no glyph rasterization): pixmap-sized
    /// (w, h) including stroke/shadow padding, computed from cosmic-text
    /// layout alone. Used for invalidation rects, where conservative
    /// bounds are fine and rasterizing on the caller's thread is not.
    pub fn layout_size(&mut self, text: &str, style: &TextStyle) -> Option<(f32, f32)> {
        if text.is_empty() {
            return None;
        }
        let size = style.font_size.max(1.0);
        let buffer = self.buffer_for(text, style, size);
        let mut w = 0.0f32;
        let mut top = f32::MAX;
        let mut bottom = 0.0f32;
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            top = top.min(run.line_top);
            bottom = bottom.max(run.line_top + run.line_height);
        }
        if w <= 0.0 || bottom <= top {
            return None;
        }
        // Same padding render_uncached applies (stroke + shadow + blur);
        // size-proportional slack keeps shadowed text inside the rect.
        let stroke_w = style.stroke.map(|(_, sw)| sw).unwrap_or(0.0);
        let pad = stroke_w + 2.0 + size * 0.25;
        Some((w + 2.0 * pad, (bottom - top) + 2.0 * pad))
    }

    /// Caret geometry for a byte index: (x, y_top, height) in the rendered
    /// pixmap's coordinate space — the space layers draw into.
    pub fn caret_geometry(
        &mut self,
        text: &str,
        style: &TextStyle,
        byte: usize,
    ) -> Option<(f32, f32, f32)> {
        let rt = self.render(text, style)?;
        let byte = byte.min(text.len());
        let buffer = self.buffer_for(text, style, rt.font_size);
        let cursor = cosmic_text::Cursor::new(0, byte);
        let mut last_run_end = None;
        for run in buffer.layout_runs() {
            if let Some(x) = run.cursor_position(&cursor) {
                return Some((
                    x - rt.block_offset.0,
                    run.line_top - rt.block_offset.1,
                    run.line_height,
                ));
            }
            let x_end = run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0);
            last_run_end = Some((x_end, run.line_top, run.line_height));
        }
        // Cursor past the final glyph (the common "editing at end" case).
        last_run_end.map(|(x, t, h)| (x - rt.block_offset.0, t - rt.block_offset.1, h))
    }

    /// Byte index of the glyph nearest a point in pixmap coordinates.
    pub fn hit_byte(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> Option<usize> {
        let rt = self.render(text, style)?;
        let mut buffer = self.buffer_for(text, style, rt.font_size);
        buffer
            .hit(x + rt.block_offset.0, y + rt.block_offset.1)
            .map(|c| c.index)
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
