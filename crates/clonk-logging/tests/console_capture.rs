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

#[test]
fn console_writer_commits_pending_bytes_on_flush() {
    // `io::Write::flush` must make written bytes observable; committing only in
    // `Drop` forces every caller to scope the writer to see its own output.
    let capture = ConsoleLogCapture::default();
    let mut writer = capture.make_writer();
    writer.write_all(b"line one\n").unwrap();
    writer.flush().unwrap();
    assert_eq!(capture.take(), "line one\n");
}
