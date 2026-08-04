use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command},
    time::{SystemTime, UNIX_EPOCH},
};

const CHILD_MODE: &str = "LC_L029_CHILD_MODE";
const CHILD_LOG_PATH: &str = "LC_L029_CHILD_LOG_PATH";
const INFO_MARKER: &str = "l029 session info marker";
const DEBUG_MARKER: &str = "l029 session debug marker";
const GPU_TEXTURE_MARKER: &str = "l029 wgpu texture allocation marker";

#[test]
fn session_log_is_overwritten_and_verbose_tees_to_stderr() {
    if let Ok(mode) = env::var(CHILD_MODE) {
        let log_path = PathBuf::from(env::var_os(CHILD_LOG_PATH).expect("child log path"));
        clonk_logging::init_verbose_with_file(mode == "verbose", &log_path)
            .expect("initialize child session log");
        tracing::info!(INFO_MARKER);
        tracing::debug!(DEBUG_MARKER);
        tracing::info!(target: "wgpu_core::device", GPU_TEXTURE_MARKER);
        let unused_path = log_path.with_file_name("unused-Clonk.log");
        let err = clonk_logging::init_verbose_with_file(false, &unused_path)
            .expect_err("a second file subscriber must not report success");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(
            !unused_path.exists(),
            "second log file should not be created"
        );
        return;
    }

    let temp_dir = unique_temp_dir();
    fs::create_dir_all(&temp_dir).expect("create test directory");

    for (mode, expect_debug) in [("normal", false), ("verbose", true)] {
        let log_path = temp_dir.join(mode).join("Clonk.log");
        if mode == "verbose" {
            fs::create_dir_all(log_path.parent().expect("verbose log parent"))
                .expect("create verbose log directory");
            fs::write(&log_path, "stale session bytes").expect("seed old session log");
        } else {
            assert!(!log_path.exists(), "normal log should start absent");
        }

        let output = Command::new(env::current_exe().expect("integration-test executable"))
            .args([
                "--exact",
                "session_log_is_overwritten_and_verbose_tees_to_stderr",
                "--nocapture",
            ])
            .env(CHILD_MODE, mode)
            .env(CHILD_LOG_PATH, &log_path)
            .env_remove("LC_LOG")
            .env_remove("RUST_LOG")
            .output()
            .expect("run isolated logging child");

        assert!(
            output.status.success(),
            "{mode} child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let session_log = fs::read_to_string(&log_path).expect("read new session log");

        assert!(stderr.contains(INFO_MARKER), "missing info on stderr");
        assert!(session_log.contains(INFO_MARKER), "missing info in file");
        assert!(!stdout.contains(INFO_MARKER), "info was written to stdout");
        assert!(!session_log.contains("stale session bytes"));
        assert_eq!(stderr.contains(DEBUG_MARKER), expect_debug);
        assert_eq!(session_log.contains(DEBUG_MARKER), expect_debug);
        assert!(
            !stdout.contains(DEBUG_MARKER),
            "debug was written to stdout"
        );
        // C++ logs only aggregate texture/material counts at C4Game.cpp:982-984;
        // its allocation/upload path at C4Surface.cpp:1242-1269 stays silent.
        assert!(
            !stderr.contains(GPU_TEXTURE_MARKER),
            "dependency texture allocation was written to stderr in {mode} mode"
        );
        assert!(
            !session_log.contains(GPU_TEXTURE_MARKER),
            "dependency texture allocation was written to the session log in {mode} mode"
        );
    }

    let _ = fs::remove_dir_all(temp_dir);
}

fn unique_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    env::temp_dir().join(format!("clonk-logging-l029-{}-{nonce}", process::id()))
}
