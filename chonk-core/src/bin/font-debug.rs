use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};

fn trace_n(db: &fontdb::Database, id: fontdb::ID) {
    let Some((source, index)) = db.face_source(id) else { println!("  no source"); return };
    let bytes: Vec<u8> = match &source {
        fontdb::Source::File(p) => std::fs::read(p).unwrap(),
        fontdb::Source::Binary(b) | fontdb::Source::SharedFile(_, b) => (**b).as_ref().to_vec(),
    };
    let font = swash::FontRef {
        data: &bytes,
        offset: index,
        key: swash::CacheKey::new(),
    };
    let gid = font.charmap().map('N');
    let mut ctx = swash::scale::ScaleContext::new();
    let mut scaler = ctx.builder(font).size(100.0).hint(false).build();
    if let Some(outline) = scaler.scale_outline(gid) {
        // Print points in order: an N's diagonal goes top-left->bottom-right (normal)
        // or bottom-left->top-right (И).
        let pts: Vec<(f32, f32)> = outline.points().iter().map(|p| (p.x, p.y)).collect();
        println!("  N outline pts (y-up font coords): {pts:?}");
    } else {
        println!("  no outline for N");
    }
}

fn main() {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    for name in ["Noto Sans", "FreeSans"] {
        let q = fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            ..Default::default()
        };
        match db.query(&q) {
            Some(id) => {
                let f = db.face(id).unwrap();
                println!(
                    "query {name:?} -> {:?} {}",
                    f.families.first().map(|(n, _)| n.clone()),
                    match &f.source {
                        fontdb::Source::File(p) => p.display().to_string(),
                        _ => "(mem)".to_string(),
                    }
                );
                trace_n(&db, id);
            }
            None => println!("query {name:?} -> NONE"),
        }
    }

    let mut fs = FontSystem::new();
    for fam in [Family::Name("Noto Sans"), Family::SansSerif] {
        let attrs = Attrs::new().family(fam.clone());
        let mut buf = Buffer::new(&mut fs, Metrics::relative(64.0, 1.2));
        buf.set_text("N", &attrs, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut fs, false);
        for run in buf.layout_runs() {
            for g in run.glyphs {
                println!("cosmic {fam:?}: glyph_id={}", g.glyph_id);
            }
        }
    }
}
