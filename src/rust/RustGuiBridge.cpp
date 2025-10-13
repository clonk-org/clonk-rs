#include "RustGuiBridge.h"

#ifdef USE_RUST_GUI_VALIDATION

#include <stdexcept>
#include <utility>

namespace RustGuiBridge {
namespace {

Color ToColor(const LcGuiColor &color) {
    return Color{color.r, color.g, color.b, color.a};
}

Rect ToRect(const LcGuiRect &rect) {
    return Rect{rect.x, rect.y, rect.width, rect.height};
}

GuiAction ToGuiAction(LcGuiActionKind action) {
    switch (action) {
    case LC_GUI_ACTION_ACTIVATE:
        return GuiAction::Activate;
    }
    throw std::runtime_error("Unsupported GUI action from Rust");
}

LcGuiKeyCode ToFfiKeyCode(KeyCode key) {
    switch (key) {
    case KeyCode::Enter:
        return LC_GUI_KEY_ENTER;
    case KeyCode::Escape:
        return LC_GUI_KEY_ESCAPE;
    case KeyCode::Space:
        return LC_GUI_KEY_SPACE;
    case KeyCode::Tab:
        return LC_GUI_KEY_TAB;
    case KeyCode::Up:
        return LC_GUI_KEY_UP;
    case KeyCode::Down:
        return LC_GUI_KEY_DOWN;
    case KeyCode::Left:
        return LC_GUI_KEY_LEFT;
    case KeyCode::Right:
        return LC_GUI_KEY_RIGHT;
    }
    throw std::runtime_error("Unsupported GUI key code");
}

std::vector<DrawCommand> ConvertCommands(const LcGuiDrawCommand *commands, size_t len) {
    std::vector<DrawCommand> result;
    if (commands == nullptr || len == 0) {
        return result;
    }
    result.reserve(len);
    for (size_t i = 0; i < len; ++i) {
        const auto &command = commands[i];
        DrawCommand converted;
        converted.rect = ToRect(command.rect);
        converted.color = ToColor(command.color);
        converted.font_size = command.font_size;
        converted.padding = command.padding;
        switch (command.kind) {
        case LC_GUI_DRAW_COMMAND_QUAD:
            converted.kind = DrawCommandKind::Quad;
            converted.pixels.clear();
            converted.image_width = 0;
            converted.image_height = 0;
            break;
        case LC_GUI_DRAW_COMMAND_TEXT:
            converted.kind = DrawCommandKind::Text;
            if (command.text_ptr != nullptr && command.text_len > 0) {
                converted.text.assign(command.text_ptr, command.text_len);
            } else {
                converted.text.clear();
            }
            converted.pixels.clear();
            converted.image_width = 0;
            converted.image_height = 0;
            break;
        case LC_GUI_DRAW_COMMAND_IMAGE:
            converted.kind = DrawCommandKind::Image;
            converted.text.clear();
            if (command.image_ptr != nullptr && command.image_len > 0) {
                converted.pixels.assign(
                    command.image_ptr,
                    command.image_ptr + command.image_len
                );
            } else {
                converted.pixels.clear();
            }
            converted.image_width = command.image_width;
            converted.image_height = command.image_height;
            break;
        default:
            throw std::runtime_error("Unknown GUI draw command from Rust");
        }
        result.emplace_back(std::move(converted));
    }
    return result;
}

EventResult ConvertEventResult(const LcGuiEventResultView &view) {
    EventResult result;
    result.captured = view.captured;
    if (view.len == 0 || view.actions == nullptr) {
        return result;
    }
    result.actions.reserve(view.len);
    for (size_t i = 0; i < view.len; ++i) {
        const auto &action = view.actions[i];
        result.actions.push_back(EventAction{action.widget_id, ToGuiAction(action.action)});
    }
    return result;
}

} // namespace

Gui::Gui() : handle_{lc_gui_create()} {
    if (!handle_) {
        throw std::runtime_error("Failed to create Rust GUI handle");
    }
}

Gui::~Gui() {
    if (handle_) {
        lc_gui_free(handle_);
        handle_ = nullptr;
    }
}

Gui::Gui(Gui &&other) noexcept : handle_{std::exchange(other.handle_, nullptr)} {}

Gui &Gui::operator=(Gui &&other) noexcept {
    if (this != &other) {
        if (handle_) {
            lc_gui_free(handle_);
        }
        handle_ = std::exchange(other.handle_, nullptr);
    }
    return *this;
}

void Gui::EnsureHandle() const {
    if (!handle_) {
        throw std::runtime_error("Rust GUI handle is not initialised");
    }
}

void Gui::Reset() {
    EnsureHandle();
    lc_gui_reset(handle_);
}

uint32_t Gui::Root() const {
    EnsureHandle();
    return lc_gui_root(handle_);
}

uint32_t Gui::AddColumn(uint32_t parent, bool expand_width) {
    EnsureHandle();
    const uint32_t id = lc_gui_add_column(handle_, parent, expand_width);
    if (id == 0) {
        throw std::runtime_error("Rust GUI failed to add column");
    }
    return id;
}

uint32_t Gui::AddLabel(uint32_t parent, std::string_view text) {
    EnsureHandle();
    const std::string copy(text);
    const uint32_t id = lc_gui_add_label(handle_, parent, copy.c_str());
    if (id == 0) {
        throw std::runtime_error("Rust GUI failed to add label");
    }
    return id;
}

uint32_t Gui::AddButton(uint32_t parent, std::string_view text) {
    EnsureHandle();
    const std::string copy(text);
    const uint32_t id = lc_gui_add_button(handle_, parent, copy.c_str());
    if (id == 0) {
        throw std::runtime_error("Rust GUI failed to add button");
    }
    return id;
}

void Gui::Layout(float max_width, float max_height) {
    EnsureHandle();
    if (!lc_gui_layout(handle_, max_width, max_height)) {
        throw std::runtime_error("Rust GUI layout failed");
    }
}

void Gui::LayoutUnbounded() {
    EnsureHandle();
    if (!lc_gui_layout_unbounded(handle_)) {
        throw std::runtime_error("Rust GUI unbounded layout failed");
    }
}

std::vector<DrawCommand> Gui::Render() const {
    EnsureHandle();
    LcGuiRenderHandle *render_handle = lc_gui_render(handle_);
    if (!render_handle) {
        return {};
    }
    size_t len = 0;
    const LcGuiDrawCommand *commands = lc_gui_render_data(render_handle, &len);
    auto converted = ConvertCommands(commands, len);
    lc_gui_render_free(render_handle);
    return converted;
}

EventResult Gui::PointerDown(Point point) {
    return DispatchPointerEvent(LC_GUI_EVENT_POINTER_DOWN, point);
}

EventResult Gui::PointerUp(Point point) {
    return DispatchPointerEvent(LC_GUI_EVENT_POINTER_UP, point);
}

EventResult Gui::PointerMove(Point point) {
    return DispatchPointerEvent(LC_GUI_EVENT_POINTER_MOVE, point);
}

EventResult Gui::KeyDown(KeyCode key) {
    return DispatchKeyEvent(LC_GUI_KEY_EVENT_DOWN, key);
}

EventResult Gui::KeyUp(KeyCode key) {
    return DispatchKeyEvent(LC_GUI_KEY_EVENT_UP, key);
}

EventResult Gui::DispatchPointerEvent(LcGuiEventKind kind, Point point) {
    EnsureHandle();
    const LcGuiPoint ffi_point{point.x, point.y};
    LcGuiEventResultHandle *event_handle = lc_gui_pointer_event(handle_, kind, ffi_point);
    if (!event_handle) {
        return {};
    }
    const LcGuiEventResultView view = lc_gui_event_result_view(event_handle);
    auto converted = ConvertEventResult(view);
    lc_gui_event_result_free(event_handle);
    return converted;
}

EventResult Gui::DispatchKeyEvent(LcGuiKeyEventKind kind, KeyCode key) {
    EnsureHandle();
    const LcGuiKeyCode ffi_key = ToFfiKeyCode(key);
    LcGuiEventResultHandle *event_handle = lc_gui_key_event(handle_, kind, ffi_key);
    if (!event_handle) {
        return {};
    }
    const LcGuiEventResultView view = lc_gui_event_result_view(event_handle);
    auto converted = ConvertEventResult(view);
    lc_gui_event_result_free(event_handle);
    return converted;
}

} // namespace RustGuiBridge

#endif // USE_RUST_GUI_VALIDATION
