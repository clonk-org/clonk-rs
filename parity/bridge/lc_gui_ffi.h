#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LcGuiHandle LcGuiHandle;
typedef struct LcGuiRenderHandle LcGuiRenderHandle;
typedef struct LcGuiEventResultHandle LcGuiEventResultHandle;

typedef struct LcGuiColor {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
} LcGuiColor;

typedef struct LcGuiRect {
    float x;
    float y;
    float width;
    float height;
} LcGuiRect;

typedef struct LcGuiPoint {
    float x;
    float y;
} LcGuiPoint;

typedef enum LcGuiDrawCommandKind {
    LC_GUI_DRAW_COMMAND_QUAD = 0,
    LC_GUI_DRAW_COMMAND_TEXT = 1,
    LC_GUI_DRAW_COMMAND_IMAGE = 2,
} LcGuiDrawCommandKind;

typedef struct LcGuiDrawCommand {
    LcGuiDrawCommandKind kind;
   LcGuiRect rect;
   LcGuiColor color;
   const char *text_ptr;
   size_t text_len;
   float font_size;
   float padding;
   const uint8_t *image_ptr;
   size_t image_len;
   uint32_t image_width;
   uint32_t image_height;
} LcGuiDrawCommand;

typedef enum LcGuiEventKind {
    LC_GUI_EVENT_POINTER_DOWN = 0,
    LC_GUI_EVENT_POINTER_UP = 1,
    LC_GUI_EVENT_POINTER_MOVE = 2,
} LcGuiEventKind;

typedef enum LcGuiKeyEventKind {
    LC_GUI_KEY_EVENT_DOWN = 0,
    LC_GUI_KEY_EVENT_UP = 1,
} LcGuiKeyEventKind;

typedef enum LcGuiKeyCode {
    LC_GUI_KEY_ENTER = 0,
    LC_GUI_KEY_ESCAPE = 1,
    LC_GUI_KEY_SPACE = 2,
    LC_GUI_KEY_TAB = 3,
    LC_GUI_KEY_UP = 4,
    LC_GUI_KEY_DOWN = 5,
    LC_GUI_KEY_LEFT = 6,
    LC_GUI_KEY_RIGHT = 7,
} LcGuiKeyCode;

typedef enum LcGuiActionKind {
    LC_GUI_ACTION_ACTIVATE = 0,
} LcGuiActionKind;

typedef struct LcGuiEventAction {
    uint32_t widget_id;
    LcGuiActionKind action;
} LcGuiEventAction;

typedef struct LcGuiEventResultView {
    bool captured;
    const LcGuiEventAction *actions;
    size_t len;
} LcGuiEventResultView;

LcGuiHandle *lc_gui_create(void);
void lc_gui_free(LcGuiHandle *handle);
void lc_gui_reset(LcGuiHandle *handle);
uint32_t lc_gui_root(const LcGuiHandle *handle);
uint32_t lc_gui_add_column(LcGuiHandle *handle, uint32_t parent, bool expand_width);
uint32_t lc_gui_add_label(LcGuiHandle *handle, uint32_t parent, const char *text);
uint32_t lc_gui_add_button(LcGuiHandle *handle, uint32_t parent, const char *text);
bool lc_gui_layout(LcGuiHandle *handle, float max_width, float max_height);
bool lc_gui_layout_unbounded(LcGuiHandle *handle);

LcGuiRenderHandle *lc_gui_render(const LcGuiHandle *handle);
const LcGuiDrawCommand *lc_gui_render_data(const LcGuiRenderHandle *handle, size_t *len_out);
void lc_gui_render_free(LcGuiRenderHandle *handle);

LcGuiEventResultHandle *lc_gui_pointer_event(LcGuiHandle *handle, LcGuiEventKind kind, LcGuiPoint point);
LcGuiEventResultHandle *lc_gui_key_event(
    LcGuiHandle *handle,
    LcGuiKeyEventKind kind,
    LcGuiKeyCode key
);
LcGuiEventResultView lc_gui_event_result_view(const LcGuiEventResultHandle *handle);
void lc_gui_event_result_free(LcGuiEventResultHandle *handle);

#ifdef __cplusplus
}
#endif
