//! The compositor: document -> Pixmap. All rasterization lands here.

use tiny_skia as sk;

use crate::document::{Background, Document, LayerKind};
use crate::text::TextEngine;

pub struct Renderer {
    pub text: TextEngine,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self { text: TextEngine::new() }
    }

    pub fn composite(&mut self, doc: &Document) -> sk::Pixmap {
        self.composite_full(doc)
    }

    /// Render only a rect of the document (dirty-rect editing: a drag frame
    /// costs the dragged layer's area, not the whole canvas). The caller
    /// overlays the result on its cached full canvas at `(x, y)`.
    pub fn render_region(&mut self, doc: &Document, x: u32, y: u32, w: u32, h: u32) -> sk::Pixmap {
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
        self.composite_full(&sub_doc)
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

    fn composite_full(&mut self, doc: &Document) -> sk::Pixmap {
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

        for layer in doc.layers.iter().filter(|l| l.visible) {
            match &layer.kind {
                LayerKind::Image { pixmap } => {
                    blit(&mut canvas, pixmap, &layer.transform, layer.opacity);
                }
                LayerKind::Svg { tree } => {
                    let pm = crate::svg::render(tree);
                    blit(&mut canvas, &pm, &layer.transform, layer.opacity);
                }
                LayerKind::Text { text, style } => {
                    if let Some(rt) = self.text.render(text, style) {
                        blit(&mut canvas, &rt.pixmap, &layer.transform, layer.opacity);
                    }
                }
            }
        }

        canvas
    }
}

fn blit(canvas: &mut sk::Pixmap, pm: &sk::Pixmap, t: &crate::Transform, opacity: f32) {
    let mut paint = sk::PixmapPaint::default();
    paint.opacity = opacity.clamp(0.0, 1.0);
    // tiny-skia's fast path: an untransformed blit is a straight blend.
    // Bicubic only pays off on scaled/rotated layers; drags use Bilinear.
    paint.quality = if t.scale == 1.0 && t.rotation_deg == 0.0 && t.x == 0.0 && t.y == 0.0 {
        sk::FilterQuality::Nearest
    } else {
        sk::FilterQuality::Bilinear
    };
    canvas.draw_pixmap(0, 0, pm.as_ref(), &paint, t.to_skia(), None);
}
