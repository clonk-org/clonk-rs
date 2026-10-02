# Unified settings screenshots

Review images for the unified settings overlay. Every frame was rendered by
the real app through the `CLONK_SETTINGS_IMAGES` fixture in
`crates/clonk-app/src/main_tests/unified_settings.rs`
(`capture_unified_settings_fixture`), which saves each state at 640x480,
800x600 and 1280x720. The "before" halves are the native options book that
the LegacyClonk compatibility profile still opens.

| Image | Shows |
| --- | --- |
| `general-before-after.jpg` | The options book's Program sheet beside the General tab and its section headings |
| `controls-before-after.jpg` | The options book's Keyboard sheet beside the Controls tab's keyboard grid |
| `controls-capture-and-controller.jpg` | A key waiting for its new key while the command holding it is framed; the controller sets |
| `voice-and-choice-list.jpg` | Voice chat with the microphone test on its row; a choice list opened under its value |
| `in-game-popup.jpg` | Settings over a paused game at 1280x720: the book alone, closed from the mark on its paper |
| `in-game-640x480.jpg` | The same at the smallest supported window |
| `language-and-font-before-after.jpg` | The options book's Program sheet beside General's Language and font section, choosing a language |
| `general-font-chat-and-program.jpg` | The font size list, which scrolls past eight sizes; white chat and timestamps under Chat; Preload game data under Program |
