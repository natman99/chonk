//! Chonk — a meme/quote-card editor. GPUI chrome, chonk-core engine.

use chonk_core::document::{Background, Document, Shadow, TextAlign, TextStyle};
use chonk_core::quote::{QuoteSpec, quote_document};
use chonk_core::render::Renderer;
use chonk_core::tiny_skia as sk;
use gpui::{
    App, Bounds, Context, Corners, KeyBinding, MouseDownEvent, MouseButton, MouseMoveEvent,
    MouseUpEvent, RenderImage, Window, WindowBounds, WindowOptions, actions, canvas, div, point,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;
use std::sync::Arc;

actions!(chonk, [ToggleQuote]);

struct MemeView {
    doc: Document,
    renderer: Renderer,
    composited: Option<Arc<RenderImage>>,
    /// (layer index, last mouse x, last mouse y) in window pixels.
    drag: Option<(usize, f32, f32)>,
    /// Size of the square canvas viewport in window pixels (set each render).
    view_side: f32,
    quote_mode: bool,
}

impl MemeView {
    fn new() -> Self {
        let mut v = Self {
            doc: Self::meme_doc(),
            renderer: Renderer::new(),
            composited: None,
            drag: None,
            view_side: 0.0,
            quote_mode: false,
        };
        v.recomposite();
        v
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
            let spec = QuoteSpec::new(
                "It is not enough to succeed; others must fail.",
                "Caligula",
            );
            quote_document(&mut self.renderer.text, &spec)
        } else {
            Self::meme_doc()
        };
        self.drag = None;
        self.recomposite();
    }

    fn recomposite(&mut self) {
        let pm = self.renderer.composite(&self.doc);
        self.composited = Some(Arc::new(render_image_from_pixmap(&pm)));
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
        let image = self.composited.clone();
        let b = _window.bounds();
        let side = (b.size.width / px(1.0) - 48.0)
            .min(b.size.height / px(1.0) - 48.0)
            .max(1.0);
        self.view_side = side;

        div()
            .size_full()
            .bg(rgb(0x101014))
            .p_3()
            .child(
                div()
                    .id("canvas")
                    .size_full()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, _| {
                        this.drag = Some((0, ev.position.x / px(1.0), ev.position.y / px(1.0)));
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        if let Some((layer, lx, ly)) = this.drag.take() {
                            if let Some(l) = this.doc.layers.get_mut(layer) {
                                let s = this.doc.width as f32 / this.view_side.max(1.0);
                                l.transform.x += (ev.position.x / px(1.0) - lx) * s;
                                l.transform.y += (ev.position.y / px(1.0) - ly) * s;
                                this.recomposite();
                            }
                            this.drag = Some((layer, ev.position.x / px(1.0), ev.position.y / px(1.0)));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| {
                        this.drag = None;
                    }))
                    .child(canvas(
                        |bounds, _window, _cx| bounds,
                        move |bounds, _bounds, window, _cx| {
                            if let Some(img) = &image {
                                let side = bounds.size.width.min(bounds.size.height);
                                let origin = point(
                                    bounds.origin.x + (bounds.size.width - side) / 2.0,
                                    bounds.origin.y + (bounds.size.height - side) / 2.0,
                                );
                                let target = Bounds { origin, size: size(side, side) };
                                window
                                    .paint_image(
                                        target,
                                        target,
                                        Corners::default(),
                                        img.clone(),
                                        0,
                                        false,
                                    )
                                    .ok();
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
                    window.set_window_title("Chonk — press Q for quote card".into());
                    cx.new(|_cx| MemeView::new())
                },
            )
            .unwrap();
        cx.on_action({
            let view = view.clone();
            move |_: &ToggleQuote, cx: &mut App| {
                view.update(cx, |v, _window, cx| {
                    v.toggle_quote();
                    eprintln!(
                        "[chonk] ToggleQuote fired; quote_mode={}",
                        v.quote_mode
                    );
                    cx.notify();
                });
            }
        });
        cx.bind_keys([KeyBinding::new("q", ToggleQuote, None)]);
        // TEMP DEBUG: log every keystroke GPUI receives.
        cx.observe_keystrokes(|ev, _window, _cx| {
            eprintln!(
                "[chonk] keystroke key={:?} key_char={:?}",
                ev.keystroke.key, ev.keystroke.key_char
            );
        });
        cx.activate(true);
    });
}
