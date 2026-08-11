//! Markdown item and section extraction remains concrete to plan semantics.
//! Shared foundation already owns generic workflow vocabulary; this module
//! still owns plan-specific parsing and normalization rules.

use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

use crate::workflow_primitives::CheckboxState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub plan_item_ref: String,
    pub text: String,
    pub checkbox_state: CheckboxState,
    pub heading_path: Vec<String>,
    pub line_number: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSectionRef {
    pub plan_ref: String,
    pub heading_title: String,
    pub heading_level: usize,
    pub line_number: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSection {
    pub plan_ref: String,
    pub heading_title: String,
    pub heading_level: usize,
    pub line_number: usize,
    pub section_markdown: String,
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPlanItemSeed {
    pub plan_item_ref: String,
    pub text: String,
    pub checkbox_state: String,
    pub heading_path: Vec<String>,
    pub line_number: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanChecklistCloseoutStatus {
    Updated,
    AlreadyDone,
    Missing,
    Ambiguous,
    NotCheckbox,
}

impl PlanChecklistCloseoutStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::AlreadyDone => "already_done",
            Self::Missing => "missing",
            Self::Ambiguous => "ambiguous",
            Self::NotCheckbox => "not_checkbox",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanChecklistCloseout {
    pub status: PlanChecklistCloseoutStatus,
    pub markdown: String,
    pub line_number: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MarkdownFence {
    marker: u8,
    minimum_length: usize,
}

#[derive(Default)]
struct MarkdownFenceState {
    active: Option<MarkdownFence>,
}

impl MarkdownFenceState {
    fn excludes_line(&mut self, line: &str) -> bool {
        if let Some(active) = self.active {
            if markdown_fence_closes(line, active) {
                self.active = None;
            }
            return true;
        }
        if let Some(opening) = markdown_fence_opening(line) {
            self.active = Some(opening);
            return true;
        }
        false
    }
}

pub fn extract_plan_items(body_markdown: Option<&str>) -> Vec<PlanItem> {
    let mut items = Vec::new();
    let mut heading_path: Vec<String> = Vec::new();
    let mut fence_state = MarkdownFenceState::default();

    for (index, raw_line) in body_markdown.unwrap_or("").lines().enumerate() {
        let line_number = (index + 1) as i64;
        if fence_state.excludes_line(raw_line) {
            continue;
        }

        if let Some(heading) = heading_re().captures(raw_line) {
            let level = heading
                .get(1)
                .map(|value| value.as_str().len())
                .unwrap_or(0);
            let title =
                remove_plan_section_refs(heading.get(2).map(|value| value.as_str()).unwrap_or(""));
            if !title.is_empty() && level > 0 {
                heading_path.truncate(level.saturating_sub(1));
                heading_path.push(title);
            }
            continue;
        }

        let Some(list_item) = markdown_list_item_re().captures(raw_line) else {
            continue;
        };

        let text = list_item
            .name("text")
            .map(|value| value.as_str().trim())
            .unwrap_or("");
        let Some(ref_match) = plan_item_ref_re().captures(text) else {
            continue;
        };
        let plan_item_ref = ref_match
            .get(1)
            .map(|value| value.as_str().trim())
            .unwrap_or("");
        if plan_item_ref.is_empty() {
            continue;
        }
        let display_text = plan_item_ref_re().replace_all(text, "").trim().to_string();
        let checked = list_item.name("checked").map(|value| value.as_str());
        items.push(PlanItem {
            plan_item_ref: plan_item_ref.to_string(),
            text: display_text,
            checkbox_state: CheckboxState::from_markdown_checked(checked),
            heading_path: heading_path.clone(),
            line_number,
        });
    }

    items
}

pub fn list_plan_section_refs(body_markdown: Option<&str>) -> Vec<PlanSectionRef> {
    let mut refs = Vec::new();
    let mut fence_state = MarkdownFenceState::default();

    for (index, raw_line) in body_markdown.unwrap_or("").lines().enumerate() {
        let line_number = index + 1;
        if fence_state.excludes_line(raw_line) {
            continue;
        }
        let Some(heading) = heading_re().captures(raw_line) else {
            continue;
        };
        let Some(ref_match) = plan_section_ref_re()
            .captures(heading.get(2).map(|value| value.as_str()).unwrap_or(""))
        else {
            continue;
        };
        let plan_ref = ref_match
            .get(1)
            .map(|value| value.as_str().trim())
            .unwrap_or("");
        if plan_ref.is_empty() {
            continue;
        }
        refs.push(PlanSectionRef {
            plan_ref: plan_ref.to_string(),
            heading_title: remove_plan_section_refs(
                heading.get(2).map(|value| value.as_str()).unwrap_or(""),
            ),
            heading_level: heading
                .get(1)
                .map(|value| value.as_str().len())
                .unwrap_or(0),
            line_number,
        });
    }

    refs
}

pub fn extract_plan_section(
    body_markdown: Option<&str>,
    plan_ref: Option<&str>,
) -> Option<PlanSection> {
    let normalized_ref = normalize_optional_text(plan_ref?)?;
    let lines: Vec<&str> = body_markdown.unwrap_or("").lines().collect();
    let mut start_index: Option<usize> = None;
    let mut heading_level = 0usize;
    let mut heading_title = String::new();
    let mut fence_state = MarkdownFenceState::default();

    for (index, raw_line) in lines.iter().enumerate() {
        if fence_state.excludes_line(raw_line) {
            continue;
        }
        let Some(heading_match) = heading_re().captures(raw_line) else {
            continue;
        };
        let Some(ref_match) = plan_section_ref_re().captures(
            heading_match
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or(""),
        ) else {
            continue;
        };
        let matched_ref = ref_match
            .get(1)
            .map(|value| value.as_str().trim())
            .unwrap_or("");
        if matched_ref != normalized_ref {
            continue;
        }
        start_index = Some(index);
        heading_level = heading_match
            .get(1)
            .map(|value| value.as_str().len())
            .unwrap_or(0);
        heading_title = remove_plan_section_refs(
            heading_match
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or(""),
        );
        break;
    }

    let start_index = start_index?;
    let mut end_index = lines.len();
    let mut fence_state = MarkdownFenceState::default();
    for (index, line) in lines.iter().enumerate().skip(start_index + 1) {
        if fence_state.excludes_line(line) {
            continue;
        }
        let Some(heading_match) = heading_re().captures(line) else {
            continue;
        };
        if heading_match
            .get(1)
            .map(|value| value.as_str().len())
            .unwrap_or(0)
            <= heading_level
        {
            end_index = index;
            break;
        }
    }

    let section_markdown = lines[start_index..end_index].join("\n").trim().to_string();
    let mut items = extract_plan_items(Some(&section_markdown));
    for item in &mut items {
        item.line_number += start_index as i64;
    }

    Some(PlanSection {
        plan_ref: normalized_ref,
        heading_title,
        heading_level,
        line_number: start_index + 1,
        section_markdown,
        items,
    })
}

pub fn find_plan_item(
    body_markdown: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Option<PlanItem> {
    let normalized_ref = normalize_optional_text(plan_item_ref?)?;
    extract_plan_items(body_markdown)
        .into_iter()
        .find(|item| item.plan_item_ref == normalized_ref)
}

pub fn normalize_plan_items(items: &[NormalizedPlanItemSeed]) -> Result<Vec<PlanItem>, String> {
    let mut normalized_items = Vec::new();
    let mut seen_refs: HashSet<&str> = HashSet::new();

    for item in items {
        if seen_refs.contains(item.plan_item_ref.as_str()) {
            return Err(format!(
                "Duplicate plan_item_ref in plan revision: {}",
                item.plan_item_ref
            ));
        }
        seen_refs.insert(item.plan_item_ref.as_str());

        let Some(checkbox_state) =
            CheckboxState::from_normalized_state(item.checkbox_state.as_str())
        else {
            return Err(format!(
                "Unsupported checkbox_state for plan item {}: {}. Expected open, done, or none.",
                item.plan_item_ref, item.checkbox_state
            ));
        };

        normalized_items.push(PlanItem {
            plan_item_ref: item.plan_item_ref.clone(),
            text: item.text.clone(),
            checkbox_state,
            heading_path: item.heading_path.clone(),
            line_number: item.line_number,
        });
    }

    Ok(normalized_items)
}

pub fn find_plan_item_in_items(
    items: &[NormalizedPlanItemSeed],
    plan_item_ref: Option<&str>,
) -> Result<Option<PlanItem>, String> {
    let Some(normalized_ref) = plan_item_ref.and_then(normalize_optional_text) else {
        return Ok(None);
    };
    Ok(normalize_plan_items(items)?
        .into_iter()
        .find(|item| item.plan_item_ref == normalized_ref))
}

pub fn close_plan_item_checkbox(body_markdown: &str, plan_item_ref: &str) -> PlanChecklistCloseout {
    let Some(normalized_ref) = normalize_optional_text(plan_item_ref) else {
        return PlanChecklistCloseout {
            status: PlanChecklistCloseoutStatus::Missing,
            markdown: body_markdown.to_string(),
            line_number: None,
        };
    };
    let matches = extract_plan_items(Some(body_markdown))
        .into_iter()
        .filter(|item| item.plan_item_ref == normalized_ref)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return PlanChecklistCloseout {
            status: PlanChecklistCloseoutStatus::Missing,
            markdown: body_markdown.to_string(),
            line_number: None,
        };
    }
    if matches.len() > 1 {
        return PlanChecklistCloseout {
            status: PlanChecklistCloseoutStatus::Ambiguous,
            markdown: body_markdown.to_string(),
            line_number: None,
        };
    }
    let item = &matches[0];
    match item.checkbox_state {
        CheckboxState::Done => PlanChecklistCloseout {
            status: PlanChecklistCloseoutStatus::AlreadyDone,
            markdown: body_markdown.to_string(),
            line_number: Some(item.line_number),
        },
        CheckboxState::None => PlanChecklistCloseout {
            status: PlanChecklistCloseoutStatus::NotCheckbox,
            markdown: body_markdown.to_string(),
            line_number: Some(item.line_number),
        },
        CheckboxState::Open => {
            let mut lines = body_markdown
                .split_inclusive('\n')
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !body_markdown.is_empty() && !body_markdown.ends_with('\n') && lines.is_empty() {
                lines.push(body_markdown.to_string());
            }
            let line_index = item.line_number.saturating_sub(1) as usize;
            let Some(line) = lines.get_mut(line_index) else {
                return PlanChecklistCloseout {
                    status: PlanChecklistCloseoutStatus::Missing,
                    markdown: body_markdown.to_string(),
                    line_number: Some(item.line_number),
                };
            };
            let Some(checkbox_match) = open_checkbox_re().find(line) else {
                return PlanChecklistCloseout {
                    status: PlanChecklistCloseoutStatus::NotCheckbox,
                    markdown: body_markdown.to_string(),
                    line_number: Some(item.line_number),
                };
            };
            let marker_start = checkbox_match.end().saturating_sub(3);
            line.replace_range(marker_start..checkbox_match.end(), "[x]");
            PlanChecklistCloseout {
                status: PlanChecklistCloseoutStatus::Updated,
                markdown: lines.concat(),
                line_number: Some(item.line_number),
            }
        }
    }
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let normalized = value.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn remove_plan_section_refs(value: &str) -> String {
    plan_section_ref_re()
        .replace_all(value, "")
        .trim()
        .to_string()
}

fn markdown_fence_opening(line: &str) -> Option<MarkdownFence> {
    let content = strip_markdown_fence_indent(line)?;
    let marker = *content.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let marker_length = content
        .as_bytes()
        .iter()
        .take_while(|candidate| **candidate == marker)
        .count();
    if marker_length < 3 {
        return None;
    }
    if marker == b'`' && content.as_bytes()[marker_length..].contains(&b'`') {
        return None;
    }
    Some(MarkdownFence {
        marker,
        minimum_length: marker_length,
    })
}

fn markdown_fence_closes(line: &str, active: MarkdownFence) -> bool {
    let Some(content) = strip_markdown_fence_indent(line) else {
        return false;
    };
    let marker_length = content
        .as_bytes()
        .iter()
        .take_while(|candidate| **candidate == active.marker)
        .count();
    marker_length >= active.minimum_length
        && content.as_bytes()[marker_length..]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t'))
}

fn strip_markdown_fence_indent(line: &str) -> Option<&str> {
    let leading_spaces = line
        .as_bytes()
        .iter()
        .take_while(|candidate| **candidate == b' ')
        .count();
    (leading_spaces <= 3).then(|| &line[leading_spaces..])
}

fn plan_item_ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\[ref:\s*([A-Za-z0-9][A-Za-z0-9._/-]*)\]").expect("valid ref regex")
    })
}

fn plan_section_ref_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\[plan-ref:\s*([A-Za-z0-9][A-Za-z0-9._/-]*)\]")
            .expect("valid plan-ref regex")
    })
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*$").expect("valid heading regex"))
}

fn markdown_list_item_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:[-*+]|\d+\.)\s+(?:\[(?P<checked>[ xX])\]\s+)?(?P<text>.+?)\s*$")
            .expect("valid list item regex")
    })
}

fn open_checkbox_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*(?:[-*+]|\d+\.)\s+\[ \]").expect("valid open checkbox regex")
    })
}

#[cfg(test)]
mod tests;
