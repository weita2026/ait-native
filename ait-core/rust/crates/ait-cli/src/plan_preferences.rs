use crate::json_support::parse_value;
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonMap, JsonValue};

pub(crate) const DEFAULT_LANGUAGE: &str = "en";
pub(crate) const DEFAULT_STYLE: &str = "concise";

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PlanPreferences {
    pub language: String,
    pub style: String,
    language_source: &'static str,
    style_source: &'static str,
}

impl PlanPreferences {
    pub fn from_config(config: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let (language, language_source) = setting(
            config,
            "plan_language",
            DEFAULT_LANGUAGE,
            normalize_language,
        )?;
        let (style, style_source) = setting(config, "plan_style", DEFAULT_STYLE, normalize_style)?;
        Ok(Self {
            language,
            style,
            language_source,
            style_source,
        })
    }

    pub fn for_repo(repo: &RepoRuntime) -> Result<Self, String> {
        // Worktree overlays are intentionally not an authority for these
        // repository-owned preferences, even when they contain the same keys.
        if repo.worktree_config_path.is_some() || repo.authoritative_repo_root() != repo.root {
            let path = repo.authoritative_repo_root().join(".ait/config.json");
            let text = std::fs::read_to_string(&path).map_err(|e| {
                format!(
                    "Failed to read Plan preferences from {}: {e}",
                    path.display()
                )
            })?;
            let value = parse_value(&text, "Invalid repository Plan preferences")?;
            let config = value
                .as_object()
                .ok_or("Repository config must be an object.")?;
            Self::from_config(config)
        } else {
            Self::from_config(&repo.config)
        }
    }

    pub fn language_summary(&self) -> JsonValue {
        json!({"value": self.language, "source": self.language_source})
    }

    pub fn style_summary(&self) -> JsonValue {
        json!({"value": self.style, "source": self.style_source})
    }

    pub fn guidance(&self) -> String {
        format!(
            "Plan prose defaults: language=`{}` (BCP 47); style=`{}`.\n\
Write new Plan and sprint prose in the specified language and style.\n\
Concise means short sentences without repetition; detailed includes useful\n\
background and rationale. Always retain goals, scope, necessary decisions,\n\
acceptance criteria, and verification. Keep commands, paths, identifiers,\n\
and binding markers unchanged. Preserve an existing document's language\n\
unless translation is requested. Explicit user instructions take precedence.",
            self.language, self.style
        )
    }
}

fn setting(
    config: &JsonMap<String, JsonValue>,
    key: &str,
    default: &str,
    normalize: fn(&str) -> Result<String, String>,
) -> Result<(String, &'static str), String> {
    match config.get(key) {
        None => Ok((default.to_string(), "built_in")),
        Some(JsonValue::String(value)) => normalize(value)
            .map(|value| (value, "repo_config"))
            .map_err(|e| format!("Invalid config.{key}: {e}")),
        Some(_) => Err(format!("config.{key} must be a string.")),
    }
}

/// The public language[-Script][-REGION] subset of BCP 47. This checks
/// syntax and casing only; it does not infer scripts or consult a catalog.
pub(crate) fn normalize_language(value: &str) -> Result<String, String> {
    let error = || {
        "Plan language must use language[-Script][-REGION], for example zh-TW, zh-Hant-TW, en, ja-JP, or ko-KR.".to_string()
    };
    let mut parts = value.split('-');
    let language = parts.next().ok_or_else(error)?;
    if !(2..=3).contains(&language.len()) || !ascii_letters(language) {
        return Err(error());
    }
    let mut normalized = language.to_ascii_lowercase();
    let mut next = parts.next();
    if let Some(script) = next.filter(|part| part.len() == 4 && ascii_letters(part)) {
        normalized.push('-');
        normalized.push_str(&script[..1].to_ascii_uppercase());
        normalized.push_str(&script[1..].to_ascii_lowercase());
        next = parts.next();
    }
    if let Some(region) = next {
        if !((region.len() == 2 && ascii_letters(region))
            || (region.len() == 3 && region.bytes().all(|b| b.is_ascii_digit())))
        {
            return Err(error());
        }
        normalized.push('-');
        normalized.push_str(&region.to_ascii_uppercase());
    }
    if parts.next().is_some() {
        return Err(error());
    }
    Ok(normalized)
}

fn ascii_letters(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_alphabetic())
}

pub(crate) fn normalize_style(value: &str) -> Result<String, String> {
    match value {
        "concise" | "detailed" => Ok(value.to_string()),
        _ => Err("Plan style must be exactly concise or detailed.".to_string()),
    }
}

fn c_locale(value: &str) -> bool {
    matches!(value.split('.').next(), Some("C" | "POSIX"))
}

fn normalize_os_locale(value: &str) -> Option<String> {
    if c_locale(value) {
        return Some(DEFAULT_LANGUAGE.to_string());
    }
    let (locale, modifier) = match value.split_once('@') {
        Some((locale, modifier)) => (locale, Some(modifier)),
        None => (value, None),
    };
    let language = normalize_language(&locale.split('.').next()?.replace('_', "-")).ok()?;
    let Some(modifier) = modifier else {
        return Some(language);
    };
    let script = match modifier {
        "latin" => "Latn",
        "cyrillic" => "Cyrl",
        _ => return None,
    };
    let mut parts = language.split('-');
    let primary = parts.next()?;
    let tail: Vec<_> = parts.collect();
    if tail.first().is_some_and(|part| part.len() == 4) {
        return (tail[0] == script).then_some(language);
    }
    Some(if tail.is_empty() {
        format!("{primary}-{script}")
    } else {
        format!("{primary}-{script}-{}", tail[0])
    })
}

#[cfg(any(target_os = "linux", test))]
fn linux_language(mut env: impl FnMut(&str) -> Option<String>) -> String {
    let base = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(&mut env)
        .find(|value| !value.is_empty());
    let Some(base) = base else {
        return DEFAULT_LANGUAGE.to_string();
    };
    if c_locale(&base) {
        return DEFAULT_LANGUAGE.to_string();
    }
    let Some(base) = normalize_os_locale(&base) else {
        return DEFAULT_LANGUAGE.to_string();
    };
    env("LANGUAGE")
        .and_then(|list| list.split(':').find_map(normalize_os_locale))
        .unwrap_or(base)
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn preferred_language(languages: &[String]) -> String {
    languages
        .iter()
        .find_map(|value| normalize_os_locale(value))
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string())
}

pub(crate) fn detect_language() -> String {
    #[cfg(target_os = "linux")]
    {
        linux_language(|key| std::env::var(key).ok())
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        preferred_language(&native_preferred_languages())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        DEFAULT_LANGUAGE.to_string()
    }
}

#[cfg(target_os = "windows")]
fn native_preferred_languages() -> Vec<String> {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let mut count = 0;
    let mut length = 0;
    // The first call obtains capacity; the second fills a caller-owned UTF-16
    // MULTI_SZ buffer. Only the successful call's reported range is decoded.
    unsafe {
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        ) == 0
            || !(2..=65536).contains(&length)
        {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            buffer.as_mut_ptr(),
            &mut length,
        ) == 0
            || length as usize > buffer.len()
        {
            return Vec::new();
        }
        decode_windows_languages(&buffer[..length as usize])
    }
}

#[cfg(any(target_os = "windows", test))]
fn decode_windows_languages(buffer: &[u16]) -> Vec<String> {
    if !buffer.ends_with(&[0, 0]) {
        return Vec::new();
    }
    buffer
        .split(|c| *c == 0)
        .take_while(|part| !part.is_empty())
        .filter_map(|part| String::from_utf16(part).ok())
        .collect()
}

#[cfg(target_os = "macos")]
fn native_preferred_languages() -> Vec<String> {
    use core_foundation_sys::{
        array::{CFArrayGetCount, CFArrayGetValueAtIndex},
        base::CFRelease,
        locale::CFLocaleCopyPreferredLanguages,
        string::{
            kCFStringEncodingUTF8, CFStringGetCString, CFStringGetLength,
            CFStringGetMaximumSizeForEncoding, CFStringRef,
        },
    };
    let mut values = Vec::new();
    // The Copy rule gives ownership of the array; its documented CFString
    // elements stay borrowed until the single CFRelease below.
    unsafe {
        let array = CFLocaleCopyPreferredLanguages();
        if array.is_null() {
            return values;
        }
        for index in 0..CFArrayGetCount(array) {
            let value = CFArrayGetValueAtIndex(array, index) as CFStringRef;
            let capacity =
                CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), kCFStringEncodingUTF8)
                    .checked_add(1);
            let Some(capacity) = capacity.filter(|size| *size > 0 && *size <= 65536) else {
                continue;
            };
            let mut buffer = vec![0u8; capacity as usize];
            if CFStringGetCString(
                value,
                buffer.as_mut_ptr().cast(),
                capacity,
                kCFStringEncodingUTF8,
            ) != 0
            {
                if let Ok(text) = std::ffi::CStr::from_ptr(buffer.as_ptr().cast()).to_str() {
                    values.push(text.to_string());
                }
            }
        }
        CFRelease(array.cast());
    }
    values
}

#[cfg(test)]
mod tests;
