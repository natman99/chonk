use tiny_skia as sk;

/// Placement of a layer on the canvas: rotate, then scale, then translate
/// (scale/rotation are about the layer's own origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotation_deg: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, scale: 1.0, rotation_deg: 0.0 }
    }
}

impl Transform {
    pub fn to_skia(&self) -> sk::Transform {
        let t = sk::Transform::from_translate(self.x, self.y);
        let t = t.pre_scale(self.scale, self.scale);
        if self.rotation_deg != 0.0 {
            t.pre_rotate(self.rotation_deg.to_radians())
        } else {
            t
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    Transparent,
    Solid(sk::Color),
    /// Linear gradient in canvas pixel coordinates (quote-card backgrounds).
    LinearGradient { x1: f32, y1: f32, x2: f32, y2: f32, stops: Vec<(f32, sk::Color)> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    /// Blur radius in pixels (0 = hard offset shadow).
    pub blur: f32,
    pub color: sk::Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: f32,
    pub fill: sk::Color,
    /// (color, stroke width)
    pub stroke: Option<(sk::Color, f32)>,
    pub shadow: Option<Shadow>,
    pub align: TextAlign,
    /// Wrap box width; `None` disables wrapping.
    pub max_width: Option<f32>,
    /// Auto-fit: shrink the font size until the text fits this box (quote cards).
    pub fit_box: Option<(f32, f32)>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 32.0,
            fill: sk::Color::from_rgba8(0, 0, 0, 255),
            stroke: None,
            shadow: None,
            align: TextAlign::Center,
            max_width: None,
            fit_box: None,
        }
    }
}

/// Animated raster source (GIF): premultiplied frames + per-frame delays.
#[derive(Clone, Debug)]
pub struct AnimatedImage {
    pub frames: Vec<sk::Pixmap>,
    pub delays_ms: Vec<u32>,
}

impl AnimatedImage {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frame(&self, index: usize) -> &sk::Pixmap {
        &self.frames[index % self.frames.len()]
    }
}

#[derive(Clone, Debug)]
pub enum LayerKind {
    Image { pixmap: sk::Pixmap },
    AnimatedImage { image: AnimatedImage },
    Svg { tree: usvg::Tree },
    Text { text: String, style: TextStyle },
}

/// Porter-Duff separable blend modes for layer compositing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub fn to_skia(self) -> sk::BlendMode {
        match self {
            BlendMode::Normal => sk::BlendMode::SourceOver,
            BlendMode::Multiply => sk::BlendMode::Multiply,
            BlendMode::Screen => sk::BlendMode::Screen,
            BlendMode::Overlay => sk::BlendMode::Overlay,
            BlendMode::Darken => sk::BlendMode::Darken,
            BlendMode::Lighten => sk::BlendMode::Lighten,
            BlendMode::ColorDodge => sk::BlendMode::ColorDodge,
            BlendMode::ColorBurn => sk::BlendMode::ColorBurn,
            BlendMode::HardLight => sk::BlendMode::HardLight,
            BlendMode::SoftLight => sk::BlendMode::SoftLight,
            BlendMode::Difference => sk::BlendMode::Difference,
            BlendMode::Exclusion => sk::BlendMode::Exclusion,
            BlendMode::Hue => sk::BlendMode::Hue,
            BlendMode::Saturation => sk::BlendMode::Saturation,
            BlendMode::Color => sk::BlendMode::Color,
            BlendMode::Luminosity => sk::BlendMode::Luminosity,
        }
    }

    /// Cycle order for the `,` / `.` keys.
    pub const CYCLE: [BlendMode; 16] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    pub fn short_name(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "dodge",
            BlendMode::ColorBurn => "burn",
            BlendMode::HardLight => "hardlight",
            BlendMode::SoftLight => "softlight",
            BlendMode::Difference => "diff",
            BlendMode::Exclusion => "excl",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "sat",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "lum",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub kind: LayerKind,
    pub transform: Transform,
    pub opacity: f32,
    pub visible: bool,
    pub blend: BlendMode,
    /// Template slot: fillable by name (see template.rs). A document
    /// with slots is a template; filling one produces a meme.
    pub slot: Option<Slot>,
}

/// A named, fillable slot in a template document.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub name: String,
    pub kind: SlotKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotKind {
    /// Fill with a string (replaces the caption text).
    Text,
    /// Fill with an image (scaled to the slot's current display box).
    Image,
}

impl Layer {
    pub fn new(kind: LayerKind) -> Self {
        Self { kind, transform: Transform::default(), opacity: 1.0, visible: true, blend: BlendMode::Normal, slot: None }
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub background: Background,
    pub layers: Vec<Layer>,
}

impl Document {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            background: Background::Transparent,
            layers: Vec::new(),
        }
    }

    pub fn add_layer(&mut self, kind: LayerKind) -> &mut Layer {
        self.layers.push(Layer::new(kind));
        self.layers.last_mut().unwrap()
    }

    pub fn add_image(&mut self, pixmap: sk::Pixmap) -> &mut Layer {
        self.add_layer(LayerKind::Image { pixmap })
    }

    pub fn add_animated(&mut self, image: AnimatedImage) -> &mut Layer {
        self.add_layer(LayerKind::AnimatedImage { image })
    }

    pub fn add_svg(&mut self, tree: usvg::Tree) -> &mut Layer {
        self.add_layer(LayerKind::Svg { tree })
    }

    pub fn add_text(&mut self, text: impl Into<String>, style: TextStyle) -> &mut Layer {
        self.add_layer(LayerKind::Text { text: text.into(), style })
    }
}
