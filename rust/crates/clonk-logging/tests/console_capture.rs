use std::io::Write;

use clonk_logging::ConsoleLogCapture;
use tracing_subscriber::fmt::MakeWriter;

#[test]
fn console_capture_projects_classic_warning_and_error_prefixes() {
    let capture = ConsoleLogCapture::default();
    {
        let mut writer = capture.make_writer();
        writer
            .write_all(
                b"2026-07-20T00:00:00Z  INFO opened scenario\n\
                  2026-07-20T00:00:01Z  WARN disk nearly full\n\
                  2026-07-20T00:00:02Z ERROR save failed\n",
            )
            .unwrap();
    }
    assert_eq!(
        capture.take(),
        "opened scenario\nWARNING: disk nearly full\nERROR: save failed\n"
    );
}
