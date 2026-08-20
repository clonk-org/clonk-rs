use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let directory = env::temp_dir().join(format!("{prefix}-{}-{nonce}", process::id()));
    fs::create_dir_all(&directory).expect("create test directory");
    directory
}
