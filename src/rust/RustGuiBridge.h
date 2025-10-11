#pragma once

#ifdef USE_RUST_GUI_VALIDATION

#include "lc_gui_ffi.h"

#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace RustGuiBridge {

struct Color {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
};

struct Rect {
    float x;
    float y;
    float width;
    float height;
};

struct Point {
    float x;
    float y;
};

enum class DrawCommandKind {
    Quad,
    Text,
};

enum class GuiAction {
    Activate,
};

enum class KeyCode {
    Enter,
    Escape,
    Space,
    Tab,
    Up,
    Down,
    Left,
    Right,
};

struct DrawCommand {
    DrawCommandKind kind;
    Rect rect;
    Color color;
    std::string text;
    float font_size {0.0f};
    float padding {0.0f};
};

struct EventAction {
    uint32_t widget_id;
    GuiAction action;
};

struct EventResult {
    bool captured {false};
    std::vector<EventAction> actions;
};

class Gui {
public:
    Gui();
    ~Gui();

    Gui(Gui &&other) noexcept;
    Gui &operator=(Gui &&other) noexcept;

    Gui(const Gui &) = delete;
    Gui &operator=(const Gui &) = delete;

    void Reset();

    uint32_t Root() const;
    uint32_t AddColumn(uint32_t parent, bool expand_width);
    uint32_t AddLabel(uint32_t parent, std::string_view text);
    uint32_t AddButton(uint32_t parent, std::string_view text);

    void Layout(float max_width, float max_height);
    void LayoutUnbounded();

    std::vector<DrawCommand> Render() const;

    EventResult PointerDown(Point point);
    EventResult PointerUp(Point point);
    EventResult PointerMove(Point point);
    EventResult KeyDown(KeyCode key);
    EventResult KeyUp(KeyCode key);

private:
    EventResult DispatchPointerEvent(LcGuiEventKind kind, Point point);
    EventResult DispatchKeyEvent(LcGuiKeyEventKind kind, KeyCode key);
    void EnsureHandle() const;

    LcGuiHandle *handle_ {nullptr};
};

} // namespace RustGuiBridge

#endif // USE_RUST_GUI_VALIDATION
