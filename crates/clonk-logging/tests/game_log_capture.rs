use std::io::Write;

use clonk_logging::GameLogCapture;
use tracing_subscriber::fmt::MakeWriter;

#[test]
fn game_log_capture_projects_gui_sink_lines() {
    // `C4LogSystem::GuiSink` formats with "%*%v": the level prefix followed by
    // the stripped message, and `C4MessageBoard::AddLog` drops empty lines
    // (src/C4Log.cpp:44-83,187-204,226-240; src/C4MessageBoard.cpp:327-347).
    let capture = GameLogCapture::default();
    {
        let mut writer = capture.make_writer();
        writer
            .write_all(
                b"2026-07-20T00:00:00Z  INFO Alpha shows Beta that he ain't bullet-proof.\n\
                  2026-07-20T00:00:01Z  WARN low ammo\n\
                  2026-07-20T00:00:02Z ERROR script failed\n",
            )
            .unwrap();
    }
    assert_eq!(
        capture.take(),
        vec![
            "Alpha shows Beta that he ain't bullet-proof.".to_string(),
            "WARNING: low ammo".to_string(),
            "ERROR: script failed".to_string(),
        ]
    );
    assert!(
        capture.take().is_empty(),
        "a drained capture reports no further lines"
    );
}
