//! A new CLI init may prepare Newton. Keep automated fixture sessions off the user's native store.
#![allow(dead_code)]
use std::{path::Path, sync::OnceLock};
use tempfile::TempDir;
pub fn path() -> &'static Path {
    static HOME: OnceLock<TempDir> = OnceLock::new();
    HOME.get_or_init(|| TempDir::new().unwrap()).path()
}
