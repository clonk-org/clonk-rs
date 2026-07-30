use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn the_previous_session_log_is_kept_beside_the_new_one() {
    // A user files a bug report after relaunching, so the run that misbehaved
    // is already the *previous* session. Truncating on startup destroys the
    // only copy of it.
    let directory = unique_temp_dir();
    let log_path = directory.join("Clonk.log");
    fs::write(&log_path, "prior session\n").expect("seed the previous session log");

    clonk_logging::init_verbose_with_file(false, &log_path).expect("initialize the session log");
    tracing::info!("new session marker");

    let previous = fs::read_to_string(directory.join("Clonk.previous.log"))
        .expect("read the retained previous session log");
    assert!(
        previous.contains("prior session"),
        "the previous session log is retained verbatim: {previous:?}"
    );

    let current = fs::read_to_string(&log_path).expect("read the new session log");
    assert!(
        !current.contains("prior session"),
        "the new session log starts empty"
    );
    assert!(current.contains("new session marker"));

    let _ = fs::remove_dir_all(&directory);
}

fn unique_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let directory =
        env::temp_dir().join(format!("clonk-logging-rotation-{}-{nonce}", process::id()));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
