use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TomlCodecError {
    message: String,
}

impl TomlCodecError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TomlCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TomlCodecError {}

impl From<TomlCodecError> for String {
    fn from(error: TomlCodecError) -> Self {
        error.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TomlEncodeStyle {
    Compact,
    Pretty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TomlEncodeOptions {
    pub style: TomlEncodeStyle,
    pub trailing_newline: bool,
}

impl TomlEncodeOptions {
    pub fn compact() -> Self {
        Self {
            style: TomlEncodeStyle::Compact,
            trailing_newline: false,
        }
    }

    pub fn pretty() -> Self {
        Self {
            style: TomlEncodeStyle::Pretty,
            trailing_newline: false,
        }
    }

    pub fn with_trailing_newline(mut self) -> Self {
        self.trailing_newline = true;
        self
    }
}

pub struct TomlCodec;

impl TomlCodec {
    pub fn encode_value(
        value: &toml::Value,
        options: TomlEncodeOptions,
    ) -> Result<String, TomlCodecError> {
        let mut text = match options.style {
            TomlEncodeStyle::Compact => toml::to_string(value),
            TomlEncodeStyle::Pretty => toml::to_string_pretty(value),
        }
        .map_err(|err| TomlCodecError::new(format!("Failed to encode TOML: {err}")))?;
        if options.trailing_newline && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    pub fn parse_value(text: &str, label: &str) -> Result<toml::Value, TomlCodecError> {
        text.parse::<toml::Value>()
            .map_err(|err| TomlCodecError::new(format!("Invalid {label} TOML: {err}")))
    }

    pub fn parse_table(
        text: &str,
        label: &str,
    ) -> Result<toml::map::Map<String, toml::Value>, TomlCodecError> {
        match Self::parse_value(text, label)? {
            toml::Value::Table(table) => Ok(table),
            _ => Err(TomlCodecError::new(format!(
                "{label} TOML must be a table."
            ))),
        }
    }
}
