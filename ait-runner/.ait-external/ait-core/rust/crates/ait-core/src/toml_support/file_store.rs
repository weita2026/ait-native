use crate::file_io::FileIoStore;
use crate::toml_support::{TomlCodec, TomlEncodeOptions};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TomlStoreError {
    message: String,
}

impl TomlStoreError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TomlStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TomlStoreError {}

impl From<TomlStoreError> for String {
    fn from(error: TomlStoreError) -> Self {
        error.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingTomlFilePolicy {
    Error,
    ReturnNone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TomlReadOptions {
    pub missing_file: MissingTomlFilePolicy,
}

impl TomlReadOptions {
    pub fn required() -> Self {
        Self {
            missing_file: MissingTomlFilePolicy::Error,
        }
    }

    pub fn optional() -> Self {
        Self {
            missing_file: MissingTomlFilePolicy::ReturnNone,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TomlWriteMode {
    Direct,
    Atomic { publish_label: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TomlWriteOptions {
    pub encode: TomlEncodeOptions,
    pub mode: TomlWriteMode,
}

impl TomlWriteOptions {
    pub fn direct() -> Self {
        Self {
            encode: TomlEncodeOptions::pretty(),
            mode: TomlWriteMode::Direct,
        }
    }

    pub fn atomic(publish_label: impl Into<String>) -> Self {
        Self {
            encode: TomlEncodeOptions::pretty(),
            mode: TomlWriteMode::Atomic {
                publish_label: publish_label.into(),
            },
        }
    }

    pub fn with_trailing_newline(mut self) -> Self {
        self.encode = self.encode.with_trailing_newline();
        self
    }
}

pub fn expand_home_path_with_toml_file_store<S>(store: &S, path_value: &str) -> PathBuf
where
    S: FileIoStore + ?Sized,
{
    if path_value == "~" {
        if let Some(home) = store.home_dir() {
            return home;
        }
    }
    if let Some(suffix) = path_value.strip_prefix("~/") {
        if let Some(home) = store.home_dir() {
            return home.join(suffix);
        }
    }
    PathBuf::from(path_value)
}

pub fn read_toml_value_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
    options: TomlReadOptions,
) -> Result<Option<toml::Value>, TomlStoreError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_toml_file_store(store, path_value);
    read_toml_value_at_path_with_file_io_store(store, &path, label, options)
}

pub fn read_toml_value_at_path_with_file_io_store<S>(
    store: &S,
    path: &Path,
    label: &str,
    options: TomlReadOptions,
) -> Result<Option<toml::Value>, TomlStoreError>
where
    S: FileIoStore + ?Sized,
{
    if !store.path_exists(path) {
        return match options.missing_file {
            MissingTomlFilePolicy::ReturnNone => Ok(None),
            MissingTomlFilePolicy::Error => Err(TomlStoreError::new(format!(
                "Missing {label} TOML {}.",
                path.display()
            ))),
        };
    }
    let text = store.read_to_string(path).map_err(|err| {
        TomlStoreError::new(format!(
            "Failed to read {label} TOML {}: {err}",
            path.display()
        ))
    })?;
    let value = TomlCodec::parse_value(&text, label).map_err(|err| {
        TomlStoreError::new(format!("Invalid {label} TOML {}: {err}", path.display()))
    })?;
    Ok(Some(value))
}

pub fn read_toml_table_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
    options: TomlReadOptions,
) -> Result<Option<toml::map::Map<String, toml::Value>>, TomlStoreError>
where
    S: FileIoStore + ?Sized,
{
    read_toml_value_with_file_io_store(store, path_value, label, options)?
        .map(|value| match value {
            toml::Value::Table(table) => Ok(table),
            _ => Err(TomlStoreError::new(format!(
                "{label} TOML must be a table."
            ))),
        })
        .transpose()
}

pub fn write_toml_value_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    value: &toml::Value,
    label: &str,
    options: TomlWriteOptions,
) -> Result<(), TomlStoreError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_toml_file_store(store, path_value);
    write_toml_value_at_path_with_file_io_store(store, &path, value, label, options)
}

pub fn write_toml_value_at_path_with_file_io_store<S>(
    store: &S,
    path: &Path,
    value: &toml::Value,
    label: &str,
    options: TomlWriteOptions,
) -> Result<(), TomlStoreError>
where
    S: FileIoStore + ?Sized,
{
    let text = TomlCodec::encode_value(value, options.encode).map_err(|err| {
        TomlStoreError::new(format!(
            "Failed to encode {label} TOML {}: {err}",
            path.display()
        ))
    })?;
    match options.mode {
        TomlWriteMode::Direct => store.write_string(path, &text).map_err(|err| {
            TomlStoreError::new(format!(
                "Failed to write {label} TOML {}: {err}",
                path.display()
            ))
        }),
        TomlWriteMode::Atomic { publish_label } => store
            .write_string_atomically(path, &text, &publish_label)
            .map_err(|err| {
                TomlStoreError::new(format!(
                    "Failed to write {label} TOML {}: {err}",
                    path.display()
                ))
            }),
    }
}

pub trait TomlFileStore: FileIoStore {
    fn expand_toml_path(&self, path_value: &str) -> PathBuf {
        expand_home_path_with_toml_file_store(self, path_value)
    }

    fn read_toml_value(
        &self,
        path_value: &str,
        label: &str,
        options: TomlReadOptions,
    ) -> Result<Option<toml::Value>, TomlStoreError> {
        read_toml_value_with_file_io_store(self, path_value, label, options)
    }

    fn read_toml_table(
        &self,
        path_value: &str,
        label: &str,
        options: TomlReadOptions,
    ) -> Result<Option<toml::map::Map<String, toml::Value>>, TomlStoreError> {
        read_toml_table_with_file_io_store(self, path_value, label, options)
    }

    fn write_toml_value(
        &self,
        path_value: &str,
        value: &toml::Value,
        label: &str,
        options: TomlWriteOptions,
    ) -> Result<(), TomlStoreError> {
        write_toml_value_with_file_io_store(self, path_value, value, label, options)
    }

    fn write_toml_value_at_path(
        &self,
        path: &Path,
        value: &toml::Value,
        label: &str,
        options: TomlWriteOptions,
    ) -> Result<(), TomlStoreError> {
        write_toml_value_at_path_with_file_io_store(self, path, value, label, options)
    }
}

impl<T> TomlFileStore for T where T: FileIoStore + ?Sized {}
