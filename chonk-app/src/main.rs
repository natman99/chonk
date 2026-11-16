//! Chonk — a meme/quote-card editor. GPUI chrome, chonk-core engine.

use chonk_core::document::{Background, Document, LayerKind, Shadow, TextAlign, TextStyle};
use chonk_core::quote::{QuoteSpec, quote_document};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;
use gpui::{
    App, AsyncApp, Bounds, Context, Corners, ExternalPaths, FocusHandle, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, RenderImage, Window, WindowBounds, WindowOptions,
    actions, canvas, div, fill, point, prelude::*, px, rgb, size,
};
use gpui_platform::application;
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
        TogglePlay
    ]
);

/// Key context this view installs while focused (Shale's pattern: bindings
/// are scoped to a focused, tracked view — not dispatched globally).
const CHONK_CTX: &str = "chonk";

struct MemeView {
    doc: Document,
    /// UI-thread renderer, used ONLY for layer sizes (hit-testing/outline);
    /// all rasterization happens on the render worker.
    renderer: Renderer,
    composited: Option<Arc<RenderImage>>,
    job_tx: std::sync::mpsc::Sender<RenderJob>,
    result_rx: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<(u64, sk::Pixmap)>>>,
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
    quote_mode: bool,
    playing: bool,
    frame: usize,
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
}

/// Worker thread owning the Renderer (and its caches) plus the last full
/// canvas; region jobs patch its copy. The UI thread never rasterizes.
fn spawn_render_worker() -> (
    std::sync::mpsc::Sender<RenderJob>,
    std::sync::mpsc::Receiver<(u64, sk::Pixmap)>,
) {
    let (jt, jr) = std::sync::mpsc::channel::<RenderJob>();
    let (rt, rr) = std::sync::mpsc::channel::<(u64, sk::Pixmap)>();
    std::thread::spawn(move || {
        let mut renderer = Renderer::new();
        let mut canvas: Option<sk::Pixmap> = None;
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
            }
            if job.seq <= last_seq {
                continue;
            }
            last_seq = job.seq;
            let pm = match (job.region, canvas.as_ref()) {
                (Some((x0, y0, x1, y1)), Some(base)) => {
                    let x = x0.max(0.0).floor() as u32;
                    let y = y0.max(0.0).floor() as u32;
                    let w = (x1.floor().max(x as f32 + 1.0)) as u32 - x;
                    let h = (y1.floor().max(y as f32 + 1.0)) as u32 - y;
                    let mut c = base.clone();
                    let region = renderer.render_region_at(&job.doc, x, y, w, h, job.frame);
                    Renderer::overlay_region(&mut c, &region, x, y);
                    c
                }
                _ => renderer.composite_at(&job.doc, job.frame),
            };
            canvas = Some(pm.clone());
            if rt.send((job.seq, pm)).is_err() {
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
            let rt = renderer.text.render(text, style)?;
            (rt.pixmap.width() as f32, rt.pixmap.height() as f32)
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
            quote_mode: false,
            playing: false,
            frame: 0,
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
                    Ok((seq, pm)) => {
                        let _ = this.update(cx, |v, cx| v.apply_frame(seq, pm, cx));
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
                self.recomposite();
            }
        }
    }

    /// Untransformed layer size in document pixels (text uses the render
    /// cache, so this is cheap for unchanged layers).
    fn layer_size(&mut self, i: usize) -> Option<(f32, f32)> {
        layer_size_of(&self.doc, &mut self.renderer, i)
    }

    /// Topmost visible layer whose (scaled) box contains the document point.
    fn hit_test(&mut self, dx: f32, dy: f32) -> Option<usize> {
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
                return Some(i);
            }
        }
        None
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

    fn recomposite(&mut self) {
        self.invalidate();
    }
}

/// chonk-core premultiplied-RGBA pixmap -> gpui RenderImage.
/// (gpui-pre 0.3.10 consumes RGBA directly — do NOT byte-swap to BGRA;
/// the doc comment on `RenderImage` is stale for this fork.)
fn render_image_from_pixmap(pm: &sk::Pixmap) -> RenderImage {
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec())
        .expect("buffer sized to pixmap");
    let frame = image::Frame::new(img);
    RenderImage::new(smallvec::smallvec![frame])
}

impl Render for MemeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Send at most one composite job per frame to the render worker;
        // rasterization never happens on the UI thread.
        if self.dirty && !self.in_flight {
            let frame = if self.playing { self.frame } else { 0 };
            self.seq += 1;
            let job = RenderJob {
                seq: self.seq,
                doc: self.doc.clone(),
                frame,
                region: self.dirty_region.take(),
            };
            if self.job_tx.send(job).is_ok() {
                self.in_flight = true;
                self.dirty = false;
            }
        }
        let image = self.composited.clone();
        let b = _window.bounds();
        // Canvas element sits inside p_3 padding (12px); the image is a
        // centered square within it. Mirror the paint closure's math so
        // mouse->doc mapping and hit-testing line up with what is drawn.
        let ew = b.size.width / px(1.0) - 24.0;
        let eh = b.size.height / px(1.0) - 24.0;
        let side = ew.min(eh).max(1.0);
        self.view_side = side;
        self.view_origin = (12.0 + (ew - side) / 2.0, 12.0 + (eh - side) / 2.0);

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
            .key_context(CHONK_CTX)
            .track_focus(&self.focus_handle)
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
            .bg(rgb(0x101014))
            .p_3()
            .child(
                div()
                    .id("canvas")
                    .size_full()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus_handle, cx);
                        let (dx, dy) = this.to_doc(ev.position.x / px(1.0), ev.position.y / px(1.0));
                        this.selected = this.hit_test(dx, dy);
                        if let Some(layer) = this.selected {
                            this.drag = Some((layer, ev.position.x / px(1.0), ev.position.y / px(1.0)));
                        }
                        cx.notify();
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        if let Some((layer, lx, ly)) = this.drag.take() {
                            let mut region_ok = false;
                            let mut rect = (0.0, 0.0, 0.0, 0.0);
                            if let Some(l) = this.doc.layers.get_mut(layer) {
                                let s = this.doc.width as f32 / this.view_side.max(1.0);
                                let scale = l.transform.scale.max(0.01);
                                let (oldx, oldy) = (l.transform.x, l.transform.y);
                                l.transform.x += (ev.position.x / px(1.0) - lx) * s;
                                l.transform.y += (ev.position.y / px(1.0) - ly) * s;
                                let (newx, newy) = (l.transform.x, l.transform.y);
                                // Dirty region = union of old+new layer bbox;
                                // the composite itself waits for render().
                                if let Some((w0, h0)) = layer_size_of(&this.doc, &mut this.renderer, layer) {
                                    let (w, h) = (w0 * scale, h0 * scale);
                                    rect = (
                                        oldx.min(newx) - 2.0,
                                        oldy.min(newy) - 2.0,
                                        oldx.max(newx) + w + 2.0,
                                        oldy.max(newy) + h + 2.0,
                                    );
                                    region_ok = true;
                                }
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
                        move |bounds, _bounds, window, _cx| {
                            let side = bounds.size.width.min(bounds.size.height);
                            let origin = point(
                                bounds.origin.x + (bounds.size.width - side) / 2.0,
                                bounds.origin.y + (bounds.size.height - side) / 2.0,
                            );
                            if let Some(img) = &image {
                                let target = Bounds { origin, size: size(side, side) };
                                window
                                    .paint_image(target, target, Corners::default(), img.clone(), 0, false)
                                    .ok();
                            }
                            // Selection outline: four 2px quads in canvas space.
                            if let Some((rx, ry, rw, rh)) = sel_rect {
                                let k = side / px(1080.0); // doc px -> window px (f32 ratio)
                                let x0 = origin.x + px(rx * k);
                                let y0 = origin.y + px(ry * k);
                                let w = px(rw * k);
                                let h = px(rh * k);
                                let t = px(2.0);
                                let c = rgb(0x22d3ee);
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0 - t), size: size(w + t + t, t) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0 + h), size: size(w + t + t, t) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 - t, y0), size: size(t, h) }, c));
                                window.paint_quad(fill(Bounds { origin: point(x0 + w, y0), size: size(t, h) }, c));
                            }
                        },
                    ).size_full()),
            )
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
                        "Chonk — Q quote · T text · arrows nudge · Del delete · click select · space play"
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
            });
            view.update(cx, |_v, window, _cx| {
                window.set_window_title(format!("Chonk — {label}").as_str());
            });
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
        ]);
        // Focus the view so its key context is live from the first frame.
        view.update(cx, |v, window, cx| window.focus(&v.focus_handle, cx)).ok();
        cx.activate(true);
    });
}
