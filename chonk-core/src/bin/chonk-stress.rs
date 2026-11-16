//! Stress benchmark: what does tiny-skia cost at "larger workload" scale?
//! Answers: can we stay real-time at 4K with many layers, and what does a
//! drag frame (dirty-region path) actually cost?

use chonk_core::document::{Background, Document};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;
use std::time::Instant;

fn solid(w: u32, h: u32, c: sk::Color) -> sk::Pixmap {
    let mut pm = sk::Pixmap::new(w, h).unwrap();
    pm.fill(c);
    pm
}

fn bench(name: &str, iters: u32, mut f: impl FnMut()) {
    f(); // warm caches
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("{name}: {ms:.2} ms/frame");
}

fn main() {
    // ---- 1080p, 10 image layers (typical meme session with several imports)
    let mut doc = Document::new(1080, 1080);
    doc.background = Background::Solid(sk::Color::from_rgba8(20, 20, 30, 255));
    for i in 0..10 {
        let pm = solid(600, 600, sk::Color::from_rgba8(200 - i * 15, 80, 120 + i * 10, 200));
        doc.add_image(pm);
        doc.layers.last_mut().unwrap().transform.x = (i as f32) * 40.0;
        doc.layers.last_mut().unwrap().transform.y = (i as f32) * 40.0;
    }
    let mut r = Renderer::new();
    bench("1080p, 10x600px alpha layers, FULL composite", 20, || {
        std::hint::black_box(r.composite(&doc));
    });

    // ---- 4K, 10 image layers
    let mut doc4k = Document::new(3840, 2160);
    doc4k.background = Background::Solid(sk::Color::from_rgba8(20, 20, 30, 255));
    for i in 0..10 {
        let pm = solid(1600, 1200, sk::Color::from_rgba8(200 - i * 15, 80, 120 + i * 10, 200));
        doc4k.add_image(pm);
        doc4k.layers.last_mut().unwrap().transform.x = (i as f32) * 150.0;
        doc4k.layers.last_mut().unwrap().transform.y = (i as f32) * 100.0;
    }
    bench("4K, 10x1600px alpha layers, FULL composite", 10, || {
        std::hint::black_box(r.composite(&doc4k));
    });

    // ---- Drag frame, dirty-region path: 4K canvas, one 1600x1200 layer moves.
    // (App path: re-render only the layer's bounding rect, overlay on cached frame.)
    bench("4K drag frame (region 1700x1300, 10 layers)", 20, || {
        std::hint::black_box(r.render_region(&doc4k, 100, 100, 1700, 1300));
    });

    // ---- Worst case: 20 full-canvas 1080p alpha layers.
    let mut doc20 = Document::new(1080, 1080);
    doc20.background = Background::Solid(sk::Color::from_rgba8(20, 20, 30, 255));
    for i in 0..20 {
        let pm = solid(1080, 1080, sk::Color::from_rgba8(10 + i * 10, 100, 200 - i * 8, 60));
        doc20.add_image(pm);
    }
    bench("1080p, 20 FULL-canvas alpha layers", 10, || {
        std::hint::black_box(r.composite(&doc20));
    });

    // ---- Animated GIF playback frame: 1080p canvas, 300x300 animated layer.
    let mut docgif = Document::new(1080, 1080);
    docgif.background = Background::Solid(sk::Color::from_rgba8(20, 20, 30, 255));
    let frames: Vec<sk::Pixmap> = (0..12).map(|i| solid(300, 300, sk::Color::from_rgba8(i * 20, 100, 150, 255))).collect();
    docgif.add_animated(chonk_core::document::AnimatedImage { frames, delays_ms: vec![50; 12] });
    let mut fr = 0usize;
    bench("1080p playback frame (300px anim layer, solid bg)", 30, || {
        fr += 1;
        std::hint::black_box(r.composite_at(&docgif, fr));
    });

}
