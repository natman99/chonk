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

#[derive(Clone, Debug)]
pub enum LayerKind {
    Image { pixmap: sk::Pixmap },
    Svg { tree: usvg::Tree },
    Text { text: String, style: TextStyle },
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub kind: LayerKind,
    pub transform: Transform,
    pub opacity: f32,
    pub visible: bool,
}

impl Layer {
    pub fn new(kind: LayerKind) -> Self {
        Self { kind, transform: Transform::default(), opacity: 1.0, visible: true }
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

    pub fn add_svg(&mut self, tree: usvg::Tree) -> &mut Layer {
        self.add_layer(LayerKind::Svg { tree })
    }

    pub fn add_text(&mut self, text: impl Into<String>, style: TextStyle) -> &mut Layer {
        self.add_layer(LayerKind::Text { text: text.into(), style })
    }
}
