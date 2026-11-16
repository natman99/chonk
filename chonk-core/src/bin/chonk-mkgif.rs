//! chonk-mkgif: generate a bouncing-ball test GIF (demo asset / e2e input).

use chonk_core::codecs::export_gif;
use chonk_core::tiny_skia as sk;

fn main() {
    let (w, h) = (320u32, 240u32);
    let n = 12;
    let mut frames = Vec::new();
    for i in 0..n {
        let mut pm = sk::Pixmap::new(w, h).unwrap();
        // sky -> ground gradient
        let stops = vec![
            sk::GradientStop::new(0.0, sk::Color::from_rgba8(70, 130, 180, 255)),
            sk::GradientStop::new(1.0, sk::Color::from_rgba8(30, 60, 90, 255)),
        ];
        let mut paint = sk::Paint::default();
        if let Some(sh) =
            sk::LinearGradient::new(sk::Point { x: 0.0, y: 0.0 }, sk::Point { x: 0.0, y: h as f32 }, stops, sk::SpreadMode::Pad, sk::Transform::default())
        {
            paint.shader = sh;
            let rect = sk::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
            pm.fill_path(&sk::PathBuilder::from_rect(rect), &paint, sk::FillRule::Winding, sk::Transform::default(), None);
        }
        // ball: horizontal sweep, |sine| bounce
        let t = i as f32 / n as f32;
        let x = 30.0 + t * (w as f32 - 60.0);
        let y = (h as f32 - 40.0) - (t * std::f32::consts::PI * 2.0).sin().abs() * 140.0;
        let mut pb = sk::PathBuilder::new();
        pb.push_circle(x, y, 22.0);
        let mut bp = sk::Paint::default();
        bp.set_color_rgba8(255, 210, 40, 255);
        pm.fill_path(&pb.finish().unwrap(), &bp, sk::FillRule::Winding, sk::Transform::default(), None);
        frames.push(pm);
    }
    let delays = vec![80u32; n];
    std::fs::write("/tmp/ball.gif", export_gif(&frames, &delays).unwrap()).unwrap();
    println!("wrote /tmp/ball.gif ({n} frames)");
}
