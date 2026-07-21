pub mod ffi;
mod scenario_browser;

use lc_graphics::{Color, TextFont};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

pub use scenario_browser::{
    ScenarioBrowser, ScenarioBrowserMessage, ScenarioBrowserResponse, ScenarioEntry,
    ScenarioEntrySummary, ScenarioKind,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    fn clamp(self, constraints: LayoutConstraints) -> Self {
        let width = self.width.min(constraints.max_width).max(0.0);
        let height = self.height.min(constraints.max_height).max(0.0);
        Size { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.y >= self.origin.y
            && point.x < self.origin.x + self.size.width
            && point.y < self.origin.y + self.size.height
    }

    pub fn inset(&self, amount: f32) -> Self {
        self.inset_by(amount, amount)
    }

    pub fn inset_by(&self, horizontal: f32, vertical: f32) -> Self {
        let horizontal = horizontal.max(0.0);
        let vertical = vertical.max(0.0);
        let width = (self.size.width - horizontal * 2.0).max(0.0);
        let height = (self.size.height - vertical * 2.0).max(0.0);
        Rect::from_origin_size(
            Point::new(self.origin.x + horizontal, self.origin.y + vertical),
            Size::new(width, height),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutConstraints {
    pub max_width: f32,
    pub max_height: f32,
}

impl LayoutConstraints {
    pub fn new(max_width: f32, max_height: f32) -> Self {
        Self {
            max_width: max_width.max(0.0),
            max_height: max_height.max(0.0),
        }
    }

    pub fn tight(size: Size) -> Self {
        Self::new(size.width, size.height)
    }

    pub fn unbounded() -> Self {
        Self::new(f32::INFINITY, f32::INFINITY)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Enter,
    Escape,
    Space,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, Debug)]
pub enum GuiEvent {
    PointerDown { position: Point },
    PointerUp { position: Point },
    PointerMove { position: Point },
    KeyDown { key: KeyCode },
    KeyUp { key: KeyCode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuiAction {
    Activate,
}

#[derive(Debug, Default)]
pub struct GuiEventResult {
    pub captured: bool,
    pub actions: Vec<(WidgetId, GuiAction)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(usize);

impl WidgetId {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn to_raw(self) -> usize {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }

    pub(crate) const fn from_raw(index: usize) -> Self {
        Self(index)
    }
}

impl fmt::Display for WidgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiError {
    InvalidWidget(WidgetId),
    WrongWidgetType {
        id: WidgetId,
        expected: &'static str,
        found: &'static str,
    },
}

impl fmt::Display for GuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuiError::InvalidWidget(id) => write!(f, "invalid widget id {}", id),
            GuiError::WrongWidgetType {
                id,
                expected,
                found,
            } => write!(
                f,
                "widget {} expected kind {} but found {}",
                id, expected, found
            ),
        }
    }
}

impl std::error::Error for GuiError {}

pub type GuiResult<T> = Result<T, GuiError>;

#[derive(Debug, Clone)]
struct Column {
    padding: f32,
    spacing: f32,
    expand_width: bool,
}

impl Column {
    fn new(padding: f32, spacing: f32, expand_width: bool) -> Self {
        Self {
            padding,
            spacing,
            expand_width,
        }
    }
}

impl Default for Column {
    fn default() -> Self {
        Column::new(8.0, 6.0, false)
    }
}

#[derive(Debug, Clone)]
struct Row {
    padding: f32,
    spacing: f32,
    expand_height: bool,
}

impl Row {
    fn new(padding: f32, spacing: f32, expand_height: bool) -> Self {
        Self {
            padding,
            spacing,
            expand_height,
        }
    }
}

impl Default for Row {
    fn default() -> Self {
        Row::new(8.0, 6.0, false)
    }
}

#[derive(Debug, Clone)]
struct Label {
    text: String,
    color: Color,
    font_size: f32,
    padding: f32,
}

impl Label {
    fn new(text: String) -> Self {
        Self {
            text,
            color: Color::opaque(235, 235, 235),
            font_size: 16.0,
            padding: 4.0,
        }
    }

    fn intrinsic_size(&self, font: &dyn TextFont) -> Size {
        let metrics = font.measure_text(&self.text, self.font_size.max(1.0));
        Size::new(
            metrics.width + self.padding * 2.0,
            metrics.height + self.padding * 2.0,
        )
    }
}

#[derive(Debug, Clone)]
struct Button {
    text: String,
    font_size: f32,
    padding: f32,
    min_width: f32,
    normal_color: Color,
    pressed_color: Color,
    selected_color: Color,
    disabled_color: Color,
    text_color: Color,
    disabled_text_color: Color,
    pressed: bool,
    selected: bool,
    enabled: bool,
}

impl Button {
    fn new(text: String) -> Self {
        Self {
            text,
            font_size: 16.0,
            padding: 6.0,
            min_width: 96.0,
            normal_color: Color::opaque(48, 96, 160),
            pressed_color: Color::opaque(28, 68, 120),
            selected_color: Color::opaque(72, 128, 196),
            disabled_color: Color::opaque(36, 44, 60),
            text_color: Color::opaque(255, 255, 255),
            disabled_text_color: Color::opaque(180, 188, 200),
            pressed: false,
            selected: false,
            enabled: true,
        }
    }

    fn intrinsic_size(&self, font: &dyn TextFont) -> Size {
        let metrics = font.measure_text(&self.text, self.font_size.max(1.0));
        let width = (metrics.width + self.padding * 2.0).max(self.min_width);
        let height = metrics.height + self.padding * 2.0;
        Size::new(width, height)
    }

    fn current_color(&self) -> Color {
        if !self.enabled {
            self.disabled_color
        } else if self.pressed {
            self.pressed_color
        } else if self.selected {
            self.selected_color
        } else {
            self.normal_color
        }
    }

    fn current_text_color(&self) -> Color {
        if self.enabled {
            self.text_color
        } else {
            self.disabled_text_color
        }
    }
}

#[derive(Debug)]
struct Gauge {
    fraction: f32,
    min_width: f32,
    height: f32,
    background_color: Color,
    high_color: Color,
    low_color: Color,
}

impl Gauge {
    fn new() -> Self {
        Self {
            fraction: 1.0,
            min_width: 120.0,
            height: 18.0,
            background_color: Color::opaque(28, 36, 52),
            high_color: Color::opaque(96, 176, 88),
            low_color: Color::opaque(208, 72, 56),
        }
    }

    fn set_size(&mut self, min_width: f32, height: f32) {
        if min_width.is_finite() && min_width > 0.0 {
            self.min_width = min_width;
        }
        if height.is_finite() && height > 0.0 {
            self.height = height;
        }
    }

    fn intrinsic_size(&self) -> Size {
        Size::new(self.min_width, self.height)
    }

    fn effective_fraction(&self) -> f32 {
        self.fraction.clamp(0.0, 1.0)
    }

    fn set_fraction(&mut self, value: f32) {
        if value.is_finite() {
            self.fraction = value;
        } else if value.is_sign_negative() {
            self.fraction = 0.0;
        } else {
            self.fraction = 1.0;
        }
    }

    fn fill_color(&self) -> Color {
        let t = self.effective_fraction();
        let blend = |start: u8, end: u8| -> u8 {
            let start = start as f32;
            let end = end as f32;
            (start + (end - start) * t).round().clamp(0.0, 255.0) as u8
        };
        Color::new(
            blend(self.low_color.r, self.high_color.r),
            blend(self.low_color.g, self.high_color.g),
            blend(self.low_color.b, self.high_color.b),
            self.high_color.a,
        )
    }
}

#[derive(Debug, Clone)]
struct Picture {
    preferred_size: Size,
    image: Option<ImageData>,
    background: Color,
    frame_color: Color,
    padding: f32,
}

impl Picture {
    fn new(width: f32, height: f32) -> Self {
        Self {
            preferred_size: Size::new(width.max(1.0), height.max(1.0)),
            image: None,
            background: Color::opaque(24, 36, 58),
            frame_color: Color::opaque(12, 20, 32),
            padding: 6.0,
        }
    }

    fn intrinsic_size(&self) -> Size {
        self.preferred_size
    }
}

#[derive(Debug)]
struct WidgetNode {
    id: WidgetId,
    #[allow(dead_code)]
    parent: Option<WidgetId>,
    children: Vec<WidgetId>,
    kind: WidgetKind,
    rect: Rect,
}

#[derive(Debug)]
enum WidgetKind {
    Column(Column),
    Row(Row),
    Label(Label),
    Button(Button),
    Gauge(Gauge),
    Picture(Picture),
}

impl WidgetKind {
    fn name(&self) -> &'static str {
        match self {
            WidgetKind::Column(_) => "column",
            WidgetKind::Row(_) => "row",
            WidgetKind::Label(_) => "label",
            WidgetKind::Button(_) => "button",
            WidgetKind::Gauge(_) => "gauge",
            WidgetKind::Picture(_) => "picture",
        }
    }
}

impl WidgetNode {
    fn new(id: WidgetId, parent: Option<WidgetId>, kind: WidgetKind) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            kind,
            rect: Rect::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    Quad {
        rect: Rect,
        color: Color,
    },
    Text {
        rect: Rect,
        text: String,
        color: Color,
        font_size: f32,
        padding: f32,
    },
    Image {
        rect: Rect,
        image: ImageData,
    },
}

#[derive(Clone, Debug)]
pub struct ImageData {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
    gpu_texture_id: lc_graphics::GpuTextureId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ImageContentKey {
    width: u32,
    height: u32,
    byte_len: usize,
    hash: u64,
}

struct InternedImageData {
    pixels: Arc<[u8]>,
    gpu_texture_id: lc_graphics::GpuTextureId,
    last_used: u64,
}

#[derive(Default)]
struct ImageDataInterner {
    entries: HashMap<ImageContentKey, Vec<InternedImageData>>,
    entry_count: usize,
    retained_bytes: usize,
    clock: u64,
}

const IMAGE_DATA_INTERNER_MAX_ENTRIES: usize = 16_384;
const IMAGE_DATA_INTERNER_MAX_BYTES: usize = 128 * 1024 * 1024;
const IMAGE_DATA_INTERNER_MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;

fn image_content_hash(pixels: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    pixels.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn intern_image_data(
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
) -> (Arc<[u8]>, lc_graphics::GpuTextureId) {
    // One unusually large runtime image must not evict the reusable UI and
    // definition working set. It still receives a valid process-local ID.
    if pixels.len() > IMAGE_DATA_INTERNER_MAX_ENTRY_BYTES {
        return (pixels, lc_graphics::GpuTextureId::fresh());
    }
    let key = ImageContentKey {
        width,
        height,
        byte_len: pixels.len(),
        hash: image_content_hash(&pixels),
    };
    static INTERNER: OnceLock<Mutex<ImageDataInterner>> = OnceLock::new();
    let mut interner = INTERNER
        .get_or_init(|| Mutex::new(ImageDataInterner::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    interner.clock = interner.clock.wrapping_add(1).max(1);
    let last_used = interner.clock;
    if let Some(entry) = interner
        .entries
        .get_mut(&key)
        .and_then(|entries| entries.iter_mut().find(|entry| entry.pixels == pixels))
    {
        entry.last_used = last_used;
        return (Arc::clone(&entry.pixels), entry.gpu_texture_id);
    }

    let gpu_texture_id = lc_graphics::GpuTextureId::fresh();
    interner
        .entries
        .entry(key)
        .or_default()
        .push(InternedImageData {
            pixels: Arc::clone(&pixels),
            gpu_texture_id,
            last_used,
        });
    interner.entry_count += 1;
    interner.retained_bytes = interner.retained_bytes.saturating_add(pixels.len());

    while interner.entry_count > IMAGE_DATA_INTERNER_MAX_ENTRIES
        || interner.retained_bytes > IMAGE_DATA_INTERNER_MAX_BYTES
    {
        let oldest = interner
            .entries
            .iter()
            .flat_map(|(key, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (*key, index, entry.last_used))
            })
            .min_by_key(|(_, _, last_used)| *last_used);
        let Some((oldest_key, oldest_index, _)) = oldest else {
            break;
        };
        let (removed_bytes, remove_bucket) = {
            let entries = interner
                .entries
                .get_mut(&oldest_key)
                .expect("oldest interned image bucket remains present");
            let removed = entries.swap_remove(oldest_index);
            (removed.pixels.len(), entries.is_empty())
        };
        if remove_bucket {
            interner.entries.remove(&oldest_key);
        }
        interner.entry_count -= 1;
        interner.retained_bytes = interner.retained_bytes.saturating_sub(removed_bytes);
    }

    (pixels, gpu_texture_id)
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels == other.pixels
    }
}

impl ImageData {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let (pixels, gpu_texture_id) =
            intern_image_data(width, height, Arc::from(pixels.into_boxed_slice()));
        Self {
            width,
            height,
            pixels,
            gpu_texture_id,
        }
    }

    pub fn from_arc(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        let (pixels, gpu_texture_id) = intern_image_data(width, height, pixels);
        Self {
            width,
            height,
            pixels,
            gpu_texture_id,
        }
    }

    /// Creates a short-lived image view without retaining it in the immutable
    /// content interner. Use this only when another retained producer supplies
    /// the real GPU resource identity and this wrapper exists for geometry or
    /// CPU replay: interning the producer's `Arc` would keep it shared and
    /// force its next mutable update to fork through copy-on-write.
    pub fn transient_from_arc(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            width,
            height,
            pixels,
            gpu_texture_id: lc_graphics::GpuTextureId::fresh(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixels_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn gpu_texture_id(&self) -> lc_graphics::GpuTextureId {
        self.gpu_texture_id
    }

    pub fn gpu_texture_resource(&self) -> lc_graphics::GpuTextureResource {
        lc_graphics::GpuTextureResource::immutable_rgba(
            self.gpu_texture_id,
            self.width,
            self.height,
            Arc::clone(&self.pixels),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonTextures {
    pub normal: ImageData,
    pub pressed: ImageData,
    pub selected: ImageData,
    pub disabled: Option<ImageData>,
}

pub struct Gui {
    nodes: Vec<WidgetNode>,
    root: WidgetId,
    next_id: usize,
    pressed_button: Option<WidgetId>,
    font: Arc<dyn TextFont>,
    button_textures: Option<ButtonTextures>,
}

impl Gui {
    pub fn new(font: Arc<dyn TextFont>) -> Self {
        let root_id = WidgetId::new(0);
        let root_node = WidgetNode::new(
            root_id,
            None,
            WidgetKind::Column(Column::new(8.0, 6.0, true)),
        );
        Self {
            nodes: vec![root_node],
            root: root_id,
            next_id: 1,
            pressed_button: None,
            font,
            button_textures: None,
        }
    }

    pub fn root(&self) -> WidgetId {
        self.root
    }

    pub fn add_column(&mut self, parent: WidgetId, expand_width: bool) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Column(Column::new(8.0, 6.0, expand_width)),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn add_row(&mut self, parent: WidgetId, expand_height: bool) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Row(Row::new(8.0, 6.0, expand_height)),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn add_label(&mut self, parent: WidgetId, text: impl Into<String>) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(id, Some(parent), WidgetKind::Label(Label::new(text.into())));
        self.attach_child(parent, node);
        id
    }

    pub fn add_button(&mut self, parent: WidgetId, text: impl Into<String>) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Button(Button::new(text.into())),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn add_gauge(&mut self, parent: WidgetId) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(id, Some(parent), WidgetKind::Gauge(Gauge::new()));
        self.attach_child(parent, node);
        id
    }

    pub fn add_picture(&mut self, parent: WidgetId, width: f32, height: f32) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Picture(Picture::new(width, height)),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn set_label_text(&mut self, id: WidgetId, text: impl Into<String>) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Label(label) => {
                label.text = text.into();
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "label", kind)),
        }
    }

    pub fn set_label_color(&mut self, id: WidgetId, color: Color) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Label(label) => {
                label.color = color;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "label", kind)),
        }
    }

    pub fn set_button_text(&mut self, id: WidgetId, text: impl Into<String>) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.text = text.into();
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn set_button_selected(&mut self, id: WidgetId, selected: bool) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.selected = selected;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn set_button_enabled(&mut self, id: WidgetId, enabled: bool) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.enabled = enabled;
                if !enabled {
                    button.pressed = false;
                }
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn set_button_textures(&mut self, textures: Option<ButtonTextures>) {
        self.button_textures = textures;
    }

    pub fn set_gauge_fraction(&mut self, id: WidgetId, value: f32) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Gauge(gauge) => {
                gauge.set_fraction(value);
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "gauge", kind)),
        }
    }

    pub fn set_gauge_size(&mut self, id: WidgetId, width: f32, height: f32) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Gauge(gauge) => {
                gauge.set_size(width, height);
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "gauge", kind)),
        }
    }

    pub fn set_picture_image(&mut self, id: WidgetId, image: Option<ImageData>) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Picture(picture) => {
                picture.image = image;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "picture", kind)),
        }
    }

    pub fn set_picture_frame_color(&mut self, id: WidgetId, color: Color) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Picture(picture) => {
                picture.frame_color = color;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "picture", kind)),
        }
    }

    pub fn set_picture_background_color(&mut self, id: WidgetId, color: Color) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Picture(picture) => {
                picture.background = color;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "picture", kind)),
        }
    }

    pub fn layout(&mut self, available: Size) -> Size {
        let constraints = LayoutConstraints::tight(available);
        self.layout_with_constraints(constraints)
    }

    pub fn layout_with_constraints(&mut self, constraints: LayoutConstraints) -> Size {
        self.layout_node(self.root, constraints, Point::new(0.0, 0.0))
    }

    pub fn render(&self) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        self.render_node(self.root, &mut commands);
        commands
    }

    pub fn handle_event(&mut self, event: GuiEvent) -> GuiEventResult {
        match event {
            GuiEvent::PointerDown { position } => self.handle_pointer_down(position),
            GuiEvent::PointerUp { position } => self.handle_pointer_up(position),
            GuiEvent::PointerMove { position } => self.handle_pointer_move(position),
            GuiEvent::KeyDown { .. } | GuiEvent::KeyUp { .. } => GuiEventResult::default(),
        }
    }

    pub fn cancel_interaction(&mut self) {
        if let Some(active) = self.pressed_button.take() {
            if let Some(node) = self.nodes.get_mut(active.index()) {
                if let WidgetKind::Button(button) = &mut node.kind {
                    button.pressed = false;
                }
            }
        }
    }

    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.nodes.get(id.index()).map(|node| node.rect)
    }

    fn widget_mut(&mut self, id: WidgetId) -> GuiResult<&mut WidgetNode> {
        if id.index() >= self.nodes.len() {
            return Err(GuiError::InvalidWidget(id));
        }
        Ok(&mut self.nodes[id.index()])
    }

    fn handle_pointer_down(&mut self, position: Point) -> GuiEventResult {
        if let Some(id) = self.hit_test(self.root, position) {
            if let WidgetKind::Button(button) = &mut self.nodes[id.index()].kind {
                if button.enabled {
                    button.pressed = true;
                    self.pressed_button = Some(id);
                    return GuiEventResult {
                        captured: true,
                        actions: Vec::new(),
                    };
                }
            }
        }
        GuiEventResult::default()
    }

    fn handle_pointer_up(&mut self, position: Point) -> GuiEventResult {
        let mut result = GuiEventResult::default();
        if let Some(active) = self.pressed_button.take() {
            let inside = self
                .nodes
                .get(active.index())
                .map(|node| node.rect.contains(position))
                .unwrap_or(false);
            if let WidgetKind::Button(button) = &mut self.nodes[active.index()].kind {
                button.pressed = false;
                if button.enabled && inside {
                    result.captured = true;
                    result.actions.push((active, GuiAction::Activate));
                }
            }
        }
        result
    }

    fn handle_pointer_move(&mut self, position: Point) -> GuiEventResult {
        if let Some(active) = self.pressed_button {
            let rect = self.nodes[active.index()].rect;
            if let WidgetKind::Button(button) = &mut self.nodes[active.index()].kind {
                if button.enabled {
                    button.pressed = rect.contains(position);
                    return GuiEventResult {
                        captured: true,
                        actions: Vec::new(),
                    };
                }
            }
        }
        GuiEventResult::default()
    }

    fn render_node(&self, id: WidgetId, commands: &mut Vec<DrawCommand>) {
        let node = &self.nodes[id.index()];
        match &node.kind {
            WidgetKind::Button(button) => {
                let mut drew_background = false;
                if let Some(textures) = &self.button_textures {
                    let image = if !button.enabled {
                        textures
                            .disabled
                            .as_ref()
                            .unwrap_or(&textures.normal)
                            .clone()
                    } else if button.pressed {
                        textures.pressed.clone()
                    } else if button.selected {
                        textures.selected.clone()
                    } else {
                        textures.normal.clone()
                    };
                    commands.push(DrawCommand::Image {
                        rect: node.rect,
                        image,
                    });
                    drew_background = true;
                }
                if !drew_background {
                    commands.push(DrawCommand::Quad {
                        rect: node.rect,
                        color: button.current_color(),
                    });
                }
                commands.push(DrawCommand::Text {
                    rect: node.rect,
                    text: button.text.clone(),
                    color: button.current_text_color(),
                    font_size: button.font_size,
                    padding: button.padding,
                });
            }
            WidgetKind::Label(label) => {
                commands.push(DrawCommand::Text {
                    rect: node.rect,
                    text: label.text.clone(),
                    color: label.color,
                    font_size: label.font_size,
                    padding: label.padding,
                });
            }
            WidgetKind::Gauge(gauge) => {
                commands.push(DrawCommand::Quad {
                    rect: node.rect,
                    color: gauge.background_color,
                });
                let fraction = gauge.effective_fraction();
                if fraction > 0.0 {
                    let width = node.rect.size.width * fraction;
                    if width > 0.0 {
                        let fill_rect = Rect::from_origin_size(
                            node.rect.origin,
                            Size::new(width, node.rect.size.height),
                        );
                        commands.push(DrawCommand::Quad {
                            rect: fill_rect,
                            color: gauge.fill_color(),
                        });
                    }
                }
            }
            WidgetKind::Picture(picture) => {
                commands.push(DrawCommand::Quad {
                    rect: node.rect,
                    color: picture.frame_color,
                });
                let content_rect = node.rect.inset(1.0);
                commands.push(DrawCommand::Quad {
                    rect: content_rect,
                    color: picture.background,
                });
                if let Some(image) = &picture.image {
                    let image_bounds = content_rect.inset(picture.padding);
                    if let Some(letterboxed) = letterbox_image_rect(image_bounds, image) {
                        commands.push(DrawCommand::Image {
                            rect: letterboxed,
                            image: image.clone(),
                        });
                    }
                }
            }
            WidgetKind::Column(_) | WidgetKind::Row(_) => {}
        }

        for child in &node.children {
            self.render_node(*child, commands);
        }
    }

    fn layout_node(&mut self, id: WidgetId, constraints: LayoutConstraints, origin: Point) -> Size {
        match self.nodes[id.index()].kind {
            WidgetKind::Label(_) => self.layout_label(id, constraints, origin),
            WidgetKind::Button(_) => self.layout_button(id, constraints, origin),
            WidgetKind::Row(_) => self.layout_row(id, constraints, origin),
            WidgetKind::Gauge(_) => self.layout_gauge(id, constraints, origin),
            WidgetKind::Picture(_) => self.layout_picture(id, constraints, origin),
            WidgetKind::Column(_) => self.layout_column(id, constraints, origin),
        }
    }

    fn layout_label(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let label = match &self.nodes[id.index()].kind {
                WidgetKind::Label(label) => label,
                _ => unreachable!(),
            };
            label.intrinsic_size(self.font.as_ref())
        };
        let size = intrinsic.clamp(constraints);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_button(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let button = match &self.nodes[id.index()].kind {
                WidgetKind::Button(button) => button,
                _ => unreachable!(),
            };
            button.intrinsic_size(self.font.as_ref())
        };
        let size = intrinsic.clamp(constraints);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_gauge(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let gauge = match &self.nodes[id.index()].kind {
                WidgetKind::Gauge(gauge) => gauge,
                _ => unreachable!(),
            };
            gauge.intrinsic_size()
        };
        let size = intrinsic.clamp(constraints);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_picture(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let picture = match &self.nodes[id.index()].kind {
                WidgetKind::Picture(picture) => picture,
                _ => unreachable!(),
            };
            picture.intrinsic_size()
        };
        let size = Size::new(
            intrinsic.width.min(constraints.max_width),
            intrinsic.height.min(constraints.max_height),
        );
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_row(&mut self, id: WidgetId, constraints: LayoutConstraints, origin: Point) -> Size {
        let (padding, spacing, expand_height) = {
            let row = match &self.nodes[id.index()].kind {
                WidgetKind::Row(row) => row,
                _ => unreachable!(),
            };
            (row.padding, row.spacing, row.expand_height)
        };

        let children = self.nodes[id.index()].children.clone();
        let mut x = origin.x + padding;
        let mut total_width: f32 = 0.0;
        let mut max_height: f32 = 0.0;
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                x += spacing;
                total_width += spacing;
            }
            let child_constraints = LayoutConstraints::new(
                constraints.max_width,
                if constraints.max_height.is_finite() {
                    (constraints.max_height - padding * 2.0).max(0.0)
                } else {
                    f32::INFINITY
                },
            );
            let child_origin = Point::new(x, origin.y + padding);
            let child_size = self.layout_node(*child, child_constraints, child_origin);
            x += child_size.width;
            total_width += child_size.width;
            max_height = max_height.max(child_size.height);
        }

        let content_width = if children.is_empty() {
            0.0
        } else {
            total_width
        };
        let mut width = padding * 2.0 + content_width;
        if constraints.max_width.is_finite() {
            width = width.min(constraints.max_width);
        }
        let mut height = padding * 2.0 + max_height;
        if expand_height && constraints.max_height.is_finite() {
            height = constraints.max_height;
        } else if constraints.max_height.is_finite() {
            height = height.min(constraints.max_height);
        }
        let size = Size::new(width, height);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_column(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let (padding, spacing, expand_width) = {
            let column = match &self.nodes[id.index()].kind {
                WidgetKind::Column(column) => column,
                _ => unreachable!(),
            };
            (column.padding, column.spacing, column.expand_width)
        };

        let children = self.nodes[id.index()].children.clone();
        let mut y = origin.y + padding;
        let mut max_width: f32 = 0.0;
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                y += spacing;
            }
            let child_constraints = LayoutConstraints::new(
                if constraints.max_width.is_finite() {
                    (constraints.max_width - padding * 2.0).max(0.0)
                } else {
                    f32::INFINITY
                },
                constraints.max_height,
            );
            let child_origin = Point::new(origin.x + padding, y);
            let child_size = self.layout_node(*child, child_constraints, child_origin);
            y += child_size.height;
            max_width = max_width.max(child_size.width);
        }

        let content_height = if children.is_empty() {
            0.0
        } else {
            y - (origin.y + padding)
        };
        let mut width = max_width + padding * 2.0;
        if expand_width && constraints.max_width.is_finite() {
            width = constraints.max_width;
        } else if constraints.max_width.is_finite() {
            width = width.min(constraints.max_width);
        }
        let mut height = padding * 2.0 + content_height;
        if constraints.max_height.is_finite() {
            height = height.min(constraints.max_height);
        }
        let size = Size::new(width, height);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn hit_test(&self, id: WidgetId, point: Point) -> Option<WidgetId> {
        let node = &self.nodes[id.index()];
        if !node.rect.contains(point) {
            return None;
        }
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test(*child, point) {
                return Some(hit);
            }
        }
        Some(id)
    }

    fn attach_child(&mut self, parent: WidgetId, node: WidgetNode) {
        let index = node.id.index();
        if self.nodes.len() != index {
            panic!(
                "widget id mismatch: expected {} got {}",
                self.nodes.len(),
                index
            );
        }
        self.nodes.push(node);
        self.nodes[parent.index()]
            .children
            .push(WidgetId::new(index));
    }

    fn alloc_id(&mut self) -> WidgetId {
        let id = WidgetId::new(self.next_id);
        self.next_id += 1;
        id
    }
}

fn wrong_widget_type(id: WidgetId, expected: &'static str, kind: &WidgetKind) -> GuiError {
    GuiError::WrongWidgetType {
        id,
        expected,
        found: kind.name(),
    }
}

fn letterbox_image_rect(bounds: Rect, image: &ImageData) -> Option<Rect> {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }

    let scale_x = bounds.size.width / width as f32;
    let scale_y = bounds.size.height / height as f32;
    let scale = scale_x.min(scale_y);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }

    let scaled_width = (width as f32 * scale).min(bounds.size.width).max(0.0);
    let scaled_height = (height as f32 * scale).min(bounds.size.height).max(0.0);
    let target_width = scaled_width.max(1.0).min(bounds.size.width);
    let target_height = scaled_height.max(1.0).min(bounds.size.height);

    let offset_x = (bounds.size.width - target_width) * 0.5;
    let offset_y = (bounds.size.height - target_height) * 0.5;
    Some(Rect::from_origin_size(
        Point::new(bounds.origin.x + offset_x, bounds.origin.y + offset_y),
        Size::new(target_width, target_height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_graphics::BitmapFont;

    fn center(rect: Rect) -> Point {
        Point::new(
            rect.origin.x + rect.size.width * 0.5,
            rect.origin.y + rect.size.height * 0.5,
        )
    }

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    #[test]
    fn identical_immutable_images_reuse_retained_gpu_identity() {
        let pixels = (1..=16).collect::<Vec<u8>>();
        let first = ImageData::new(2, 2, pixels.clone());
        let second = ImageData::from_arc(2, 2, Arc::from(pixels));

        assert_eq!(first, second);
        assert_eq!(first.gpu_texture_id(), second.gpu_texture_id());
        assert!(Arc::ptr_eq(&first.pixels, &second.pixels));
    }

    #[test]
    fn retained_gpu_identity_includes_dimensions_and_collision_checked_bytes() {
        let baseline = ImageData::new(2, 1, vec![10, 20, 30, 40, 50, 60, 70, 80]);
        let different_dimensions = ImageData::new(1, 2, vec![10, 20, 30, 40, 50, 60, 70, 80]);
        let different_pixels = ImageData::new(2, 1, vec![10, 20, 30, 40, 50, 60, 70, 81]);

        assert_ne!(
            baseline.gpu_texture_id(),
            different_dimensions.gpu_texture_id()
        );
        assert_ne!(baseline.gpu_texture_id(), different_pixels.gpu_texture_id());
    }

    #[test]
    fn transient_image_view_does_not_pin_or_intern_mutable_backing() {
        let pixels: Arc<[u8]> = Arc::from([10, 20, 30, 255]);
        assert_eq!(Arc::strong_count(&pixels), 1);
        let first = ImageData::transient_from_arc(1, 1, Arc::clone(&pixels));
        let first_id = first.gpu_texture_id();
        assert_eq!(Arc::strong_count(&pixels), 2);
        drop(first);
        assert_eq!(Arc::strong_count(&pixels), 1);

        let second = ImageData::transient_from_arc(1, 1, Arc::clone(&pixels));
        assert_ne!(second.gpu_texture_id(), first_id);
        drop(second);
        assert_eq!(Arc::strong_count(&pixels), 1);
    }

    #[test]
    fn column_layout_places_children_vertically() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let label = gui.add_label(root, "Ready?");
        let button = gui.add_button(root, "Start Game");

        gui.layout(Size::new(300.0, 200.0));

        let label_rect = gui.rect_of(label).unwrap();
        let button_rect = gui.rect_of(button).unwrap();

        assert!(button_rect.origin.y > label_rect.origin.y + label_rect.size.height);
        assert!((label_rect.origin.x - button_rect.origin.x).abs() < f32::EPSILON);
    }

    #[test]
    fn button_click_generates_activate_action() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let button = gui.add_button(root, "Launch");

        gui.layout(Size::new(200.0, 120.0));
        let button_rect = gui.rect_of(button).unwrap();
        let inside = center(button_rect);

        let down = gui.handle_event(GuiEvent::PointerDown { position: inside });
        assert!(down.captured);
        if let WidgetKind::Button(button_state) = &gui.nodes[button.index()].kind {
            assert!(button_state.pressed);
        }

        let up = gui.handle_event(GuiEvent::PointerUp { position: inside });
        assert!(up.captured);
        assert_eq!(up.actions.len(), 1);
        assert_eq!(up.actions[0], (button, GuiAction::Activate));
    }

    #[test]
    fn button_release_outside_cancels_action() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let button = gui.add_button(root, "Abort");

        gui.layout(Size::new(200.0, 120.0));
        let button_rect = gui.rect_of(button).unwrap();
        let inside = center(button_rect);
        let outside = Point::new(button_rect.origin.x - 10.0, button_rect.origin.y - 10.0);

        gui.handle_event(GuiEvent::PointerDown { position: inside });
        let up = gui.handle_event(GuiEvent::PointerUp { position: outside });
        assert!(!up.captured);
        assert!(up.actions.is_empty());
    }

    #[test]
    fn render_emits_draw_commands() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        gui.add_label(root, "Headline");
        gui.add_button(root, "Continue");

        gui.layout(Size::new(320.0, 180.0));
        let commands = gui.render();
        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, DrawCommand::Quad { .. })));
        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, DrawCommand::Text { text, .. } if text == "Continue")));
    }

    #[test]
    fn gauge_renders_background_and_fill() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let gauge = gui.add_gauge(root);

        gui.set_gauge_fraction(gauge, 1.5).expect("gauge exists");
        gui.layout(Size::new(160.0, 64.0));

        let mut quads: Vec<(Rect, Color)> = gui
            .render()
            .into_iter()
            .filter_map(|cmd| match cmd {
                DrawCommand::Quad { rect, color } => Some((rect, color)),
                _ => None,
            })
            .collect();

        assert_eq!(quads.len(), 2, "gauge renders background and fill");
        let fill = quads.pop().unwrap();
        let background = quads.pop().unwrap();
        assert_eq!(background.0.origin, fill.0.origin);
        assert_eq!(background.0.size.height, fill.0.size.height);
        assert_eq!(background.0.size.width, fill.0.size.width);
        assert_ne!(
            background.1, fill.1,
            "fill uses distinct color to indicate energy"
        );
    }

    #[test]
    fn gauge_fraction_clamps_to_zero() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let gauge = gui.add_gauge(root);

        gui.set_gauge_fraction(gauge, -0.75).expect("gauge exists");
        gui.layout(Size::new(200.0, 64.0));

        let quads: Vec<_> = gui
            .render()
            .into_iter()
            .filter(|cmd| matches!(cmd, DrawCommand::Quad { .. }))
            .collect();

        assert_eq!(
            quads.len(),
            1,
            "only background quad rendered when fraction is zero"
        );
    }

    #[test]
    fn picture_letterboxes_and_frames_image() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let picture = gui.add_picture(root, 200.0, 100.0);

        let pixels = vec![255u8; 200 * 200 * 4];
        gui.set_picture_image(picture, Some(ImageData::new(200, 200, pixels)))
            .expect("set picture image");
        gui.layout(Size::new(200.0, 100.0));

        let mut image_rect = None;
        let mut frame_quads = 0;
        for cmd in gui.render() {
            match cmd {
                DrawCommand::Image { rect, .. } => image_rect = Some(rect),
                DrawCommand::Quad { .. } => frame_quads += 1,
                _ => {}
            }
        }

        let rect = image_rect.expect("image rendered");
        let picture_bounds = gui.rect_of(picture).expect("picture bounds");
        let padded_bounds = picture_bounds.inset(1.0).inset(6.0);
        assert!(
            (rect.size.width - rect.size.height).abs() < 0.01,
            "expected letterboxed image with matching width/height, got {} x {}",
            rect.size.width,
            rect.size.height
        );
        assert!(
            rect.size.width <= padded_bounds.size.width + 0.01,
            "letterboxed width should fit inside padded bounds"
        );
        assert!(
            rect.size.height <= padded_bounds.size.height + 0.01,
            "letterboxed height should fit inside padded bounds"
        );
        assert!(
            rect.origin.x >= padded_bounds.origin.x - 0.01,
            "image origin should start within padded bounds"
        );
        assert!(
            rect.origin.y >= padded_bounds.origin.y - 0.01,
            "image origin should start within padded bounds"
        );
        assert!(
            rect.origin.x + rect.size.width
                <= padded_bounds.origin.x + padded_bounds.size.width + 0.01,
            "image should not overflow padded bounds horizontally"
        );
        assert!(
            rect.origin.y + rect.size.height
                <= padded_bounds.origin.y + padded_bounds.size.height + 0.01,
            "image should not overflow padded bounds vertically"
        );
        assert!(
            frame_quads >= 2,
            "expected frame/background quads for picture widget"
        );
    }

    #[test]
    fn gauge_respects_layout_constraints() {
        let mut gui = Gui::new(test_font());
        let root = gui.root();
        let gauge = gui.add_gauge(root);

        gui.layout(Size::new(80.0, 32.0));

        let rect = gui.rect_of(gauge).expect("gauge has rect");
        assert!((rect.size.width - 64.0).abs() < f32::EPSILON);
        assert!(rect.size.height > 0.0);
    }
}
