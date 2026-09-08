// The C++ side of the GUI bridge differential (clonk-org/clonk-rs#1266).
//
// The pinned oracle links the Rust GUI wrapper (src/rust/RustGuiBridge.*) but
// never calls it, so this harness is the consumer: it drives the wrapper over
// the pinned C ABI through the script crates/clonk-gui/examples/bridge_scenario.rs
// runs through the safe API, and prints the same lines. The differential diffs
// the two dumps; `--perturb` changes one label so they must differ.
//
// Ownership is exercised on the way: the tree is built in one Gui, moved into
// another before layout (the moved-from wrapper must free nothing), render and
// event handles are freed by the wrapper on every call, reset replaces the
// tree under the same handle, and both wrappers are destroyed at exit, which
// is what `leaks --atExit` checks.
//
// Build: see parity/bridge/run-gui-differential.sh.

#include "RustGuiBridge.h"

#include <cstdio>
#include <cstring>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace {

constexpr const char *kLabel = "Hello, bridge";
constexpr const char *kPerturbedLabel = "Hello, bridge!";
constexpr const char *kButton = "Press me";
constexpr const char *kNested = "Nested";
constexpr float kBoundedWidth = 320.0f;
constexpr float kBoundedHeight = 240.0f;

std::string RectText(const RustGuiBridge::Rect &rect) {
    char buffer[96];
    std::snprintf(buffer, sizeof buffer, "%.3f,%.3f,%.3f,%.3f", rect.x, rect.y, rect.width, rect.height);
    return buffer;
}

void DumpRender(const char *stage, const std::vector<RustGuiBridge::DrawCommand> &commands) {
    std::printf("render %s count=%zu\n", stage, commands.size());
    for (std::size_t index = 0; index < commands.size(); ++index) {
        const auto &command = commands[index];
        switch (command.kind) {
        case RustGuiBridge::DrawCommandKind::Quad:
            std::printf("draw %zu quad rect=%s color=%u,%u,%u,%u\n", index, RectText(command.rect).c_str(),
                        command.color.r, command.color.g, command.color.b, command.color.a);
            break;
        case RustGuiBridge::DrawCommandKind::Text:
            std::printf("draw %zu text rect=%s color=%u,%u,%u,%u text=\"%s\" font=%.3f padding=%.3f\n", index,
                        RectText(command.rect).c_str(), command.color.r, command.color.g, command.color.b,
                        command.color.a, command.text.c_str(), command.font_size, command.padding);
            break;
        case RustGuiBridge::DrawCommandKind::Image:
            std::printf("draw %zu image rect=%s width=%u height=%u bytes=%zu\n", index, RectText(command.rect).c_str(),
                        command.image_width, command.image_height, command.pixels.size());
            break;
        }
    }
}

void DumpResult(const char *event, const RustGuiBridge::EventResult &result) {
    std::string actions;
    for (const auto &action : result.actions) {
        if (!actions.empty()) {
            actions += ",";
        }
        actions += std::to_string(action.widget_id);
        actions += ":";
        switch (action.action) {
        case RustGuiBridge::GuiAction::Activate:
            actions += "activate";
            break;
        }
    }
    std::printf("event %s captured=%s actions=[%s]\n", event, result.captured ? "true" : "false", actions.c_str());
}

// The pointer target both sides derive the same way: the centre of the text
// command carrying the button's caption.
std::optional<RustGuiBridge::Point> ButtonCentre(const std::vector<RustGuiBridge::DrawCommand> &commands) {
    for (const auto &command : commands) {
        if (command.kind == RustGuiBridge::DrawCommandKind::Text && command.text == kButton) {
            return RustGuiBridge::Point{command.rect.x + command.rect.width / 2.0f,
                                        command.rect.y + command.rect.height / 2.0f};
        }
    }
    return std::nullopt;
}

} // namespace

int main(int argc, char **argv) {
    bool perturb = false;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--perturb") == 0) {
            perturb = true;
        }
    }

    RustGuiBridge::Gui built;
    const uint32_t root = built.Root();
    const uint32_t column = built.AddColumn(root, true);
    const uint32_t label = built.AddLabel(column, perturb ? kPerturbedLabel : kLabel);
    const uint32_t button = built.AddButton(column, kButton);
    const uint32_t inner = built.AddColumn(column, false);
    const uint32_t nested = built.AddLabel(inner, kNested);
    std::printf("ids root=%u column=%u label=%u button=%u inner=%u nested=%u\n", root, column, label, button, inner,
                nested);

    // Move ownership: the tree continues under a second wrapper and the first
    // must be left holding nothing.
    RustGuiBridge::Gui gui(std::move(built));

    gui.Layout(kBoundedWidth, kBoundedHeight);
    const auto bounded = gui.Render();
    DumpRender("bounded", bounded);

    const auto centre = ButtonCentre(bounded);
    if (!centre) {
        std::fprintf(stderr, "the button caption was not rendered\n");
        return 2;
    }
    std::printf("pointer target=%.3f,%.3f\n", centre->x, centre->y);
    const RustGuiBridge::Point outside{-1.0f, -1.0f};
    DumpResult("move", gui.PointerMove(*centre));
    DumpResult("down", gui.PointerDown(*centre));
    DumpResult("up", gui.PointerUp(*centre));
    DumpResult("down-outside", gui.PointerDown(outside));
    DumpResult("up-outside", gui.PointerUp(outside));
    DumpResult("key-down-tab", gui.KeyDown(RustGuiBridge::KeyCode::Tab));
    DumpResult("key-down-enter", gui.KeyDown(RustGuiBridge::KeyCode::Enter));
    DumpResult("key-up-enter", gui.KeyUp(RustGuiBridge::KeyCode::Enter));
    DumpResult("key-down-escape", gui.KeyDown(RustGuiBridge::KeyCode::Escape));

    gui.LayoutUnbounded();
    DumpRender("unbounded", gui.Render());

    gui.Reset();
    gui.Layout(kBoundedWidth, kBoundedHeight);
    DumpRender("after-reset", gui.Render());
    std::printf("root-after-reset=%u\n", gui.Root());
    return 0;
}
