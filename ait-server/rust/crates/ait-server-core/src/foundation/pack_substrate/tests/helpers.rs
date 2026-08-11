use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn temp_path(name: &str) -> String {
    temp_path_with_suffix(name, ".zstpack")
}

pub(super) fn temp_path_with_suffix(name: &str, suffix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("ait-{name}-{unique}{suffix}"))
        .to_string_lossy()
        .into_owned()
}

pub(super) fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
