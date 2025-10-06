use crate::{
    DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, LayoutConstraints, Point, Rect, Size,
    WidgetId,
};
use lc_graphics::Color;
use std::convert::TryFrom;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

pub struct GuiHandle {
    gui: Gui,
}

pub struct RenderHandle {
    commands: Vec<LcGuiDrawCommand>,
    _texts: Vec<CString>,
}

pub struct EventResultHandle {
    captured: bool,
    actions: Vec<LcGuiEventAction>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LcGuiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl From<Color> for LcGuiColor {
    fn from(value: Color) -> Self {
        Self {
            r: value.r,
            g: value.g,
            b: value.b,
            a: value.a,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LcGuiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<Rect> for LcGuiRect {
    fn from(value: Rect) -> Self {
        Self {
            x: value.origin.x,
            y: value.origin.y,
            width: value.size.width,
            height: value.size.height,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LcGuiPoint {
    pub x: f32,
    pub y: f32,
}

impl From<LcGuiPoint> for Point {
    fn from(value: LcGuiPoint) -> Self {
        Point::new(value.x, value.y)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcGuiDrawCommandKind {
    Quad = 0,
    Text = 1,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LcGuiDrawCommand {
    pub kind: LcGuiDrawCommandKind,
    pub rect: LcGuiRect,
    pub color: LcGuiColor,
    pub text_ptr: *const c_char,
    pub text_len: usize,
}

impl LcGuiDrawCommand {
    fn quad(rect: Rect, color: Color) -> Self {
        Self {
            kind: LcGuiDrawCommandKind::Quad,
            rect: rect.into(),
            color: color.into(),
            text_ptr: ptr::null(),
            text_len: 0,
        }
    }

    fn text(rect: Rect, text: CString, color: Color) -> (Self, CString) {
        let len = text.as_bytes().len();
        let ptr = text.as_ptr();
        (
            Self {
                kind: LcGuiDrawCommandKind::Text,
                rect: rect.into(),
                color: color.into(),
                text_ptr: ptr,
                text_len: len,
            },
            text,
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcGuiEventKind {
    PointerDown = 0,
    PointerUp = 1,
    PointerMove = 2,
}

impl LcGuiEventKind {
    fn to_gui_event(self, point: Point) -> GuiEvent {
        match self {
            LcGuiEventKind::PointerDown => GuiEvent::PointerDown { position: point },
            LcGuiEventKind::PointerUp => GuiEvent::PointerUp { position: point },
            LcGuiEventKind::PointerMove => GuiEvent::PointerMove { position: point },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcGuiActionKind {
    Activate = 0,
}

impl From<GuiAction> for LcGuiActionKind {
    fn from(value: GuiAction) -> Self {
        match value {
            GuiAction::Activate => LcGuiActionKind::Activate,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LcGuiEventAction {
    pub widget_id: u32,
    pub action: LcGuiActionKind,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LcGuiEventResultView {
    pub captured: bool,
    pub actions: *const LcGuiEventAction,
    pub len: usize,
}

fn handle_mut(handle: *mut GuiHandle) -> Option<&'static mut GuiHandle> {
    unsafe { handle.as_mut() }
}

fn handle_ref(handle: *const GuiHandle) -> Option<&'static GuiHandle> {
    unsafe { handle.as_ref() }
}

fn read_widget(gui: &Gui, raw_id: u32) -> Option<WidgetId> {
    let widget = WidgetId::from_raw(raw_id as usize);
    if gui.rect_of(widget).is_some() || widget.to_raw() == gui.root().to_raw() {
        Some(widget)
    } else {
        None
    }
}

fn read_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe {
        CStr::from_ptr(ptr)
            .to_str()
            .map(|s| s.to_owned())
            .ok()
            .or_else(|| Some(CStr::from_ptr(ptr).to_string_lossy().into_owned()))
    }
}

fn to_u32(id: WidgetId) -> Option<u32> {
    u32::try_from(id.to_raw()).ok()
}

fn build_render_commands(commands: Vec<DrawCommand>) -> RenderHandle {
    let mut ffi_commands = Vec::with_capacity(commands.len());
    let mut texts = Vec::new();
    for command in commands {
        match command {
            DrawCommand::Quad { rect, color } => {
                ffi_commands.push(LcGuiDrawCommand::quad(rect, color))
            }
            DrawCommand::Text { rect, text, color } => {
                let sanitized: String = text.chars().filter(|ch| *ch != '\u{0}').collect();
                let c_string = CString::new(sanitized)
                    .unwrap_or_else(|_| CString::new("").expect("empty string"));
                let (cmd, stored) = LcGuiDrawCommand::text(rect, c_string, color);
                ffi_commands.push(cmd);
                texts.push(stored);
            }
        }
    }
    RenderHandle {
        commands: ffi_commands,
        _texts: texts,
    }
}

fn build_event_result(result: GuiEventResult) -> EventResultHandle {
    let mut actions = Vec::with_capacity(result.actions.len());
    for (widget, action) in result.actions {
        if let Some(widget_id) = to_u32(widget) {
            actions.push(LcGuiEventAction {
                widget_id,
                action: action.into(),
            });
        }
    }
    EventResultHandle {
        captured: result.captured,
        actions,
    }
}

fn layout_internal(handle: &mut GuiHandle, constraints: LayoutConstraints) {
    handle.gui.layout_with_constraints(constraints);
}

#[no_mangle]
pub extern "C" fn lc_gui_create() -> *mut GuiHandle {
    Box::into_raw(Box::new(GuiHandle { gui: Gui::new() }))
}

#[no_mangle]
pub extern "C" fn lc_gui_free(handle: *mut GuiHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_reset(handle: *mut GuiHandle) {
    if let Some(handle) = handle_mut(handle) {
        handle.gui = Gui::new();
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_root(handle: *const GuiHandle) -> u32 {
    match handle_ref(handle).and_then(|handle| to_u32(handle.gui.root())) {
        Some(id) => id,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_add_column(
    handle: *mut GuiHandle,
    parent: u32,
    expand_width: bool,
) -> u32 {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return 0,
    };
    let parent = match read_widget(&handle.gui, parent) {
        Some(parent) => parent,
        None => return 0,
    };
    match to_u32(handle.gui.add_column(parent, expand_width)) {
        Some(id) => id,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_add_label(
    handle: *mut GuiHandle,
    parent: u32,
    text: *const c_char,
) -> u32 {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return 0,
    };
    let parent = match read_widget(&handle.gui, parent) {
        Some(parent) => parent,
        None => return 0,
    };
    let text = match read_c_string(text) {
        Some(text) => text,
        None => return 0,
    };
    match to_u32(handle.gui.add_label(parent, text)) {
        Some(id) => id,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_add_button(
    handle: *mut GuiHandle,
    parent: u32,
    text: *const c_char,
) -> u32 {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return 0,
    };
    let parent = match read_widget(&handle.gui, parent) {
        Some(parent) => parent,
        None => return 0,
    };
    let text = match read_c_string(text) {
        Some(text) => text,
        None => return 0,
    };
    match to_u32(handle.gui.add_button(parent, text)) {
        Some(id) => id,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_layout(handle: *mut GuiHandle, max_width: f32, max_height: f32) -> bool {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return false,
    };
    layout_internal(
        handle,
        LayoutConstraints::tight(Size::new(max_width, max_height)),
    );
    true
}

#[no_mangle]
pub extern "C" fn lc_gui_layout_unbounded(handle: *mut GuiHandle) -> bool {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return false,
    };
    layout_internal(handle, LayoutConstraints::unbounded());
    true
}

#[no_mangle]
pub extern "C" fn lc_gui_render(handle: *const GuiHandle) -> *mut RenderHandle {
    let handle = match handle_ref(handle) {
        Some(handle) => handle,
        None => return ptr::null_mut(),
    };
    let render_handle = build_render_commands(handle.gui.render());
    Box::into_raw(Box::new(render_handle))
}

#[no_mangle]
pub extern "C" fn lc_gui_render_data(
    handle: *const RenderHandle,
    len_out: *mut usize,
) -> *const LcGuiDrawCommand {
    if handle.is_null() {
        if !len_out.is_null() {
            unsafe { *len_out = 0 };
        }
        return ptr::null();
    }
    let handle = unsafe { &*handle };
    if !len_out.is_null() {
        unsafe { *len_out = handle.commands.len() };
    }
    if handle.commands.is_empty() {
        ptr::null()
    } else {
        handle.commands.as_ptr()
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_render_free(handle: *mut RenderHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_pointer_event(
    handle: *mut GuiHandle,
    kind: LcGuiEventKind,
    point: LcGuiPoint,
) -> *mut EventResultHandle {
    let handle = match handle_mut(handle) {
        Some(handle) => handle,
        None => return ptr::null_mut(),
    };
    let result = handle.gui.handle_event(kind.to_gui_event(point.into()));
    Box::into_raw(Box::new(build_event_result(result)))
}

#[no_mangle]
pub extern "C" fn lc_gui_event_result_view(
    handle: *const EventResultHandle,
) -> LcGuiEventResultView {
    if handle.is_null() {
        return LcGuiEventResultView {
            captured: false,
            actions: ptr::null(),
            len: 0,
        };
    }
    let handle = unsafe { &*handle };
    let actions_ptr = if handle.actions.is_empty() {
        ptr::null()
    } else {
        handle.actions.as_ptr()
    };
    LcGuiEventResultView {
        captured: handle.captured,
        actions: actions_ptr,
        len: handle.actions.len(),
    }
}

#[no_mangle]
pub extern "C" fn lc_gui_event_result_free(handle: *mut EventResultHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_button_click_triggers_action() {
        unsafe {
            let gui = lc_gui_create();
            assert!(!gui.is_null());
            let root = lc_gui_root(gui);
            assert_eq!(root, 0);

            let label_text = CString::new("Heading").unwrap();
            let label_id = lc_gui_add_label(gui, root, label_text.as_ptr());
            assert_ne!(label_id, 0);

            let button_text = CString::new("Launch").unwrap();
            let button_id = lc_gui_add_button(gui, root, button_text.as_ptr());
            assert_ne!(button_id, 0);

            assert!(lc_gui_layout(gui, 320.0, 180.0));

            let render_handle = lc_gui_render(gui);
            assert!(!render_handle.is_null());
            let mut len = 0usize;
            let commands_ptr = lc_gui_render_data(render_handle, &mut len as *mut usize);
            assert!(len >= 2);
            let commands = if commands_ptr.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(commands_ptr, len)
            };
            let button_rect = commands
                .iter()
                .find(|cmd| cmd.kind == LcGuiDrawCommandKind::Quad)
                .map(|cmd| cmd.rect)
                .expect("button quad");
            lc_gui_render_free(render_handle);

            let center = LcGuiPoint {
                x: button_rect.x + button_rect.width * 0.5,
                y: button_rect.y + button_rect.height * 0.5,
            };
            let down = lc_gui_pointer_event(gui, LcGuiEventKind::PointerDown, center);
            assert!(!down.is_null());
            let down_view = lc_gui_event_result_view(down);
            assert!(down_view.captured);
            assert_eq!(down_view.len, 0);
            lc_gui_event_result_free(down);

            let up = lc_gui_pointer_event(gui, LcGuiEventKind::PointerUp, center);
            assert!(!up.is_null());
            let up_view = lc_gui_event_result_view(up);
            assert!(up_view.captured);
            assert_eq!(up_view.len, 1);
            let actions = if up_view.actions.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(up_view.actions, up_view.len)
            };
            assert_eq!(actions[0].widget_id, button_id);
            assert_eq!(actions[0].action, LcGuiActionKind::Activate);
            lc_gui_event_result_free(up);

            lc_gui_free(gui);
        }
    }
}
