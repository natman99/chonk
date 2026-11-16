//! SVG import via usvg (parse/normalize) + resvg (render to tiny-skia Pixmap).

use tiny_skia as sk;

pub type SvgError = usvg::Error;

pub fn parse(data: &[u8]) -> Result<usvg::Tree, SvgError> {
    let opts = usvg::Options::default();
    usvg::Tree::from_data(data, &opts)
}

/// Render an SVG tree into a fresh pixmap sized to its intrinsic size.
pub fn render(tree: &usvg::Tree) -> sk::Pixmap {
    let size = tree.size();
    let w = size.width().ceil().max(1.0) as u32;
    let h = size.height().ceil().max(1.0) as u32;
    let mut pm = sk::Pixmap::new(w, h).expect("non-zero pixmap");
    let scale = sk::Transform::from_scale(
        w as f32 / size.width(),
        h as f32 / size.height(),
    );
    resvg::render(tree, scale, &mut pm.as_mut());
    pm
}
