use chonk_core::document::TextStyle;
use chonk_core::text::TextEngine;
use chonk_core::codecs;
use chonk_core::tiny_skia as sk;

fn main() {
    let mut e = TextEngine::new();
    let style = TextStyle {
        font_family: "Noto Sans".into(),
        font_size: 200.0,
        fill: sk::Color::from_rgba8(0, 0, 0, 255),
        ..TextStyle::default()
    };
    let rt = e.render("Nw", &style).unwrap();
    let mut canvas = sk::Pixmap::new(400, 300).unwrap();
    canvas.fill(sk::Color::from_rgba8(255, 255, 255, 255));
    canvas
        .draw_pixmap(
            50,
            50,
            rt.pixmap.as_ref(),
            &sk::PixmapPaint::default(),
            sk::Transform::default(),
            None,
        );
    std::fs::write("/tmp/nw.png", codecs::export_png(&canvas).unwrap()).unwrap();
    println!("wrote /tmp/nw.png {}x{}", rt.pixmap.width(), rt.pixmap.height());
}
