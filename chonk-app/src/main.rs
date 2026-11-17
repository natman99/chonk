//! Chonk — a meme/quote-card editor. GPUI chrome, chonk-core engine.

use chonk_core::document::{Background, Document, LayerKind, Shadow, TextAlign, TextStyle};
use chonk_core::quote::{QuoteSpec, quote_document};
use chonk_core::render::Renderer;
use chonk_core::text::TextEngine;
use chonk_core::tiny_skia as sk;
use gpui::{
    App, AsyncApp, Bounds, ClickEvent, ClipboardEntry, Context, Corners, ElementInputHandler,
    EntityInputHandler, ExternalPaths, FocusHandle, KeyBinding, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, RenderImage, ScrollDelta, ScrollEvent, SharedString,
    UTF16Selection, Window, WindowBounds, WindowOptions, actions, canvas, div, fill, point,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

actions!(
    chonk,
    [
        ToggleQuote,
        Undo,
        Redo,
        AddText,
        DeleteLayer,
        NudgeUp,
        NudgeDown,
        NudgeLeft,
        NudgeRight,
        TogglePlay,
        EditCaption,
        FinishEdit,
        DeleteChar,
        CaretLeft,
        CaretRight,
        ExportPng,
        ExportJpg,
        ExportWebp,
        ExportGif,
        ResetView,
        ZoomIn,
        ZoomOut,
        OpenFile,
        SaveAs,
        CycleUp,
        CycleDown,
        ToggleTheme,
        OpacityUp,
        OpacityDown,
        BlendPrev,
        BlendNext,
        ToggleSlot,
        FryLayer,
        CopyLayer,
        Paste,
        CancelPending,
        PasteText,
        UndoText,
        RedoText,
    ]
);

/// Key context this view installs while focused (Shale's pattern: bindings
/// are scoped to a focused, tracked view — not dispatched globally).
const CHONK_CTX: &str = "chonk";
/// Swapped in while editing a caption: printable keys must reach the input
/// handler instead of firing app shortcuts.
const CHONK_EDIT_CTX: &str = "chonk-edit";

/// Largest char boundary <= `i` (str::floor_char_boundary is unstable).
fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut i = i;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary >= `i`.
fn ceil_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Layer panel width and the gap between canvas and panel (window px).
const PANEL_W: f32 = 260.0;
const GAP: f32 = 8.0;

/// Minimal theme palette. Dark is the default; a light theme is planned —
/// every color in the UI should come from here, not an inline rgb literal.
struct Theme {
    bg: u32,
    panel: u32,
    border: u32,
    text: u32,
    dim: u32,
    accent: u32,
    sel: u32,
    danger: u32,
    caret: u32,
}

const DARK: Theme = Theme {
    bg: 0x101014,
    panel: 0x17171e,
    border: 0x3a3a46,
    text: 0xe8e8ee,
    dim: 0x8f8f9c,
    accent: 0x22d3ee,
    sel: 0x23233a,
    danger: 0xf87171,
    caret: 0xf59e0b,
};

const LIGHT: Theme = Theme {
    bg: 0xd9d9e0,
    panel: 0xcbcbd6,
    border: 0x9494a2,
    text: 0x1a1a22,
    dim: 0x555562,
    accent: 0x0e7490,
    sel: 0xb9d8e4,
    danger: 0xc23636,
    caret: 0xc2410c,
};

/// Short human name for a layer row.
fn layer_name(l: &chonk_core::document::Layer) -> String {
    match &l.kind {
        LayerKind::Text { text, .. } => {
            let t: String = text.chars().take(18).collect();
            if text.chars().count() > 18 {
                format!("“{t}…”")
            } else if t.is_empty() {
                "(empty)".to_string()
            } else {
                format!("“{t}”")
            }
        }
        LayerKind::Image { pixmap } => format!("image {}×{}", pixmap.width(), pixmap.height()),
        LayerKind::AnimatedImage { image } => format!("gif {} frames", image.frame_count()),
        LayerKind::Svg { .. } => "svg".to_string(),
    }
}

struct MemeView {
    doc: Document,
    /// UI-thread renderer, used ONLY for layer sizes (hit-testing/outline);
    /// all rasterization happens on the render worker.
    renderer: Renderer,
    composited: Option<Arc<RenderImage>>,
    /// Last composited pixmap (palette extraction source).
    last_pixmap: Option<Arc<sk::Pixmap>>,
    /// Extracted palette swatches (click = recolor selected caption).
    palette: Vec<sk::Color>,
    /// In-app layer clipboard (Ctrl+C / Ctrl+V) — deliberately NOT the
    /// system clipboard; system paste is only a Ctrl+V fallback.
    layer_clip: Option<chonk_core::Layer>,
    /// T is armed and waiting for the placement click.
    pending_text_place: bool,
    /// Per-edit-session text history: (text, caret) snapshots taken at
    /// BATCH boundaries (a burst of same-kind edits within 600ms = one
    /// batch). Ctrl+Z while editing walks these and never touches the
    /// canvas undo stack or document structure.
    text_undo: Vec<(String, usize)>,
    text_redo: Vec<(String, usize)>,
    last_text_edit: Option<(std::time::Instant, bool /* was_insert */)>,
    job_tx: std::sync::mpsc::Sender<RenderJob>,
    result_rx: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<WorkerMsg>>>,
    seq: u64,
    applied_seq: u64,
    in_flight: bool,
    selected: Option<usize>,
    /// (layer index, last mouse x, last mouse y) in window pixels.
    drag: Option<(usize, f32, f32)>,
    /// Set by any mutation; render() sends at most one job per frame
    /// (mouse-move events fire far faster than frames).
    dirty: bool,
    /// Union of dirty document rects since the last job (drag path),
    /// or None = full canvas.
    dirty_region: Option<(f32, f32, f32, f32)>,
    /// Size of the square canvas viewport in window pixels + its origin
    /// (set each render), used to map mouse coords into document space.
    view_side: f32,
    view_origin: (f32, f32),
    /// Canvas zoom (1.0 = fit) and pan offset in window px.
    zoom: f32,
    pan: (f32, f32),
    /// Pan gesture in flight: (start_x, start_y, start_pan).
    pan_drag: Option<(f32, f32, (f32, f32))>,
    quote_mode: bool,
    playing: bool,
    frame: usize,
    /// True while the selected text layer receives typed characters.
    edit_mode: bool,
    /// One-shot message to surface in the window title (export results).
    status: Option<String>,
    /// Dark theme by default; Ctrl+T toggles.
    dark: bool,
    /// True while the worker is encoding an export — draws a badge.
    busy: bool,
    /// Caret byte-offset within the caption being edited (always a char
    /// boundary). UI-side text measurement uses `texts`.
    caret_pos: usize,
    texts: TextEngine,
    /// Snapshot undo/redo. Document clones are cheap at meme scale
    /// (Pixmap memcpy ~ms); history is capped.
    undo_stack: Vec<Document>,
    redo_stack: Vec<Document>,
    /// True once the in-progress drag has taken its snapshot (so a pure
    /// click, or a drag that never moves, creates no undo step).
    drag_snapshotted: bool,
    last_snap_tag: Option<&'static str>,
    focus_handle: FocusHandle,
}

/// Load an image/gif/svg file into the document as a new top layer.
fn load_path(doc: &mut Document, path: &str) -> Result<(), String> {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    if ext == "chonk" {
        // A whole document replaces the current one.
        *doc = chonk_core::save::load_chonk(std::path::Path::new(path))?;
        return Ok(());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    match ext.as_str() {
        "gif" => {
            let anim = chonk_core::codecs::load_gif(&bytes).map_err(|e| e.to_string())?;
            doc.add_animated(anim);
        }
        "svg" => {
            let tree = chonk_core::svg::parse(&bytes).map_err(|e| e.to_string())?;
            doc.add_svg(tree);
        }
        _ => {
            let pm = chonk_core::codecs::load_image(&bytes).map_err(|e| e.to_string())?;
            doc.add_image(pm);
        }
    }
    Ok(())
}

/// A compositing request for the background render worker.
struct RenderJob {
    seq: u64,
    doc: Document,
    frame: usize,
    /// Document-space rect to patch, or None = full canvas.
    region: Option<(f32, f32, f32, f32)>,
    /// Optional file to write after compositing (encoding stays off the UI
    /// thread; gif re-composites every frame with the worker's caches).
    export: Option<(String, String)>, // (path, format)
    /// Preview resolution: composite at doc*scale (zoomed-out display).
    /// Export jobs always force 1.0.
    scale: f32,
}

/// What the worker sends back.
enum WorkerMsg {
    Frame(u64, sk::Pixmap),
    Status(String),
}

/// Worker thread owning the Renderer (and its caches) plus the last full
/// canvas; region jobs patch its copy. The UI thread never rasterizes.
fn spawn_render_worker() -> (
    std::sync::mpsc::Sender<RenderJob>,
    std::sync::mpsc::Receiver<WorkerMsg>,
) {
    let (jt, jr) = std::sync::mpsc::channel::<RenderJob>();
    let (rt, rr) = std::sync::mpsc::channel::<WorkerMsg>();
    std::thread::spawn(move || {
        let mut renderer = Renderer::new();
        let mut canvas: Option<sk::Pixmap> = None;
        let mut canvas_scale: f32 = 1.0;
        let mut last_seq: u64 = 0;
        while let Ok(mut job) = jr.recv() {
            // Coalesce queued jobs into one (newest doc/frame, union region).
            while let Ok(j2) = jr.try_recv() {
                job.region = match (job.region, j2.region) {
                    (Some((a0, b0, a1, b1)), Some((c0, d0, c1, d1))) => {
                        Some((a0.min(c0), b0.min(d0), a1.max(c1), b1.max(d1)))
                    }
                    (None, _) | (_, None) => None,
                };
                job.doc = j2.doc;
                job.frame = j2.frame;
                job.seq = j2.seq;
                job.export = j2.export.or(job.export);
                job.scale = j2.scale;
            }
            if job.seq <= last_seq {
                continue;
            }
            last_seq = job.seq;
            // Exports must be full resolution.
            if job.export.is_some() {
                job.scale = 1.0;
            }
            let pm = if job.scale < 0.999 {
                // Zoomed-out preview: composite at display resolution.
                renderer.composite_scaled(&job.doc, job.frame, job.scale)
            } else if let (Some((x0, y0, x1, y1)), Some(base)) = (job.region, canvas.as_ref())
            && canvas_scale == 1.0
            {
                let x = x0.max(0.0).floor() as u32;
                let y = y0.max(0.0).floor() as u32;
                let w = (x1.floor().max(x as f32 + 1.0)) as u32 - x;
                let h = (y1.floor().max(y as f32 + 1.0)) as u32 - y;
                let mut c = base.clone();
                let region = renderer.render_region_at(&job.doc, x, y, w, h, job.frame);
                Renderer::overlay_region(&mut c, &region, x, y);
                c
            } else {
                renderer.composite_at(&job.doc, job.frame)
            };
            canvas_scale = job.scale;
            if let Some((path, fmt)) = &job.export {
                let encoded = match fmt.as_str() {
                    "gif" => {
                        let (frames, delays) = renderer.composite_animated(&job.doc);
                        chonk_core::codecs::export_gif(&frames, &delays)
                    }
                    "jpg" => chonk_core::codecs::export_jpeg(&pm, 92),
                    "webp" => chonk_core::codecs::export_webp(&pm),
                    _ => chonk_core::codecs::export_png(&pm),
                };
                let msg = match encoded {
                    Ok(bytes) => match std::fs::write(path, bytes) {
                        Ok(()) => format!("saved {path}"),
                        Err(e) => format!("export failed: {e}"),
                    },
                    Err(e) => format!("export failed: {e}"),
                };
                if rt.send(WorkerMsg::Status(msg)).is_err() {
                    break;
                }
            }
            canvas = Some(pm.clone());
            if rt.send(WorkerMsg::Frame(job.seq, pm)).is_err() {
                break; // view gone
            }
        }
    });
    (jt, rr)
}

/// Untransformed layer size in document pixels (free fn so event handlers
/// can borrow doc and renderer disjointly).
fn layer_size_of(doc: &Document, renderer: &mut Renderer, i: usize) -> Option<(f32, f32)> {
    let layer = doc.layers.get(i)?;
    Some(match &layer.kind {
        LayerKind::Image { pixmap } => (pixmap.width() as f32, pixmap.height() as f32),
        LayerKind::AnimatedImage { image } => {
            let f = image.frame(0);
            (f.width() as f32, f.height() as f32)
        }
        LayerKind::Svg { tree } => {
            let pm = chonk_core::svg::render(tree);
            (pm.width() as f32, pm.height() as f32)
        }
        LayerKind::Text { text, style } => {
            match renderer.text.render(text, style) {
                Some(rt) if rt.pixmap.width() > 0 && rt.pixmap.height() > 0 => {
                    (rt.pixmap.width() as f32, rt.pixmap.height() as f32)
                }
                // Empty caption: placeholder box so it still has a selection
                // outline and stays clickable while being edited.
                _ => ((style.font_size * 2.0).max(60.0), style.font_size * 1.2),
            }
        }
    })
}

impl MemeView {
    fn new(cx: &mut Context<Self>) -> Self {
        let (job_tx, rr) = spawn_render_worker();
        let mut v = Self {
            doc: Self::meme_doc(),
            renderer: Renderer::new(),
            composited: None,
            last_pixmap: None,
            palette: Vec::new(),
            layer_clip: None,
            pending_text_place: false,
            text_undo: Vec::new(),
            text_redo: Vec::new(),
            last_text_edit: None,
            job_tx,
            result_rx: Arc::new(std::sync::Mutex::new(rr)),
            seq: 0,
            applied_seq: 0,
            in_flight: false,
            selected: None,
            drag: None,
            dirty: false,
            dirty_region: None,
            view_side: 0.0,
            view_origin: (0.0, 0.0),
            zoom: 1.0,
            pan: (0.0, 0.0),
            pan_drag: None,
            quote_mode: false,
            playing: false,
            frame: 0,
            edit_mode: false,
            status: None,
            dark: true,
            busy: false,
            caret_pos: 0,
            texts: TextEngine::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            drag_snapshotted: false,
            last_snap_tag: None,
            focus_handle: cx.focus_handle(),
        };
        v.recomposite();
        // Result pump: block on the worker's channel off the UI thread,
        // apply finished frames back on it.
        let rx = v.result_rx.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                let got = {
                    let rx = rx.clone();
                    cx.background_executor()
                        .spawn(async move { rx.lock().unwrap().recv() })
                        .await
                };
                match got {
                    Ok(WorkerMsg::Frame(seq, pm)) => {
                        let _ = this.update(cx, |v, cx| v.apply_frame(seq, pm, cx));
                    }
                    Ok(WorkerMsg::Status(msg)) => {
                        let _ = this.update(cx, |v, _cx| {
                            eprintln!("[chonk] {msg}");
                            v.busy = false;
                            v.status = Some(msg);
                        });
                    }
                    Err(_) => break, // view dropped
                }
            }
        })
        .detach();
        v
    }

    fn apply_frame(&mut self, seq: u64, pm: sk::Pixmap, cx: &mut Context<Self>) {
        if seq < self.applied_seq {
            return; // stale
        }
        self.applied_seq = seq;
        self.in_flight = false;
        let pm = Arc::new(pm);
        self.last_pixmap = Some(pm.clone());
        self.composited = Some(Arc::new(render_image_from_pixmap(&pm)));
        // If more happened while the job ran, render() sends the next job.
        cx.notify();
    }

    fn meme_doc() -> Document {
        let (w, h) = (1080u32, 1080u32);
        let mut doc = Document::new(w, h);
        doc.background = Background::LinearGradient {
            x1: 0.0,
            y1: 0.0,
            x2: w as f32,
            y2: h as f32,
            stops: vec![
                (0.0, sk::Color::from_rgba8(30, 30, 90, 255)),
                (1.0, sk::Color::from_rgba8(230, 120, 40, 255)),
            ],
        };
        doc.add_text(
            "CHONK",
            TextStyle {
                font_family: "Noto Sans".into(),
                font_size: 260.0,
                fill: sk::Color::from_rgba8(255, 255, 255, 255),
                stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 16.0)),
                shadow: Some(Shadow {
                    dx: 10.0,
                    dy: 14.0,
                    blur: 10.0,
                    color: sk::Color::from_rgba8(0, 0, 0, 170),
                }),
                align: TextAlign::Center,
                max_width: Some(1000.0),
                fit_box: None,
                ..TextStyle::default()
            },
        );
        doc.add_text(
            "when the meme opens in a single frame",
            TextStyle {
                font_family: "Noto Sans".into(),
                font_size: 64.0,
                fill: sk::Color::from_rgba8(255, 255, 255, 255),
                stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 6.0)),
                shadow: None,
                align: TextAlign::Center,
                max_width: Some(900.0),
                fit_box: Some((900.0, 400.0)),
                ..TextStyle::default()
            },
        );
        doc.layers[0].transform =
            chonk_core::Transform { x: 0.0, y: 140.0, ..Default::default() };
        doc.layers[1].transform =
            chonk_core::Transform { x: 0.0, y: 640.0, ..Default::default() };
        doc
    }

    fn toggle_quote(&mut self) {
        self.snapshot();
        self.quote_mode = !self.quote_mode;
        self.doc = if self.quote_mode {
            let spec =
                QuoteSpec::new("It is not enough to succeed; others must fail.", "Caligula");
            quote_document(&mut self.renderer.text, &spec)
        } else {
            Self::meme_doc()
        };
        self.drag = None;
        self.selected = None;
        self.recomposite();
    }

    /// T arms placement mode: the next canvas click drops a caption there.
    fn add_text(&mut self) {
        self.pending_text_place = true;
        self.status = Some("click to place a caption (Esc cancels)".into());
    }

    fn place_text_at(&mut self, dx: f32, dy: f32) {
        self.pending_text_place = false;
        self.snapshot();
        const MW: f32 = 900.0;
        const FS: f32 = 72.0;
        self.doc.add_text(
            "",
            TextStyle {
                font_family: "Noto Sans".into(),
                font_size: FS,
                fill: sk::Color::from_rgba8(255, 255, 255, 255),
                stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 6.0)),
                align: TextAlign::Center,
                max_width: Some(MW),
                ..TextStyle::default()
            },
        );
        // Center the (center-aligned) text box on the click point.
        if let Some(l) = self.doc.layers.last_mut() {
            l.transform.x = dx - MW / 2.0;
            l.transform.y = dy - FS * 0.6;
        }
        self.selected = Some(self.doc.layers.len() - 1);
        // Go straight into editing: an empty caption shows its placeholder
        // box and is pruned if left empty on finish (no ghost layers).
        self.caret_pos = 0;
        self.edit_mode = true;
        self.recomposite();
    }

    /// Ctrl+C: copy the selected layer into the in-app clipboard. The
    /// system clipboard is intentionally left alone.
    fn copy_layer(&mut self) {
        if let Some(i) = self.selected {
            self.layer_clip = Some(self.doc.layers[i].clone());
            self.status = Some("layer copied (in-app clipboard)".into());
        }
    }

    /// Ctrl+V: in-app layer clipboard wins; otherwise paste from the
    /// system clipboard (files, an image, or plain text).
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(l) = self.layer_clip.clone() {
            self.snapshot();
            let mut l = l;
            l.transform.x += 20.0;
            l.transform.y += 20.0;
            self.doc.layers.push(l);
            self.selected = Some(self.doc.layers.len() - 1);
            self.status = Some("layer pasted".into());
            self.recomposite();
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            self.status = Some("clipboard is empty".into());
            return;
        };
        let mut loaded = 0usize;
        let mut snapshotted = false;
        for entry in item.entries {
            match entry {
                ClipboardEntry::ExternalPaths(ep) => {
                    for p in ep.paths() {
                        if !snapshotted {
                            self.snapshot();
                            snapshotted = true;
                        }
                        match load_path(&mut self.doc, &p.to_string_lossy()) {
                            Ok(()) => {
                                self.selected = Some(self.doc.layers.len() - 1);
                                loaded += 1;
                            }
                            Err(e) => self.status = Some(format!("paste failed: {e}")),
                        }
                    }
                }
                ClipboardEntry::Image(img) => {
                    match chonk_core::codecs::load_image(&img.bytes) {
                        Ok(pm) => {
                            if !snapshotted {
                                self.snapshot();
                                snapshotted = true;
                            }
                            self.doc.add_image(pm);
                            self.selected = Some(self.doc.layers.len() - 1);
                            loaded += 1;
                        }
                        Err(e) => self.status = Some(format!("paste failed: {e}")),
                    }
                }
                ClipboardEntry::String(s) => {
                    let t = s.text.trim().to_string();
                    if std::path::Path::new(&t).is_file() {
                        if !snapshotted {
                            self.snapshot();
                            snapshotted = true;
                        }
                        match load_path(&mut self.doc, &t) {
                            Ok(()) => {
                                self.selected = Some(self.doc.layers.len() - 1);
                                loaded += 1;
                            }
                            Err(e) => self.status = Some(format!("paste failed: {e}")),
                        }
                    } else if !t.is_empty() {
                        if !snapshotted {
                            self.snapshot();
                            snapshotted = true;
                        }
                        self.doc.add_text(
                            t,
                            TextStyle {
                                font_family: "Noto Sans".into(),
                                font_size: 48.0,
                                fill: sk::Color::from_rgba8(255, 255, 255, 255),
                                stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 5.0)),
                                align: TextAlign::Center,
                                max_width: Some(900.0),
                                ..TextStyle::default()
                            },
                        );
                        self.selected = Some(self.doc.layers.len() - 1);
                        loaded += 1;
                    }
                }
            }
        }
        if loaded > 0 {
            self.status = Some(format!("pasted {loaded} item(s)"));
            self.recomposite();
        } else if self.status.as_deref().map(|s| s.starts_with("paste failed")) != Some(true) {
            self.status = Some("nothing to paste on the clipboard".into());
        }
    }

    /// Push the current document onto the undo stack BEFORE a mutation.
    fn snapshot(&mut self) {
        self.undo_stack.push(self.doc.clone());
        if self.undo_stack.len() > 64 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.last_snap_tag = None;
    }

    /// Snapshot, but repeated same-tag actions (holding an arrow key,
    /// holding [ for opacity) coalesce into ONE undo step.
    fn snapshot_tagged(&mut self, tag: &'static str) {
        if self.last_snap_tag == Some(tag) {
            return;
        }
        self.snapshot();
        self.last_snap_tag = Some(tag);
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.doc.clone());
            self.doc = prev;
            self.after_history_change();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.doc.clone());
            self.doc = next;
            self.after_history_change();
        }
    }

    fn after_history_change(&mut self) {
        self.edit_mode = false;
        self.drag = None;
        if self.selected.map_or(false, |i| i >= self.doc.layers.len()) {
            self.selected = None;
        }
        self.recomposite();
    }

    fn delete_layer(&mut self) {
        if let Some(i) = self.selected {
            if i < self.doc.layers.len() {
                self.snapshot();
                self.doc.layers.remove(i);
                self.selected = None;
                self.recomposite();
            }
        }
    }

    fn nudge(&mut self, dx: f32, dy: f32) {
        if let Some(i) = self.selected {
            self.snapshot_tagged("nudge");
            if let Some(l) = self.doc.layers.get_mut(i) {
                l.transform.x += dx;
                l.transform.y += dy;
            }
            self.clamp_layer_pos(i);
            self.recomposite();
        }
    }

    /// Keep a layer on the canvas: fully inside when it fits; when it is
    /// larger than the document it can still slide, but the canvas must stay
    /// covered by it (x in [docw - w, 0]).
    fn clamp_layer_pos(&mut self, i: usize) {
        let Some((w0, h0)) = self.layer_size(i) else { return };
        let (docw, doch) = (self.doc.width as f32, self.doc.height as f32);
        if let Some(l) = self.doc.layers.get_mut(i) {
            let s = l.transform.scale.max(0.01);
            let (w, h) = (w0 * s, h0 * s);
            l.transform.x = l.transform.x.clamp((docw - w).min(0.0), (docw - w).max(0.0));
            l.transform.y = l.transform.y.clamp((doch - h).min(0.0), (doch - h).max(0.0));
        }
    }

    /// Untransformed layer size in document pixels (text uses the render
    /// cache, so this is cheap for unchanged layers).
    fn layer_size(&mut self, i: usize) -> Option<(f32, f32)> {
        layer_size_of(&self.doc, &mut self.renderer, i)
    }

    /// Topmost visible layer whose (scaled) box contains the document point.
    fn hit_test(&mut self, dx: f32, dy: f32) -> Option<usize> {
        self.hit_stack(dx, dy).first().copied()
    }

    /// All visible layers whose (scaled) box contains the point, top first.
    fn hit_stack(&mut self, dx: f32, dy: f32) -> Vec<usize> {
        let mut out = Vec::new();
        for i in (0..self.doc.layers.len()).rev() {
            if !self.doc.layers[i].visible {
                continue;
            }
            let l = &self.doc.layers[i];
            let (tx, ty, scale) = (l.transform.x, l.transform.y, l.transform.scale);
            // layer_size needs &mut self; copy the transform out first.
            let Some((w, h)) = self.layer_size_checked(i) else { continue };
            let s = scale.max(0.01);
            if dx >= tx && dx <= tx + w * s && dy >= ty && dy <= ty + h * s {
                out.push(i);
            }
        }
        out
    }

    /// Tab / Shift+Tab: cycle selection through the layer stack (wrapping).
    fn cycle_layer(&mut self, dir: i32) {
        let n = self.doc.layers.len();
        if n == 0 {
            return;
        }
        self.selected = match self.selected {
            Some(i) => {
                let j = if dir > 0 { i + 1 } else { i + n - 1 };
                Some(j % n)
            }
            None => Some(n - 1),
        };
        self.edit_mode = false;
    }

    fn layer_size_checked(&mut self, i: usize) -> Option<(f32, f32)> {
        if i < self.doc.layers.len() { self.layer_size(i) } else { None }
    }

    /// Window pixel -> document pixel (canvas is a centered square).
    fn to_doc(&self, wx: f32, wy: f32) -> (f32, f32) {
        let s = self.doc.width as f32 / self.view_side.max(1.0);
        ((wx - self.view_origin.0) * s, (wy - self.view_origin.1) * s)
    }

    fn frame_delay_ms(&self) -> u64 {
        self.doc
            .layers
            .iter()
            .find_map(|l| match &l.kind {
                LayerKind::AnimatedImage { image } => {
                    Some(image.delays_ms[self.frame % image.delays_ms.len()] as u64)
                }
                _ => None,
            })
            .unwrap_or(100)
            .max(20)
    }

    fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        self.playing = playing;
        if !playing {
            self.recomposite();
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                let Some(delay) = this
                    .update(cx, |v, _cx| if v.playing { Some(v.frame_delay_ms()) } else { None })
                    .ok()
                    .flatten()
                else {
                    break;
                };
                cx.background_executor().timer(Duration::from_millis(delay)).await;
                let _ = this.update(cx, |v, cx| {
                    if v.playing {
                        v.frame += 1;
                        v.recomposite();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// Full recomposite requested; actual work happens once in render().
    fn invalidate(&mut self) {
        self.dirty = true;
        self.dirty_region = None;
    }

    /// Mark a document-space rect dirty (union-accumulated until the frame).
    fn invalidate_region(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.dirty = true;
        match &mut self.dirty_region {
            Some((ax0, ay0, ax1, ay1)) => {
                *ax0 = ax0.min(x0);
                *ay0 = ay0.min(y0);
                *ax1 = ax1.max(x1);
                *ay1 = ay1.max(y1);
            }
            None => self.dirty_region = Some((x0, y0, x1, y1)),
        }
    }

    /// Selected layer is editable text?
    fn selected_text(&self) -> Option<&str> {
        let i = self.selected?;
        match &self.doc.layers.get(i)?.kind {
            LayerKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    fn edit_caption(&mut self, window: &mut Window) {
        let len = self.selected_text().map(|t| t.len());
        if let Some(len) = len {
            // One undo step per editing session, not per keystroke.
            self.snapshot();
            self.text_undo.clear();
            self.text_redo.clear();
            self.last_text_edit = None;
            self.caret_pos = len;
            self.edit_mode = true;
        }
    }

    /// Text-batch bookkeeping: snapshot the caption at batch boundaries
    /// only, so undo steps back over whole bursts (a typed word, a held
    /// backspace), not single keystrokes. `is_insert` distinguishes
    /// typing from deleting; a >600ms gap or a kind flip starts a batch.
    fn begin_text_batch(&mut self, is_insert: bool) {
        let Some(i) = self.selected else { return };
        let now = std::time::Instant::now();
        let fresh = match self.last_text_edit {
            Some((t, was_insert)) => {
                was_insert != is_insert || now.duration_since(t) > Duration::from_millis(600)
            }
            None => true,
        };
        self.last_text_edit = Some((now, is_insert));
        if fresh {
            if let Some(LayerKind::Text { text, .. }) =
                self.doc.layers.get(i).map(|l| &l.kind)
            {
                self.text_undo.push((text.clone(), self.caret_pos));
                self.text_redo.clear();
                if self.text_undo.len() > 128 {
                    self.text_undo.remove(0);
                }
            }
        }
    }

    /// Ctrl+Z while editing: undo one text batch in the caption only.
    /// Deliberately does NOT touch the canvas undo stack or the layer.
    fn undo_text(&mut self) {
        let Some(i) = self.selected else { return };
        let Some((prev, caret)) = self.text_undo.pop() else { return };
        if let Some(LayerKind::Text { text, .. }) =
            self.doc.layers.get_mut(i).map(|l| &mut l.kind)
        {
            self.text_redo.push((text.clone(), self.caret_pos));
            *text = prev;
            self.caret_pos = caret.min(text.len());
            // Next edit must start a fresh batch, not merge into the
            // undone one.
            self.last_text_edit = None;
            self.invalidate();
        }
    }

    fn redo_text(&mut self) {
        let Some(i) = self.selected else { return };
        let Some((next, caret)) = self.text_redo.pop() else { return };
        if let Some(LayerKind::Text { text, .. }) =
            self.doc.layers.get_mut(i).map(|l| &mut l.kind)
        {
            self.text_undo.push((text.clone(), self.caret_pos));
            *text = next;
            self.caret_pos = caret.min(text.len());
            self.last_text_edit = None;
            self.invalidate();
        }
    }

    /// Caret geometry (x, y_top, height) in DOCUMENT coords for the caption
    /// being edited; None if unavailable (falls back to text-box edge).
    fn caret_doc_geom(&mut self) -> Option<(f32, f32, f32)> {
        if !self.edit_mode {
            return None;
        }
        let i = self.selected?;
        let (text, style) = match self.doc.layers.get(i).map(|l| l.kind.clone()) {
            Some(LayerKind::Text { text, style }) => (text, style),
            _ => return None,
        };
        let (lx, ly, s) = self
            .doc
            .layers
            .get(i)
            .map(|l| (l.transform.x, l.transform.y, l.transform.scale.max(0.01)))
            .unwrap_or((0.0, 0.0, 1.0));
        let (x, y_top, h) = self.texts.caret_geometry(&text, &style, self.caret_pos)?;
        Some((lx + x * s, ly + y_top * s, h * s))
    }

    /// Move the caret to the glyph nearest a document-space point.
    fn position_caret_at(&mut self, doc_x: f32, doc_y: f32) {
        let Some(i) = self.selected else { return };
        let (text, style) = match self.doc.layers.get(i).map(|l| l.kind.clone()) {
            Some(LayerKind::Text { text, style }) => (text, style),
            _ => return,
        };
        let (lx, ly, s) = self
            .doc
            .layers
            .get(i)
            .map(|l| (l.transform.x, l.transform.y, l.transform.scale.max(0.01)))
            .unwrap_or((0.0, 0.0, 1.0));
        if let Some(idx) = self.texts.hit_byte(&text, &style, (doc_x - lx) / s, (doc_y - ly) / s) {
            self.caret_pos = floor_char_boundary(&text, idx.min(text.len()));
        }
    }

    /// Zoom by an adaptive factor (gentler at high magnification), anchored
    /// at the canvas center. The canvas center is 12 + ew/2 + pan — it does
    /// not depend on side — so center-anchored zoom needs no pan change.
    fn zoom_at_center(&mut self, base: f32) {
        let speed = 0.22f32.ln() / (1.0 + (self.zoom - 1.0).max(0.0) * 0.35);
        let factor = if base > 1.0 { speed.exp() } else { (-speed).exp() };
        self.zoom = (self.zoom * factor).clamp(0.1, 8.0);
    }

    fn finish_edit(&mut self, window: &mut Window) {
        self.edit_mode = false;
        self.prune_empty_caption();
    }

    /// An emptied caption is a ghost: remove the layer entirely.
    fn prune_empty_caption(&mut self) {
        if let Some(i) = self.selected {
            let empty = self.doc.layers.get(i).is_some_and(|l| {
                matches!(&l.kind, LayerKind::Text { text, .. } if text.trim().is_empty())
            });
            if empty {
                self.doc.layers.remove(i);
                self.selected = None;
                self.edit_mode = false;
                self.invalidate();
            }
        }
    }

    /// Queue an export job; encoding + file write happen on the worker.
    fn export(&mut self, fmt: &str, window: &mut Window) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = std::env::var("HOME")
            .map(|h| format!("{h}/Downloads"))
            .unwrap_or_else(|_| ".".to_string());
        std::fs::create_dir_all(&dir).ok();
        let path = format!("{dir}/chonk-{ts}.{fmt}");
        self.export_to(std::path::Path::new(&path), fmt);
    }

    /// Export to an explicit path (dialog save-as). Format from the caller.
    fn export_to(&mut self, path: &std::path::Path, fmt: &str) {
        let path = path.display().to_string();
        self.seq += 1;
        let frame = if self.playing { self.frame } else { 0 };
        let job = RenderJob {
            seq: self.seq,
            doc: self.doc.clone(),
            frame,
            region: None,
            export: Some((path.clone(), fmt.to_string())),
            scale: 1.0,
        };
        if self.job_tx.send(job).is_ok() {
            self.busy = true;
            self.status = Some(format!("exporting {path}…"));
        }
    }

    /// Portal file dialogs (rfd/xdg-portal): run on a background thread so
    /// the UI never blocks while the picker is open.
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let picked = cx.background_executor()
                .spawn(async {
                    rfd::FileDialog::new()
                        .set_title("Chonk — open image or document")
                        .add_filter("Images & documents", &["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp", "chonk"])
                        .pick_file()
                })
                .await;
            let Some(p) = picked else { return };
            let _ = me.update(cx, |v, cx| {
                v.snapshot();
                match load_path(&mut v.doc, &p.to_string_lossy()) {
                    Ok(()) => {
                        v.selected = Some(v.doc.layers.len() - 1);
                        v.recomposite();
                        v.status = Some(format!("opened {}", p.display()));
                    }
                    Err(e) => v.status = Some(format!("open failed: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_dialog(&mut self, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let doc = self.doc.clone();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let picked = cx.background_executor()
                .spawn(async {
                    rfd::FileDialog::new()
                        .set_title("Chonk — save as")
                        .set_file_name("chonk.chonk")
                        .add_filter("Chonk document", &["chonk"])
                        .add_filter("PNG", &["png"])
                        .add_filter("JPEG", &["jpg", "jpeg"])
                        .add_filter("WebP", &["webp"])
                        .add_filter("GIF (animated)", &["gif"])
                        .save_file()
                })
                .await;
            let Some(p) = picked else { return };
            // .chonk = the full editable document (serialized off the UI
            // thread); anything else is an image export via the worker.
            if p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) == Some("chonk".to_string()) {
                let path = p.clone();
                let msg = match cx
                    .background_executor()
                    .spawn(async move { chonk_core::save::save_chonk(&doc, &path) })
                    .await
                {
                    Ok(()) => format!("saved {}", p.display()),
                    Err(e) => format!("save failed: {e}"),
                };
                let _ = me.update(cx, |v, cx| {
                    v.status = Some(msg);
                    cx.notify();
                });
                return;
            }
            let fmt = match p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase()
                .as_str()
            {
                "jpg" | "jpeg" => "jpg",
                "webp" => "webp",
                "gif" => "gif",
                _ => "png",
            };
            let _ = me.update(cx, |v, cx| {
                v.export_to(&p, fmt);
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_char(&mut self) {
        if let Some(i) = self.selected {
            let pos = self.caret_pos.min(self.selected_text().map_or(0, |t| t.len()));
            if pos == 0 {
                return;
            }
            self.begin_text_batch(false);
            if let Some(LayerKind::Text { text, .. }) = self.doc.layers.get_mut(i).map(|l| &mut l.kind) {
                let prev = floor_char_boundary(text, pos - 1);
                let prev = if prev == pos { prev.saturating_sub(1) } else { prev };
                if text.is_char_boundary(prev) {
                    text.remove(prev);
                    self.caret_pos = prev;
                    self.invalidate();
                }
            }
        }
        // NOTE: no pruning here — an empty caption mid-edit is still being
        // typed into; pruning happens when editing ENDS (finish_edit or
        // clicking away).
    }

    fn recomposite(&mut self) {
        self.invalidate();
    }
}

/// chonk-core premultiplied-RGBA pixmap -> gpui RenderImage.
/// gpui-pre 0.3.13 expects **BGRA** frames (its own decoders swap RGBA->BGR
/// before upload; see platform.rs decode_static_image_from_decoder), so swap
/// R/B here. (0.3.10 consumed RGBA directly — re-check on every gpui bump.)
fn render_image_from_pixmap(pm: &sk::Pixmap) -> RenderImage {
    let mut buf = pm.data().to_vec();
    for pixel in buf.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), buf)
        .expect("buffer sized to pixmap");
    let frame = image::Frame::new(img);
    RenderImage::new(smallvec::smallvec![frame])
}

impl EntityInputHandler for MemeView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let t = self.selected_text()?.to_string();
        adjusted.replace(range.clone());
        Some(t.get(range)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // v1: caret-at-end append model, no selection.
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _marked_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // v1: no inline IME marking; commit composed text as a normal insert.
        self.replace_text_in_range(range, text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        _element_bounds: Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        None // no IME candidate window to position
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None // v1: caret stays at text end (no click-to-position)
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.edit_mode || text.is_empty() {
            return;
        }
        self.begin_text_batch(true);
        if let Some(i) = self.selected {
            if let Some(LayerKind::Text { text: t, .. }) =
                self.doc.layers.get_mut(i).map(|l| &mut l.kind)
            {
                match range {
                    Some(r) => {
                        let start = floor_char_boundary(t, r.start.min(t.len()));
                        let end = ceil_char_boundary(t, r.end.max(start).min(t.len()));
                        t.replace_range(start..end, text);
                        self.caret_pos = start + text.len();
                    }
                    None => {
                        let at = self.caret_pos.min(t.len());
                        let at = floor_char_boundary(t, at);
                        t.insert_str(at, text);
                        self.caret_pos = at + text.len();
                    }
                }
                self.invalidate();
                cx.notify();
            }
        }
    }
}

impl Render for MemeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Send at most one composite job per frame to the render worker;
        // rasterization never happens on the UI thread.
        if self.dirty && !self.in_flight {
            let frame = if self.playing { self.frame } else { 0 };
            self.seq += 1;
            // Preview resolution: when the canvas displays smaller than the
            // document, composite at display size (dirty-region path is
            // full-res only). Mirror render()'s side math below.
            let ew = _window.bounds().size.width / px(1.0) - 24.0 - GAP - PANEL_W;
            let eh = _window.bounds().size.height / px(1.0) - 24.0 - 30.0; // -status bar+gap
            let side = (ew.min(eh).max(1.0) * self.zoom).max(40.0);
            let scale = (side / self.doc.width as f32).min(1.0).max(0.05);
            let region = if scale < 0.999 {
                self.dirty_region = None;
                None
            } else {
                self.dirty_region.take()
            };
            let job = RenderJob {
                seq: self.seq,
                doc: self.doc.clone(),
                frame,
                region,
                export: None,
                scale,
            };
            if self.job_tx.send(job).is_ok() {
                self.in_flight = true;
                self.dirty = false;
            }
        }
        let image = self.composited.clone();
        let b = _window.bounds();
        let th = if self.dark { &DARK } else { &LIGHT };
        // Canvas element sits inside p_3 padding (12px) and shares the row
        // with the layer panel (PANEL_W + GAP). The image is a centered
        // square within the remaining area. Mirror the paint closure's math
        // so mouse->doc mapping and hit-testing line up with what is drawn.
        let ew = b.size.width / px(1.0) - 24.0 - GAP - PANEL_W;
        let eh = b.size.height / px(1.0) - 24.0 - 30.0; // -status bar+gap
        let base = ew.min(eh).max(1.0);
        let side = (base * self.zoom).max(40.0);
        self.view_side = side;
        self.view_origin = (
            12.0 + (ew - side) / 2.0 + self.pan.0,
            12.0 + (eh - side) / 2.0 + self.pan.1,
        );
        let side_px = px(side);
        let origin_px = point(px(self.view_origin.0), px(self.view_origin.1));

        // Selection outline in document coords (converted to window px in the
        // paint closure); recomputed lazily is fine — selection changes rarely.
        let sel_rect = self.selected.and_then(|i| {
            let (w, h) = self.layer_size(i)?;
            let l = &self.doc.layers[i];
            let s = l.transform.scale.max(0.01);
            Some((l.transform.x, l.transform.y, w * s, h * s))
        });

        // Caret geometry (doc coords) for the caption being edited;
        // None falls back to painting at the text box's right edge.
        let caret_geom = self.caret_doc_geom();

        div()
            .size_full()
            .bg(rgb(th.bg))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
        div()
            .id("chonk-root")
            .key_context(if self.edit_mode { CHONK_EDIT_CTX } else { CHONK_CTX })
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &EditCaption, window, cx| {
                this.edit_caption(window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FinishEdit, window, cx| {
                this.finish_edit(window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DeleteChar, _window, cx| {
                this.delete_char();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CaretLeft, _window, cx| {
                if let Some(t) = this.selected_text() {
                    let pos = this.caret_pos.min(t.len());
                    this.caret_pos = if pos == 0 { 0 } else { floor_char_boundary(t, pos - 1) };
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CaretRight, _window, cx| {
                if let Some(t) = this.selected_text() {
                    this.caret_pos = ceil_char_boundary(t, (this.caret_pos + 1).min(t.len()));
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &UndoText, _window, cx| {
                this.undo_text();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RedoText, _window, cx| {
                this.redo_text();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PasteText, window, cx| {
                // Ctrl+V while editing: insert the clipboard's text at the
                // caret (system clipboard; newlines kept for multi-line
                // captions, other control characters stripped).
                let Some(item) = cx.read_from_clipboard() else { return };
                let s: String = item
                    .entries
                    .iter()
                    .filter_map(|e| match e {
                        ClipboardEntry::String(cs) => Some(cs.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let s: String = s
                    .chars()
                    .filter(|c| *c == '\n' || !c.is_control())
                    .collect();
                if !s.is_empty() {
                    this.replace_text_in_range(None, &s, window, cx);
                    this.status = Some(format!("pasted {} chars into caption", s.chars().count()));
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportPng, window, cx| {
                this.export("png", window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportJpg, window, cx| {
                this.export("jpg", window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportWebp, window, cx| {
                this.export("webp", window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportGif, window, cx| {
                this.export("gif", window);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ResetView, _window, cx| {
                this.zoom = 1.0;
                this.pan = (0.0, 0.0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _window, cx| {
                this.zoom_at_center(1.25);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _window, cx| {
                this.zoom_at_center(0.8);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenFile, _window, cx| this.open_dialog(cx)))
            .on_action(cx.listener(|this, _: &SaveAs, _window, cx| this.save_dialog(cx)))
            .on_action(cx.listener(|this, _: &CycleUp, _window, cx| {
                this.cycle_layer(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CycleDown, _window, cx| {
                this.cycle_layer(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _window, cx| {
                this.dark = !this.dark;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpacityUp, _window, cx| {
                this.adjust_opacity(0.05);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpacityDown, _window, cx| {
                this.adjust_opacity(-0.05);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BlendPrev, _window, cx| {
                this.cycle_blend(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BlendNext, _window, cx| {
                this.cycle_blend(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSlot, _window, cx| {
                this.toggle_slot();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FryLayer, _window, cx| {
                this.fry_selected(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleQuote, _window, cx| {
                this.toggle_quote();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Undo, _window, cx| {
                this.undo();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Redo, _window, cx| {
                this.redo();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &AddText, window, cx| {
                this.add_text();
                    cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyLayer, _window, cx| {
                this.copy_layer();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Paste, _window, cx| {
                this.paste(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CancelPending, _window, cx| {
                if this.pending_text_place {
                    this.pending_text_place = false;
                    this.status = Some("placement cancelled".into());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &DeleteLayer, _window, cx| {
                this.delete_layer();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NudgeUp, _window, cx| {
                this.nudge(0.0, -10.0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NudgeDown, _window, cx| {
                this.nudge(0.0, 10.0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NudgeLeft, _window, cx| {
                this.nudge(-10.0, 0.0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NudgeRight, _window, cx| {
                this.nudge(10.0, 0.0);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePlay, window, cx| {
                let has_anim =
                    this.doc
                        .layers
                        .iter()
                        .any(|l| matches!(l.kind, LayerKind::AnimatedImage { .. }));
                if has_anim {
                    this.set_playing(!this.playing, cx);
                    cx.notify();
                }
            }))
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_row()
            .gap_2()
            .child(
                div()
                    .id("canvas")
                    .flex_1()
                    .h_full()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus_handle, cx);
                        let (dx, dy) = this.to_doc(ev.position.x / px(1.0), ev.position.y / px(1.0));
                        // T armed: this click places the caption.
                        if this.pending_text_place {
                            this.place_text_at(dx, dy);
                            cx.notify();
                            return;
                        }
                        // Topmost wins; Alt+click steps DOWN through the
                        // stack at this point (Illustrator/Inkscape style).
                        let stack = this.hit_stack(dx, dy);
                        let mut new_sel = if ev.modifiers.alt && !stack.is_empty() {
                            match this
                                .selected
                                .and_then(|cur| stack.iter().position(|&i| i == cur))
                            {
                                Some(p) => stack.get(p + 1).or(stack.first()).copied(),
                                None => stack.first().copied(),
                            }
                        } else {
                            stack.first().copied()
                        };
                        // Clicking away from an empty caption being edited:
                        // it is a ghost now — remove it. Clicking ON it
                        // (e.g. to drag it) keeps it alive.
                        if this.edit_mode && new_sel != this.selected {
                            let old = this.selected;
                            this.prune_empty_caption();
                            if this.selected.is_none() && old.is_some() {
                                // Layer removed: shift higher indices.
                                let old = old.unwrap();
                                new_sel = new_sel.map(|s| if s > old { s - 1 } else { s });
                            }
                        }
                        // Clicking the caption being edited positions the
                        // caret instead of dragging (exit editing to drag).
                        if this.edit_mode && new_sel == this.selected {
                            if let Some(i) = new_sel {
                                if matches!(this.doc.layers[i].kind, LayerKind::Text { .. }) {
                                    this.position_caret_at(dx, dy);
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                        if new_sel != this.selected {
                            this.selected = new_sel;
                            this.edit_mode = false; // selection change ends editing
                        }
                        if let Some(layer) = this.selected {
                            this.drag = Some((layer, ev.position.x / px(1.0), ev.position.y / px(1.0)));
                            this.drag_snapshotted = false;
                        } else {
                            // Empty canvas: drag pans the view (GPU-only, no
                            // recomposite).
                            this.pan_drag = Some((
                                ev.position.x / px(1.0),
                                ev.position.y / px(1.0),
                                this.pan,
                            ));
                        }
                        cx.notify();
                    }))
                    .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| {
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y,
                            ScrollDelta::Pixels(p) => p.y / px(18.0),
                        };
                        if dy == 0.0 {
                            return;
                        }
                        // Zoom speed falls off at high magnification: one
                        // notch is ~20% at 1x but only ~5% at 8x, which
                        // kills the jolt when deep-zoomed.
                        let speed = 0.18 / (1.0 + (this.zoom - 1.0).max(0.0) * 0.35);
                        let new_zoom = (this.zoom * (dy * speed).exp()).clamp(0.1, 8.0);
                        let k = new_zoom / this.zoom;
                        // Keep the point under the cursor anchored. origin =
                        // 12 + (ew - side)/2 + pan, so pan must absorb both
                        // the anchor shift AND the centering shift from the
                        // side change: dpan = (pos-origin)(1-k) + ds/2.
                        let cx_ = ev.position.x / px(1.0);
                        let cy_ = ev.position.y / px(1.0);
                        let ds = this.view_side * (k - 1.0);
                        this.pan.0 += (cx_ - this.view_origin.0) * (1.0 - k) + ds / 2.0;
                        this.pan.1 += (cy_ - this.view_origin.1) * (1.0 - k) + ds / 2.0;
                        this.zoom = new_zoom;
                        // Resolution may have changed with zoom: refresh the
                        // composite at the new preview scale.
                        this.invalidate();
                        cx.notify();
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        if let Some((sx, sy, sp)) = this.pan_drag {
                            this.pan = (
                                sp.0 + ev.position.x / px(1.0) - sx,
                                sp.1 + ev.position.y / px(1.0) - sy,
                            );
                            cx.notify();
                            return;
                        }
                        if let Some((layer, lx, ly)) = this.drag.take() {
                            // Snapshot on the first actual movement: a pure
                            // click never creates an undo step.
                            if !this.drag_snapshotted {
                                this.snapshot();
                                this.drag_snapshotted = true;
                            }
                            let mut region_ok = false;
                            let mut rect = (0.0, 0.0, 0.0, 0.0);
                            let mut oldx = 0.0;
                            let mut oldy = 0.0;
                            if let Some(l) = this.doc.layers.get_mut(layer) {
                                let s = this.doc.width as f32 / this.view_side.max(1.0);
                                oldx = l.transform.x;
                                oldy = l.transform.y;
                                l.transform.x += (ev.position.x / px(1.0) - lx) * s;
                                l.transform.y += (ev.position.y / px(1.0) - ly) * s;
                            }
                            // Clamp into the canvas, then dirty the union of
                            // old+new layer bbox; the composite waits for
                            // render().
                            if let Some((w0, h0)) = layer_size_of(&this.doc, &mut this.renderer, layer) {
                                let (docw, doch) = (this.doc.width as f32, this.doc.height as f32);
                                let mut scale = 1.0;
                                let mut newxy = (oldx, oldy);
                                if let Some(l) = this.doc.layers.get_mut(layer) {
                                    scale = l.transform.scale.max(0.01);
                                    let (w, h) = (w0 * scale, h0 * scale);
                                    l.transform.x = l.transform.x
                                        .clamp((docw - w).min(0.0), (docw - w).max(0.0));
                                    l.transform.y = l.transform.y
                                        .clamp((doch - h).min(0.0), (doch - h).max(0.0));
                                    newxy = (l.transform.x, l.transform.y);
                                }
                                let (newx, newy) = newxy;
                                let (w, h) = (w0 * scale, h0 * scale);
                                rect = (
                                    oldx.min(newx) - 2.0,
                                    oldy.min(newy) - 2.0,
                                    oldx.max(newx) + w + 2.0,
                                    oldy.max(newy) + h + 2.0,
                                );
                                region_ok = true;
                            }
                            if region_ok {
                                this.invalidate_region(rect.0, rect.1, rect.2, rect.3);
                            } else {
                                this.invalidate();
                            }
                            this.drag = Some((layer, ev.position.x / px(1.0), ev.position.y / px(1.0)));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| {
                        this.drag = None;
                        this.pan_drag = None;
                    }))
                    // Wayland/X11 file drops (gpui turns them into a drag
                    // session carrying ExternalPaths).
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                        this.snapshot();
                        for p in paths.paths() {
                            match load_path(&mut this.doc, &p.to_string_lossy()) {
                                Ok(()) => {
                                    this.selected = Some(this.doc.layers.len() - 1);
                                }
                                Err(e) => eprintln!("chonk: {e}"),
                            }
                        }
                        this.invalidate();
                        cx.notify();
                    }))
                    .child(canvas(
                        |bounds, _window, _cx| bounds,
                        {
                            let view = cx.entity().clone();
                            let fh = self.focus_handle.clone();
                            let editing = self.edit_mode;
                            let border = th.border;
                            let accent = th.accent;
                            let caret = th.caret;
                            let busy = self.busy;
                            let caret_geom = caret_geom;
                            move |bounds, _bounds, window, cx| {
                                // Route typed characters (IME included) to the
                                // view's EntityInputHandler while focused.
                                window.handle_input(
                                    &fh,
                                    ElementInputHandler::new(bounds, view.clone()),
                                    cx,
                                );
                                let side = side_px;
                                let origin = origin_px;
                            if let Some(img) = &image {
                                let target = Bounds { origin, size: size(side, side) };
                                window
                                    .paint_image(target, target, Corners::default(), img.clone(), 0, false)
                                    .ok();
                            }
                            // Frame around the canvas.
                            let bt = px(1.0);
                            let bc = rgb(border);
                            window.paint_quad(fill(Bounds { origin: point(origin.x - bt, origin.y - bt), size: size(side + bt + bt, bt) }, bc));
                            window.paint_quad(fill(Bounds { origin: point(origin.x - bt, origin.y + side), size: size(side + bt + bt, bt) }, bc));
                            window.paint_quad(fill(Bounds { origin: point(origin.x - bt, origin.y), size: size(bt, side) }, bc));
                            window.paint_quad(fill(Bounds { origin: point(origin.x + side, origin.y), size: size(bt, side) }, bc));
                            // Busy badge: accent square in the canvas' top-right
                            // while the worker encodes an export.
                            if busy {
                                let s = px(10.0);
                                window.paint_quad(fill(
                                    Bounds { origin: point(origin.x + side - s - px(6.0), origin.y + px(6.0)), size: size(s, s) },
                                    rgb(accent),
                                ));
                            }
                            // Selection outline: four 2px quads in canvas space.
                            if let Some((rx, ry, rw, rh)) = sel_rect {
                                let k = side / px(1080.0); // doc px -> window px (f32 ratio)
                                let x0 = origin.x + px(rx * k);
                                let y0 = origin.y + px(ry * k);
                                let w = px(rw * k);
                                let h = px(rh * k);
                                let t = px(2.0);
                                let c = rgb(accent);
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0 - t), size: size(w + t + t, t) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0 + h), size: size(w + t + t, t) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0), size: size(t, h) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 + w, y0), size: size(t, h) }, c));
                                // Caret while editing: at the caret offset
                                // (click/arrow positioned), else text end.
                                if editing {
                                    match caret_geom {
                                        Some((gx, gy, gh)) => {
                                            window.paint_quad(fill(
                                                Bounds {
                                                    origin: point(origin.x + px(gx * k), origin.y + px(gy * k)),
                                                    size: size(px(3.0), px(gh * k) * 0.85),
                                                },
                                                rgb(caret),
                                            ));
                                        }
                                        None => {
                                            window.paint_quad(fill(
                                                Bounds { origin: point(x0 + w + px(4.0), y0 + h * 0.15), size: size(px(3.0), h * 0.7) },
                                                rgb(caret),
                                            ));
                                        }
                                    }
                                }
                            }
                            }
                        },
                    ).size_full()),
            )
            .child(self.layer_panel(th, cx)),
            )
            .child(
                // Status bar: feedback lives here, never in the window title.
                div()
                    .flex_none()
                    .w_full()
                    .h(px(22.0))
                    .px_2()
                    .bg(rgb(th.panel))
                    .text_size(px(11.5))
                    .text_color(rgb(if self.status.is_some() { th.text } else { th.dim }))
                    .child(
                        self.status.clone().unwrap_or_else(|| {
                            "Q quote · T text · F deep-fry · ,/. blend · [ ] opacity · S slot · Ctrl+C/V copy·paste · Ctrl+Z undo · drag files in".into()
                        }),
                    ),
            )
    }
}

impl MemeView {
    /// Right-hand layer list: topmost layer first. Click row = select;
    /// eye = visibility; ▲/▼ = reorder; ✕ = delete.
    fn layer_panel(&self, th: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .id("layer-panel")
            .flex_none()
            .w(px(PANEL_W))
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(th.panel))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(rgb(th.dim))
                    .child("LAYERS (top first)"),
            );
        for i in (0..self.doc.layers.len()).rev() {
            let l = &self.doc.layers[i];
            let selected = self.selected == Some(i);
            let row = div()
                .id(SharedString::from(format!("lyr{i}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .text_size(px(13.0))
                .text_color(if l.visible { rgb(th.text) } else { rgb(th.dim) })
                .bg(if selected { rgb(th.sel) } else { rgb(th.panel) })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected = Some(i);
                    this.edit_mode = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id(SharedString::from(format!("eye{i}")))
                        .cursor_pointer()
                        .text_color(rgb(if l.visible { th.text } else { th.dim }))
                        .child(if l.visible { "◉" } else { "◎" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.snapshot();
                            if let Some(l) = this.doc.layers.get_mut(i) {
                                l.visible = !l.visible;
                                this.invalidate();
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_1().child(layer_name(l)))
                .child(
                    // Blend mode (, / . cycles the selected layer).
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(if l.blend == chonk_core::BlendMode::Normal { th.dim } else { th.accent }))
                        .child(l.blend.short_name()),
                )
                .child(
                    // Slot tag: this layer is a fillable template slot.
                    div()
                        .id(SharedString::from(format!("slot{i}")))
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .text_color(rgb(if l.slot.is_some() { th.accent } else { th.border }))
                        .child(l.slot.as_ref().map(|s| format!("[{}]", s.name)).unwrap_or_else(|| "[ ]".into()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected = Some(i);
                            this.toggle_slot();
                            cx.notify();
                        })),
                )
                .child(
                    // Opacity bar (keyboard [ / ] adjusts the selected layer).
                    div()
                        .w(px(40.0))
                        .h(px(4.0))
                        .bg(rgb(th.border))
                        .child(
                            div()
                                .w(px(40.0 * l.opacity))
                                .h(px(4.0))
                                .bg(rgb(th.accent)),
                        ),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("up{i}")))
                        .cursor_pointer()
                        .text_color(rgb(th.dim))
                        .child("▲")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.move_layer(i, 1);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("dn{i}")))
                        .cursor_pointer()
                        .text_color(rgb(th.dim))
                        .child("▼")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.move_layer(i, -1);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("del{i}")))
                        .cursor_pointer()
                        .text_color(rgb(th.danger))
                        .child("✕")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if i < this.doc.layers.len() {
                                this.snapshot();
                                this.doc.layers.remove(i);
                                this.selected = None;
                                this.edit_mode = false;
                                this.invalidate();
                            }
                            cx.notify();
                        })),
                );
            panel = panel.child(row);
        }
        // Palette section: extract from the composite, click a swatch to
        // recolor the selected caption.
        panel = panel
            .child(
                div()
                    .id("extract-palette")
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(rgb(th.accent))
                    .child("EXTRACT PALETTE")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(pm) = &this.last_pixmap {
                            this.palette = chonk_core::palette::extract(pm, 8);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_1()
                    .px_2()
                    .pb_2()
                    .children(self.palette.iter().enumerate().map(|(idx, c)| {
                        let c8 = c.to_color_u8();
                        let hex =
                            ((c8.red() as u32) << 16) | ((c8.green() as u32) << 8) | c8.blue() as u32;
                        div()
                            .id(SharedString::from(format!("swatch{idx}")))
                            .cursor_pointer()
                            .w(px(20.0))
                            .h(px(20.0))
                            .bg(rgb(hex))
                            .child(div().w(px(20.)).h(px(20.)).bg(rgb(hex)))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.apply_swatch(idx);
                                cx.notify();
                            }))
                    })),
            );
        panel
    }

    /// Recolor the selected caption with palette swatch `idx`.
    fn apply_swatch(&mut self, idx: usize) {
        let Some(c) = self.palette.get(idx).copied() else {
            return;
        };
        let Some(i) = self.selected else { return };
        self.snapshot();
        if let Some(chonk_core::LayerKind::Text { style, .. }) =
            self.doc.layers.get_mut(i).map(|l| &mut l.kind)
        {
            style.fill = c;
            self.invalidate();
        }
    }

    /// Nudge the selected layer's opacity by `delta` (clamped 0.05..1).
    fn adjust_opacity(&mut self, delta: f32) {
        if let Some(i) = self.selected {
            self.snapshot_tagged("opacity");
            if let Some(l) = self.doc.layers.get_mut(i) {
                l.opacity = (l.opacity + delta).clamp(0.05, 1.0);
                self.invalidate();
            }
        }
    }

    /// Deep-fry the selected image layer on a background thread.
    /// Each `F` walks the ladder (mild -> medium -> heavy -> ultra);
    /// the snapshot happens up front so one press = one undo step.
    fn fry_selected(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        let targets: Vec<sk::Pixmap> = match &self.doc.layers[i].kind {
            chonk_core::LayerKind::Image { pixmap } => vec![pixmap.clone()],
            chonk_core::LayerKind::AnimatedImage { image } => image.frames.clone(),
            _ => {
                self.status = Some("fry needs an image layer".into());
                return;
            }
        };
        self.snapshot();
        // Ladder position lives on the layer (undo restores it with the
        // pixels); bump it up front so rapid presses walk the ladder.
        let rung = (self.doc.layers[i].fry_depth as usize)
            .min(chonk_core::fry::FRY_LADDER.len() - 1);
        self.doc.layers[i].fry_depth = self.doc.layers[i].fry_depth.saturating_add(1);
        let spec = chonk_core::fry::FRY_LADDER[rung];
        let name = chonk_core::fry::FRY_NAMES[rung];
        self.busy = true;
        self.status = Some(format!("deep-frying: {name}"));
        let me = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let fried = cx
                .background_executor()
                .spawn(async move {
                    targets.iter().map(|pm| chonk_core::fry::fry(pm, &spec)).collect::<Vec<_>>()
                })
                .await;
            let _ = me.update(cx, |v, cx| {
                v.busy = false;
                if let Some(l) = v.doc.layers.get_mut(i) {
                    match &mut l.kind {
                        chonk_core::LayerKind::Image { pixmap } => {
                            if let Some(f) = fried.into_iter().next() {
                                *pixmap = f;
                            }
                        }
                        chonk_core::LayerKind::AnimatedImage { image } => {
                            image.frames = fried;
                        }
                        _ => {}
                    }
                    v.invalidate();
                    v.status = Some(format!("deep-fried: {name}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Mark/unmark the selected layer as a template slot (S).
    fn toggle_slot(&mut self) {
        let Some(i) = self.selected else { return };
        // SVG slots have no fill path yet — don't touch history at all.
        match self.doc.layers.get(i).map(|l| &l.kind) {
            Some(chonk_core::LayerKind::Svg { .. }) | None => return,
            _ => {}
        }
        self.snapshot();
        // First free slotN name (computed before the mutable borrow below).
        let used: Vec<String> = self
            .doc
            .layers
            .iter()
            .filter_map(|x| x.slot.as_ref().map(|s| s.name.clone()))
            .collect();
        if let Some(l) = self.doc.layers.get_mut(i) {
            if l.slot.is_some() {
                l.slot = None;
            } else {
                let kind = match &l.kind {
                    chonk_core::LayerKind::Text { .. } => chonk_core::SlotKind::Text,
                    chonk_core::LayerKind::Image { .. }
                    | chonk_core::LayerKind::AnimatedImage { .. } => chonk_core::SlotKind::Image,
                    chonk_core::LayerKind::Svg { .. } => return,
                };
                let mut n = 1;
                let name = loop {
                    let cand = format!("slot{n}");
                    if !used.contains(&cand) {
                        break cand;
                    }
                    n += 1;
                };
                l.slot = Some(chonk_core::Slot { name, kind });
            }
            self.invalidate();
        }
    }

    /// Cycle the selected layer's blend mode (dir = +1 / -1 through CYCLE).
    fn cycle_blend(&mut self, dir: i32) {
        if let Some(i) = self.selected {
            self.snapshot_tagged("blend");
            if let Some(l) = self.doc.layers.get_mut(i) {
                let pos = chonk_core::BlendMode::CYCLE
                    .iter()
                    .position(|b| *b == l.blend)
                    .unwrap_or(0);
                let n = chonk_core::BlendMode::CYCLE.len() as i64;
                let next = (((pos as i64 + dir as i64).rem_euclid(n))) as usize;
                l.blend = chonk_core::BlendMode::CYCLE[next];
                self.status = Some(format!("blend: {}", l.blend.short_name()));
                self.invalidate();
            }
        }
    }

    /// Swap a layer with its neighbor (dir=1 moves it up the stack).
    fn move_layer(&mut self, i: usize, dir: i32) {
        self.snapshot();
        let n = self.doc.layers.len();
        let j = if dir > 0 { i + 1 } else { match i.checked_sub(1) { Some(j) => j, None => return } };
        if j >= n {
            return;
        }
        self.doc.layers.swap(i, j);
        self.selected = Some(j);
        self.invalidate();
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(900.0)), cx);
        let view = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Chonk".into());
                    cx.new(MemeView::new)
                },
            )
            .unwrap();
        // Optional file argument: chonk-app image.png|photo.jpg|anim.gif|art.svg
        if let Some(path) = std::env::args().nth(1) {
            let label = path.clone();
            view.update(cx, |v, _window, _cx| {
                match load_path(&mut v.doc, &path) {
                    Ok(()) => {
                        v.selected = Some(v.doc.layers.len() - 1);
                        v.recomposite();
                    }
                    Err(e) => eprintln!("chonk: {e}"),
                }
            }).ok();
            view.update(cx, |_v, window, _cx| {
                window.set_window_title(format!("Chonk — {label}").as_str());
            }).ok();
        }
        cx.bind_keys([
            KeyBinding::new("q", ToggleQuote, Some(CHONK_CTX)),
            KeyBinding::new("t", AddText, Some(CHONK_CTX)),
            KeyBinding::new("delete", DeleteLayer, Some(CHONK_CTX)),
            KeyBinding::new("backspace", DeleteLayer, Some(CHONK_CTX)),
            KeyBinding::new("up", NudgeUp, Some(CHONK_CTX)),
            KeyBinding::new("down", NudgeDown, Some(CHONK_CTX)),
            KeyBinding::new("left", NudgeLeft, Some(CHONK_CTX)),
            KeyBinding::new("right", NudgeRight, Some(CHONK_CTX)),
            KeyBinding::new("space", TogglePlay, Some(CHONK_CTX)),
            KeyBinding::new("enter", EditCaption, Some(CHONK_CTX)),
            KeyBinding::new("e", ExportPng, Some(CHONK_CTX)),
            KeyBinding::new("j", ExportJpg, Some(CHONK_CTX)),
            KeyBinding::new("w", ExportWebp, Some(CHONK_CTX)),
            KeyBinding::new("g", ExportGif, Some(CHONK_CTX)),
            KeyBinding::new("0", ResetView, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-o", OpenFile, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-s", SaveAs, Some(CHONK_CTX)),
            KeyBinding::new("tab", CycleUp, Some(CHONK_CTX)),
            KeyBinding::new("shift-tab", CycleDown, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-t", ToggleTheme, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-z", Undo, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-y", Redo, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-shift-z", Redo, Some(CHONK_CTX)),
            KeyBinding::new("]", OpacityUp, Some(CHONK_CTX)),
            KeyBinding::new("[", OpacityDown, Some(CHONK_CTX)),
            KeyBinding::new(".", BlendNext, Some(CHONK_CTX)),
            KeyBinding::new(",", BlendPrev, Some(CHONK_CTX)),
            KeyBinding::new("s", ToggleSlot, Some(CHONK_CTX)),
            KeyBinding::new("f", FryLayer, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-c", CopyLayer, Some(CHONK_CTX)),
            KeyBinding::new("ctrl-v", Paste, Some(CHONK_CTX)),
            KeyBinding::new("escape", CancelPending, Some(CHONK_CTX)),
            KeyBinding::new("=", ZoomIn, Some(CHONK_CTX)),
            KeyBinding::new("-", ZoomOut, Some(CHONK_CTX)),
            // While editing, printable keys must NOT match any binding — they
            // flow to the EntityInputHandler instead.
            KeyBinding::new("enter", FinishEdit, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("escape", FinishEdit, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("backspace", DeleteChar, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("left", CaretLeft, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("right", CaretRight, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("ctrl-v", PasteText, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("ctrl-z", UndoText, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("ctrl-y", RedoText, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("ctrl-shift-z", RedoText, Some(CHONK_EDIT_CTX)),
        ]);
        // Focus the view so its key context is live from the first frame.
        view.update(cx, |v, window, cx| window.focus(&v.focus_handle, cx)).ok();
        cx.activate(true);
    });
}
