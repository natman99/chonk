//! The compositor: document -> Pixmap. All rasterization lands here.

use tiny_skia as sk;

use crate::document::{Background, Document, LayerKind};
use crate::text::TextEngine;

pub struct Renderer {
    pub text: TextEngine,
    /// Composited backgrounds keyed on (size, background): the gradient fill
    /// is ~250ms in debug builds and only changes when the user edits it.
    bg_cache: std::collections::HashMap<String, sk::Pixmap>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self { text: TextEngine::new(), bg_cache: std::collections::HashMap::new() }
    }

    pub fn composite(&mut self, doc: &Document) -> sk::Pixmap {
        self.composite_at(doc, 0)
    }

    /// Composite one animation frame: animated layers use `frame % len`,
    /// static layers repeat.
    pub fn composite_at(&mut self, doc: &Document, frame: usize) -> sk::Pixmap {
        self.composite_full(doc, frame)
    }

    /// Composite at preview resolution: the canvas is `doc * scale` and every
    /// layer blit carries the extra global scale. Used when the canvas is
    /// displayed smaller than the document (zoomed out) — blits and the
    /// texture upload shrink with the preview, while text keeps its full
    /// rasterization (cache reuse) and is downsampled at blit time.
    pub fn composite_scaled(&mut self, doc: &Document, frame: usize, scale: f32) -> sk::Pixmap {
        if scale >= 0.999 {
            return self.composite_at(doc, frame);
        }
        let w = ((doc.width as f32) * scale).max(1.0) as u32;
        let h = ((doc.height as f32) * scale).max(1.0) as u32;
        let key = format!("S{scale:.4} {}x{}{:?}", doc.width, doc.height, doc.background);
        let mut canvas = match self.bg_cache.get(&key) {
            Some(c) => c.clone(),
            None => {
                let full = self.background_pm(doc);
                let mut c = sk::Pixmap::new(w, h).expect("valid canvas size");
                let mut paint = sk::PixmapPaint::default();
                paint.quality = sk::FilterQuality::Bicubic;
                c.draw_pixmap(
                    0,
                    0,
                    full.as_ref(),
                    &paint,
                    sk::Transform::from_scale(scale, scale),
                    None,
                );
                if !matches!(doc.background, Background::Transparent) {
                    self.bg_cache.insert(key, c.clone());
                }
                c
            }
        };
        self.paint_layers(&mut canvas, doc, frame, scale);
        canvas
    }

    /// Number of frames an animated export of `doc` would have.
    pub fn frame_count(doc: &Document) -> usize {
        doc.layers
            .iter()
            .filter(|l| l.visible)
            .map(|l| match &l.kind {
                LayerKind::AnimatedImage { image } => image.frame_count(),
                _ => 1,
            })
            .max()
            .unwrap_or(1)
    }

    /// Composite every frame of an animated document. Delays come from the
    /// animated layer with the most frames.
    pub fn composite_animated(&mut self, doc: &Document) -> (Vec<sk::Pixmap>, Vec<u32>) {
        let n = Self::frame_count(doc);
        let delays = doc
            .layers
            .iter()
            .filter(|l| l.visible)
            .map(|l| match &l.kind {
                LayerKind::AnimatedImage { image } => Some(image.delays_ms.clone()),
                _ => None,
            })
            .max_by(|a, b| match (a, b) {
                (Some(x), Some(y)) => x.len().cmp(&y.len()),
                (Some(_), None) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Less,
            })
            .flatten()
            .unwrap_or_else(|| vec![100; n]);
        let frames = (0..n).map(|i| self.composite_at(doc, i)).collect();
        (frames, delays)
    }

    /// Render only a rect of the document (dirty-rect editing: a drag frame
    /// costs the dragged layer's area, not the whole canvas). The caller
    /// overlays the result on its cached full canvas at `(x, y)`.
    pub fn render_region(&mut self, doc: &Document, x: u32, y: u32, w: u32, h: u32) -> sk::Pixmap {
        self.render_region_at(doc, x, y, w, h, 0)
    }

    pub fn render_region_at(
        &mut self,
        doc: &Document,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        frame: usize,
    ) -> sk::Pixmap {
        let x = x.min(doc.width);
        let y = y.min(doc.height);
        let w = w.min(doc.width - x).max(1);
        let h = h.min(doc.height - y).max(1);

        let mut sub_doc = Document::new(w, h);
        sub_doc.background = match &doc.background {
            Background::Transparent | Background::Solid(_) => doc.background.clone(),
            Background::LinearGradient { x1, y1, x2, y2, stops } => Background::LinearGradient {
                x1: *x1 - x as f32,
                y1: *y1 - y as f32,
                x2: *x2 - x as f32,
                y2: *y2 - y as f32,
                stops: stops.clone(),
            },
        };
        for layer in doc.layers.iter().filter(|l| l.visible) {
            let mut l2 = layer.clone();
            l2.transform.x -= x as f32;
            l2.transform.y -= y as f32;
            sub_doc.layers.push(l2);
        }
        self.composite_full(&sub_doc, frame)
    }

    /// Blit a region produced by `render_region` back onto a cached canvas.
    pub fn overlay_region(canvas: &mut sk::Pixmap, region: &sk::Pixmap, x: u32, y: u32) {
        canvas.draw_pixmap(
            x as i32,
            y as i32,
            region.as_ref(),
            &sk::PixmapPaint::default(),
            sk::Transform::default(),
            None,
        );
    }

    /// Full-resolution background pixmap for the document (cached).
    fn background_pm(&mut self, doc: &Document) -> sk::Pixmap {
        let bg_key = format!("{}x{}{:?}", doc.width, doc.height, doc.background);
        if let Some(cached) = self.bg_cache.get(&bg_key) {
            return cached.clone();
        }
        let mut canvas = sk::Pixmap::new(doc.width, doc.height).expect("valid canvas size");

        match &doc.background {
            Background::Transparent => {}
            Background::Solid(c) => canvas.fill(*c),
            Background::LinearGradient { x1, y1, x2, y2, stops } => {
                let mut paint = sk::Paint::default();
                let stops: Vec<sk::GradientStop> =
                    stops.iter().map(|(p, c)| sk::GradientStop::new(*p, *c)).collect();
                if let Some(shader) = sk::LinearGradient::new(
                    sk::Point { x: *x1, y: *y1 },
                    sk::Point { x: *x2, y: *y2 },
                    stops,
                    sk::SpreadMode::Pad,
                    sk::Transform::default(),
                ) {
                    paint.shader = shader;
                }
                let rect = sk::Rect::from_xywh(0.0, 0.0, doc.width as f32, doc.height as f32)
                    .expect("non-zero canvas");
                let path = sk::PathBuilder::from_rect(rect);
                canvas.fill_path(&path, &paint, sk::FillRule::Winding, sk::Transform::default(), None);
            }
        }

        // Background painted fresh: keep it for future composites of the same
        // (size, background).
        if matches!(doc.background, Background::Transparent) {
            // Nothing to cache; canvas starts cleared anyway.
        } else {
            self.bg_cache.insert(bg_key, canvas.clone());
        }
        canvas
    }

    fn composite_full(&mut self, doc: &Document, frame: usize) -> sk::Pixmap {
        let mut canvas = self.background_pm(doc);
        self.paint_layers(&mut canvas, doc, frame, 1.0);
        canvas
    }

    fn paint_layers(&mut self, canvas: &mut sk::Pixmap, doc: &Document, frame: usize, scale: f32) {
        for layer in doc.layers.iter().filter(|l| l.visible) {
            match &layer.kind {
                LayerKind::Image { pixmap } => {
                    blit(canvas, pixmap, &layer.transform, layer.opacity, layer.blend, scale);
                }
                LayerKind::AnimatedImage { image } => {
                    blit(canvas, image.frame(frame), &layer.transform, layer.opacity, layer.blend, scale);
                }
                LayerKind::Svg { tree } => {
                    let pm = crate::svg::render(tree);
                    blit(canvas, &pm, &layer.transform, layer.opacity, layer.blend, scale);
                }
                LayerKind::Text { text, style } => {
                    if let Some(rt) = self.text.render(text, style) {
                        blit(canvas, &rt.pixmap, &layer.transform, layer.opacity, layer.blend, scale);
                    }
                }
            }
        }
    }
}

fn blit(canvas: &mut sk::Pixmap, pm: &sk::Pixmap, t: &crate::Transform, opacity: f32, blend: crate::BlendMode, scale: f32) {
    let mut paint = sk::PixmapPaint::default();
    paint.opacity = opacity.clamp(0.0, 1.0);
    paint.blend_mode = blend.to_skia();
    // tiny-skia's fast path: an untransformed blit is a straight blend.
    // Bicubic only pays off on scaled/rotated layers; translation alone
    // (drags) stays Nearest — Bilinear in a debug build costs 10x for
    // sub-pixel smoothness nobody sees at meme scale. Preview compositing
    // (scale != 1) always downsamples, so Bilinear.
    paint.quality = if t.scale == 1.0 && t.rotation_deg == 0.0 && scale == 1.0 {
        sk::FilterQuality::Nearest
    } else {
        sk::FilterQuality::Bilinear
    };
    let tr = if scale == 1.0 {
        t.to_skia()
    } else {
        t.to_skia().post_scale(scale, scale)
    };
    canvas.draw_pixmap(0, 0, pm.as_ref(), &paint, tr, None);
}
