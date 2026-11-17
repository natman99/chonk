//! chonk-fill: fill a .chonk template's slots and export.
//!
//!   chonk-fill template.chonk --set caption="real words" \
//!       --set avatar=/path/to.png --out meme.png [--frame N]
//!
//! `--set name=value`: value is treated as an image path if it resolves
//! to a readable file, otherwise as literal text. `--list` prints slot
//! names. Batch: run once per row of your data (or loop a folder).


fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut template_path = None;
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut out = None;
    let mut list = false;
    let mut frame = 0usize;
    let mut fry_level: Option<String> = None;
    let mut watermarks: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--set" if i + 1 < args.len() => {
                let kv = &args[i + 1];
                if let Some((k, v)) = kv.split_once('=') {
                    sets.push((k.to_string(), v.to_string()));
                } else {
                    eprintln!("--set expects name=value, got {kv}");
                    std::process::exit(2);
                }
                i += 2;
            }
            "--out" if i + 1 < args.len() => {
                out = Some(args[i + 1].clone());
                i += 2;
            }
            "--frame" if i + 1 < args.len() => {
                frame = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--list" => {
                list = true;
                i += 1;
            }
            "--fry" if i + 1 < args.len() => {
                fry_level = Some(args[i + 1].to_ascii_lowercase());
                i += 2;
            }
            "--watermark" if i + 1 < args.len() => {
                watermarks.push(args[i + 1].clone());
                i += 2;
            }
            s if !s.starts_with("--") && template_path.is_none() => {
                template_path = Some(s.to_string());
                i += 1;
            }
            other => {
                eprintln!("unrecognized argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(path) = template_path else {
        eprintln!("usage: chonk-fill template.chonk [--list | --set name=value ... --out out.png]");
        std::process::exit(2);
    };

    let text = std::fs::read_to_string(&path).expect("cannot read template");
    let mut doc = chonk_core::save::from_json(&text).expect("invalid .chonk");

    if list {
        for (name, kind) in chonk_core::template::slot_names(&doc) {
            let kind = match kind {
                chonk_core::SlotKind::Text => "text",
                chonk_core::SlotKind::Image => "image",
            };
            println!("{name} ({kind})");
        }
        return;
    }

    for (name, value) in &sets {
        let fill_path = std::path::Path::new(value);
        let filled = if fill_path.is_file() {
            let img = image::open(fill_path).expect("cannot open fill image");
            let rgba = img.to_rgba8();
            let pm = chonk_core::codecs::rgba_to_pixmap(&rgba);
            chonk_core::template::fill_image(&mut doc, name, &pm)
        } else {
            chonk_core::template::fill_text(&mut doc, name, value)
        };
        if !filled {
            eprintln!("no {name} slot (or kind mismatch) in {path}");
            std::process::exit(1);
        }
    }

    for wm in &watermarks {
        let (plat, text) = wm.split_once('=').unwrap_or((wm.as_str(), "@reposted"));
        let Some(style) = chonk_core::watermark::WatermarkStyle::parse(plat) else {
            eprintln!("unknown watermark platform: {plat} (tiktok|instagram|youtube|snapchat|reddit)");
            std::process::exit(2);
        };
        chonk_core::watermark::add_watermark(&mut doc, style, text);
    }

    let out_path = out.expect("--out required when filling");
    let mut renderer = chonk_core::Renderer::new();
    let mut pm = renderer.composite_at(&doc, frame);
    if let Some(level) = fry_level {
        let spec = match level.as_str() {
            "mild" => chonk_core::fry::FRY_MILD,
            "medium" => chonk_core::fry::FRY_MEDIUM,
            "heavy" => chonk_core::fry::FRY_HEAVY,
            "ultra" => chonk_core::fry::FRY_ULTRA,
            other => {
                eprintln!("--fry expects mild|medium|heavy|ultra, got {other}");
                std::process::exit(2);
            }
        };
        pm = chonk_core::fry::fry(&pm, &spec);
    }
    let ext = std::path::Path::new(&out_path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "png".into());
    let bytes = match ext.as_str() {
        "jpg" | "jpeg" => chonk_core::codecs::export_jpeg(&pm, 92),
        "webp" => chonk_core::codecs::export_webp(&pm),
        _ => chonk_core::codecs::export_png(&pm),
    }
    .expect("export failed");
    std::fs::write(&out_path, bytes).expect("cannot write output");
    println!("wrote {out_path}");
}
