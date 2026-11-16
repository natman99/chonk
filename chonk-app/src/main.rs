//! Chonk — a meme/quote-card editor. GPUI chrome, chonk-core engine.

use chonk_core::document::{Background, Document, LayerKind, Shadow, TextAlign, TextStyle};
use chonk_core::quote::{QuoteSpec, quote_document};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;
use gpui::{
    App, AsyncApp, Bounds, ClickEvent, Context, Corners, ElementInputHandler, EntityInputHandler,
    ExternalPaths, FocusHandle, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, RenderImage, ScrollDelta, ScrollEvent, SharedString, UTF16Selection, Window,
    WindowBounds, WindowOptions, actions, canvas, div, fill, point, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

actions!(
    chonk,
    [
        ToggleQuote,
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
        ToggleTheme
    ]
);

/// Key context this view installs while focused (Shale's pattern: bindings
/// are scoped to a focused, tracked view — not dispatched globally).
const CHONK_CTX: &str = "chonk";
/// Swapped in while editing a caption: printable keys must reach the input
/// handler instead of firing app shortcuts.
const CHONK_EDIT_CTX: &str = "chonk-edit";

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
    focus_handle: FocusHandle,
}

/// Load an image/gif/svg file into the document as a new top layer.
fn load_path(doc: &mut Document, path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
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

    fn add_text(&mut self) {
        self.doc.add_text(
            "new caption",
            TextStyle {
                font_family: "Noto Sans".into(),
                font_size: 72.0,
                fill: sk::Color::from_rgba8(255, 255, 255, 255),
                stroke: Some((sk::Color::from_rgba8(0, 0, 0, 255), 6.0)),
                align: TextAlign::Center,
                max_width: Some(900.0),
                ..TextStyle::default()
            },
        );
        self.selected = Some(self.doc.layers.len() - 1);
        self.recomposite();
    }

    fn delete_layer(&mut self) {
        if let Some(i) = self.selected {
            if i < self.doc.layers.len() {
                self.doc.layers.remove(i);
                self.selected = None;
                self.recomposite();
            }
        }
    }

    fn nudge(&mut self, dx: f32, dy: f32) {
        if let Some(i) = self.selected {
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
        if self.selected_text().is_some() {
            self.edit_mode = true;
            window.set_window_title("Chonk — EDITING caption (type · Enter finishes)".into());
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
        window.set_window_title("Chonk — Q quote · T text · Enter edit · scroll zoom · drag empty pan · 0 reset".into());
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
        window.set_window_title(format!("Chonk — exporting {path}…").as_str());
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
                        .set_title("Chonk — open image")
                        .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "svg", "bmp"])
                        .pick_file()
                })
                .await;
            let Some(p) = picked else { return };
            let _ = me.update(cx, |v, cx| {
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
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let picked = cx.background_executor()
                .spawn(async {
                    rfd::FileDialog::new()
                        .set_title("Chonk — save as")
                        .set_file_name("chonk.png")
                        .add_filter("PNG", &["png"])
                        .add_filter("JPEG", &["jpg", "jpeg"])
                        .add_filter("WebP", &["webp"])
                        .add_filter("GIF (animated)", &["gif"])
                        .save_file()
                })
                .await;
            let Some(p) = picked else { return };
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
            if let Some(LayerKind::Text { text, .. }) = self.doc.layers.get_mut(i).map(|l| &mut l.kind) {
                if let Some(pos) = text.chars().next_back().map(|c| text.len() - c.len_utf8()) {
                    text.remove(pos);
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
        if let Some(i) = self.selected {
            if let Some(LayerKind::Text { text: t, .. }) =
                self.doc.layers.get_mut(i).map(|l| &mut l.kind)
            {
                match range {
                    Some(r) => {
                        let start = r.start.min(t.len());
                        let end = r.end.max(start).min(t.len());
                        t.replace_range(start..end, text);
                    }
                    None => t.push_str(text),
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
            let eh = _window.bounds().size.height / px(1.0) - 24.0;
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
        if let Some(msg) = self.status.take() {
            _window.set_window_title(format!("Chonk — {msg}").as_str());
        }
        // Canvas element sits inside p_3 padding (12px) and shares the row
        // with the layer panel (PANEL_W + GAP). The image is a centered
        // square within the remaining area. Mirror the paint closure's math
        // so mouse->doc mapping and hit-testing line up with what is drawn.
        let ew = b.size.width / px(1.0) - 24.0 - GAP - PANEL_W;
        let eh = b.size.height / px(1.0) - 24.0;
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
            .on_action(cx.listener(|this, _: &ExportPng, window, _cx| this.export("png", window)))
            .on_action(cx.listener(|this, _: &ExportJpg, window, _cx| this.export("jpg", window)))
            .on_action(cx.listener(|this, _: &ExportWebp, window, _cx| this.export("webp", window)))
            .on_action(cx.listener(|this, _: &ExportGif, window, _cx| this.export("gif", window)))
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
            .on_action(cx.listener(|this, _: &ToggleQuote, _window, cx| {
                this.toggle_quote();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &AddText, _window, cx| {
                this.add_text();
                cx.notify();
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
                    window.set_window_title(if this.playing {
                        "Chonk — playing (space to pause)".into()
                    } else {
                        "Chonk — paused".into()
                    });
                    cx.notify();
                }
            }))
            .size_full()
            .bg(rgb(th.bg))
            .p_3()
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
                        if new_sel != this.selected {
                            this.selected = new_sel;
                            this.edit_mode = false; // selection change ends editing
                        }
                        if let Some(layer) = this.selected {
                            this.drag = Some((layer, ev.position.x / px(1.0), ev.position.y / px(1.0)));
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
                                // Caret at text end while editing.
                                if editing {
                                    window.paint_quad(fill(
                                        Bounds { origin: point(x0 + w + px(4.0), y0 + h * 0.15), size: size(px(3.0), h * 0.7) },
                                        rgb(caret),
                                    ));
                                }
                            }
                            }
                        },
                    ).size_full()),
            )
            .child(self.layer_panel(th, cx))
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
                            if let Some(l) = this.doc.layers.get_mut(i) {
                                l.visible = !l.visible;
                                this.invalidate();
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_1().child(layer_name(l)))
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
        panel
    }

    /// Swap a layer with its neighbor (dir=1 moves it up the stack).
    fn move_layer(&mut self, i: usize, dir: i32) {
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
                    window.set_window_title(
                        "Chonk — Q quote · T text · Enter edit · scroll zoom · drag empty pan · E/J/W/G export · space play"
                            .into(),
                    );
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
            KeyBinding::new("=", ZoomIn, Some(CHONK_CTX)),
            KeyBinding::new("-", ZoomOut, Some(CHONK_CTX)),
            // While editing, printable keys must NOT match any binding — they
            // flow to the EntityInputHandler instead.
            KeyBinding::new("enter", FinishEdit, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("escape", FinishEdit, Some(CHONK_EDIT_CTX)),
            KeyBinding::new("backspace", DeleteChar, Some(CHONK_EDIT_CTX)),
        ]);
        // Focus the view so its key context is live from the first frame.
        view.update(cx, |v, window, cx| window.focus(&v.focus_handle, cx)).ok();
        cx.activate(true);
    });
}
