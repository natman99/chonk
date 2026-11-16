use chonk_core::codecs::load_image;
fn main() {
    let mut args = std::env::args().skip(1).peekable();
    while let Some(path) = args.next() {
        let pm = match load_image(&std::fs::read(&path).unwrap()) {
            Ok(pm) => pm,
            Err(e) => { println!("{path}: load error {e}"); continue }
        };
        let (w, h) = (pm.width(), pm.height());
        println!("{path}: {w}x{h}");
        for (fx, fy) in [(0.25, 0.25), (0.5, 0.3), (0.5, 0.75)] {
            let x = (w as f32 * fx) as usize;
            let y = (h as f32 * fy) as usize;
            let p = pm.pixels()[y * w as usize + x];
            println!("  ({fx},{fy}): rgba = ({}, {}, {}, {})", p.red(), p.green(), p.blue(), p.alpha());
        }
    }
}
