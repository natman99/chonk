use chonk_core::codecs::{export_png, load_image};
use chonk_core::document::Document;
use chonk_core::render::Renderer;
fn main() {
    let path = &std::env::args().nth(1).unwrap();
    let mut doc = Document::new(1080, 1080);
    doc.add_image(load_image(&std::fs::read(path).unwrap()).unwrap());
    let pm = Renderer::new().composite(&doc);
    std::fs::write("/tmp/composited.png", export_png(&pm).unwrap()).unwrap();
    let p = pm.pixels()[(400 * 1080 + 300) as usize]; // hair area
    println!("composited hair pixel: ({}, {}, {})", p.red(), p.green(), p.blue());
}
