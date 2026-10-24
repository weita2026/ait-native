use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::external::materializer::ExternalLocalLinkOverride;
use crate::external::{ExternalError, ExternalResult};

pub const EXTERNAL_LINKS_FILE: &str = "ait-external.links.toml";

pub trait ExternalLinkStore {
    fn load_links(&self) -> ExternalResult<Vec<ExternalLocalLinkOverride>>;
    fn save_links(&self, links: &[ExternalLocalLinkOverride]) -> ExternalResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsExternalLinkStore {
    path: PathBuf,
}

impl FsExternalLinkStore {
    pub fn for_repo_root(repo_root: impl AsRef<Path>) -> Self {
        Self {
            path: repo_root.as_ref().join(EXTERNAL_LINKS_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ExternalLinkStore for FsExternalLinkStore {
    fn load_links(&self) -> ExternalResult<Vec<ExternalLocalLinkOverride>> {
        match fs::read(&self.path) {
            Ok(bytes) => parse_external_local_link_overrides(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(ExternalError::with_code(
                "external_local_links_read",
                format!("failed to read {}: {err}", self.path.display()),
            )),
        }
    }

    fn save_links(&self, links: &[ExternalLocalLinkOverride]) -> ExternalResult<()> {
        if links.is_empty() {
            match fs::remove_file(&self.path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
                Err(err) => {
                    return Err(ExternalError::with_code(
                        "external_local_links_remove",
                        format!("failed to remove {}: {err}", self.path.display()),
                    ));
                }
            }
        }

        let bytes = render_external_local_link_overrides(links)?;
        atomic_write(&self.path, &bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLinkMutation {
    pub links: Vec<ExternalLocalLinkOverride>,
    pub changed: bool,
}

pub fn parse_external_local_link_overrides(
    bytes: &[u8],
) -> ExternalResult<Vec<ExternalLocalLinkOverride>> {
    let value = std::str::from_utf8(bytes)
        .map_err(|err| {
            ExternalError::with_code(
                "external_local_links_utf8",
                format!("{EXTERNAL_LINKS_FILE} must be UTF-8: {err}"),
            )
        })?
        .parse::<toml::Value>()
        .map_err(|err| {
            ExternalError::with_code(
                "external_local_links_parse",
                format!("failed to parse {EXTERNAL_LINKS_FILE}: {err}"),
            )
        })?;
    let Some(links) = value.get("link").and_then(toml::Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for link in links {
        let name = normalized_link_name(
            link.get("name")
                .and_then(toml::Value::as_str)
                .unwrap_or_default(),
        )?;
        let path = normalized_link_path(
            link.get("path")
                .and_then(toml::Value::as_str)
                .unwrap_or_default(),
            &name,
        )?;
        parsed.push(ExternalLocalLinkOverride { name, path });
    }
    Ok(sorted_unique_links(parsed))
}

pub fn render_external_local_link_overrides(
    links: &[ExternalLocalLinkOverride],
) -> ExternalResult<Vec<u8>> {
    let entries = sorted_unique_links(links.to_vec())
        .into_iter()
        .map(|link| LinkTomlEntry {
            name: link.name,
            path: link.path,
        })
        .collect::<Vec<_>>();
    toml::to_string_pretty(&LinksToml { link: entries })
        .map(|text| text.into_bytes())
        .map_err(|err| {
            ExternalError::with_code(
                "external_local_links_render",
                format!("failed to render {EXTERNAL_LINKS_FILE}: {err}"),
            )
        })
}

pub fn upsert_external_local_link_override(
    links: &[ExternalLocalLinkOverride],
    name: &str,
    path: &str,
) -> ExternalResult<ExternalLinkMutation> {
    let name = normalized_link_name(name)?;
    let path = normalized_link_path(path, &name)?;
    let mut by_name = links_by_name(links);
    let changed = by_name.get(&name) != Some(&path);
    by_name.insert(name, path);
    Ok(ExternalLinkMutation {
        links: links_from_map(by_name),
        changed,
    })
}

pub fn remove_external_local_link_override(
    links: &[ExternalLocalLinkOverride],
    name: &str,
) -> ExternalResult<ExternalLinkMutation> {
    let name = normalized_link_name(name)?;
    let mut by_name = links_by_name(links);
    let changed = by_name.remove(&name).is_some();
    Ok(ExternalLinkMutation {
        links: links_from_map(by_name),
        changed,
    })
}

#[derive(Debug, Serialize)]
struct LinksToml {
    link: Vec<LinkTomlEntry>,
}

#[derive(Debug, Serialize)]
struct LinkTomlEntry {
    name: String,
    path: String,
}

fn normalized_link_name(name: &str) -> ExternalResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ExternalError::with_code(
            "external_local_links_name",
            "local external link entry must include a non-empty name",
        ));
    }
    Ok(trimmed.to_string())
}

fn normalized_link_path(path: &str, name: &str) -> ExternalResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ExternalError::with_code(
            "external_local_links_path",
            format!("local external link {name:?} must include a non-empty path"),
        ));
    }
    Ok(trimmed.to_string())
}

fn sorted_unique_links(links: Vec<ExternalLocalLinkOverride>) -> Vec<ExternalLocalLinkOverride> {
    links_from_map(
        links
            .iter()
            .map(|link| (link.name.clone(), link.path.clone()))
            .collect(),
    )
}

fn links_by_name(links: &[ExternalLocalLinkOverride]) -> BTreeMap<String, String> {
    links
        .iter()
        .map(|link| (link.name.clone(), link.path.clone()))
        .collect()
}

fn links_from_map(by_name: BTreeMap<String, String>) -> Vec<ExternalLocalLinkOverride> {
    by_name
        .into_iter()
        .map(|(name, path)| ExternalLocalLinkOverride { name, path })
        .collect()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> ExternalResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                ExternalError::with_code(
                    "external_local_links_parent",
                    format!("failed to create {}: {err}", parent.display()),
                )
            })?;
        }
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(EXTERNAL_LINKS_FILE);
    let temp_path = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&temp_path, bytes).map_err(|err| {
        ExternalError::with_code(
            "external_local_links_write",
            format!("failed to write {}: {err}", temp_path.display()),
        )
    })?;
    fs::rename(&temp_path, path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        ExternalError::with_code(
            "external_local_links_commit",
            format!("failed to replace {}: {err}", path.display()),
        )
    })
}
