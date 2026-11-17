//! chonk-core: the headless meme/quote-card engine.
//!
//! No GUI dependency: documents are composited to `tiny_skia::Pixmap`s, which
//! any front-end (GPUI window, CLI, clipboard) can consume.

pub mod codecs;
pub mod document;
pub mod filters;
pub mod quote;
pub mod render;
pub mod save;
pub mod svg;
pub mod text;

pub use document::{
    Background, BlendMode, Document, Layer, LayerKind, Shadow, TextAlign, TextStyle, Transform,
};
pub use quote::{QuoteBackground, QuoteSpec, quote_document};
pub use render::Renderer;

pub use tiny_skia;
