use std::io::Write;

use clonk_logging::ConsoleLogCapture;
use tracing_subscriber::fmt::MakeWriter;

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
