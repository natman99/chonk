//! Template slots: fill a template document by slot name.
//!
//! A template is just a `.chonk` document whose layers carry `slot`
//! metadata. Filling produces a regular document — render/export it
//! like any other. Batch mode = fill the same template with different
//! values (or a folder of images) and export each.

use crate::document::{Document, LayerKind, SlotKind};
use crate::tiny_skia as sk;

/// Replace the text of the `Text` slot named `name`. Returns false if no
/// such slot exists (or its kind mismatches).
pub fn fill_text(doc: &mut Document, name: &str, text: &str) -> bool {
    for l in &mut doc.layers {
        if let Some(s) = &l.slot {
            if s.name == name && s.kind == SlotKind::Text {
                if let LayerKind::Text { text: t, .. } = &mut l.kind {
                    *t = text.to_string();
                    return true;
                }
            }
        }
    }
    false
}

/// Replace the image of the `Image` slot named `name`, scaling the new
/// image to the slot's current display size so the layout is preserved.
pub fn fill_image(doc: &mut Document, name: &str, image: &sk::Pixmap) -> bool {
    for l in &mut doc.layers {
        if let Some(s) = &l.slot {
            if s.name == name && s.kind == SlotKind::Image {
                if let LayerKind::Image { pixmap } = &mut l.kind {
                    let (w, h) = (pixmap.width(), pixmap.height());
                    let scaled = if (image.width(), image.height()) == (w, h) {
                        image.clone()
                    } else {
                        let mut out = match sk::Pixmap::new(w, h) {
                            Some(p) => p,
                            None => return false,
                        };
                        let mut paint = sk::PixmapPaint::default();
                        paint.quality = sk::FilterQuality::Bilinear;
                        let sc = sk::Transform::from_scale(
                            w as f32 / image.width() as f32,
                            h as f32 / image.height() as f32,
                        );
                        out.draw_pixmap(0, 0, image.as_ref(), &paint, sc, None);
                        out
                    };
                    *pixmap = scaled;
                    return true;
                }
            }
        }
    }
    false
}

/// All slot names in document order (for CLI listing / validation).
pub fn slot_names(doc: &Document) -> Vec<(String, SlotKind)> {
    doc.layers
        .iter()
        .filter_map(|l| l.slot.as_ref().map(|s| (s.name.clone(), s.kind)))
        .collect()
}
