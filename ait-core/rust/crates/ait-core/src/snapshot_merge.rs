use similar::{capture_diff_slices, Algorithm, DiffOp};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextMergeOutcome {
    Merged(Vec<u8>),
    Conflict,
    NonText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TextEdit {
    start: usize,
    end: usize,
    replacement: Vec<String>,
}

pub fn merge_utf8_text_bytes(base: &[u8], target: &[u8], source: &[u8]) -> TextMergeOutcome {
    if base.contains(&0) || target.contains(&0) || source.contains(&0) {
        return TextMergeOutcome::NonText;
    }
    let (Ok(base), Ok(target), Ok(source)) = (
        std::str::from_utf8(base),
        std::str::from_utf8(target),
        std::str::from_utf8(source),
    ) else {
        return TextMergeOutcome::NonText;
    };
    if target == source {
        return TextMergeOutcome::Merged(target.as_bytes().to_vec());
    }
    if target == base {
        return TextMergeOutcome::Merged(source.as_bytes().to_vec());
    }
    if source == base {
        return TextMergeOutcome::Merged(target.as_bytes().to_vec());
    }

    let base_lines = split_preserving_lines(base);
    let target_lines = split_preserving_lines(target);
    let source_lines = split_preserving_lines(source);
    let mut target_edits = diff_edits(&base_lines, &target_lines);
    let source_edits = diff_edits(&base_lines, &source_lines);
    for source_edit in source_edits {
        let mut duplicate = false;
        for target_edit in &target_edits {
            if *target_edit == source_edit {
                duplicate = true;
                break;
            }
            if edits_overlap(target_edit, &source_edit) {
                return TextMergeOutcome::Conflict;
            }
        }
        if !duplicate {
            target_edits.push(source_edit);
        }
    }
    target_edits.sort_by(|left, right| {
        right
            .start
            .cmp(&left.start)
            .then_with(|| right.end.cmp(&left.end))
            .then_with(|| right.replacement.cmp(&left.replacement))
    });
    let mut merged = base_lines;
    for edit in target_edits {
        merged.splice(edit.start..edit.end, edit.replacement);
    }
    TextMergeOutcome::Merged(merged.concat().into_bytes())
}

fn split_preserving_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

fn diff_edits(base: &[String], side: &[String]) -> Vec<TextEdit> {
    capture_diff_slices(Algorithm::Myers, base, side)
        .into_iter()
        .filter_map(|operation| match operation {
            DiffOp::Equal { .. } => None,
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => Some(TextEdit {
                start: old_index,
                end: old_index,
                replacement: side[new_index..new_index + new_len].to_vec(),
            }),
            DiffOp::Delete {
                old_index, old_len, ..
            } => Some(TextEdit {
                start: old_index,
                end: old_index + old_len,
                replacement: Vec::new(),
            }),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => Some(TextEdit {
                start: old_index,
                end: old_index + old_len,
                replacement: side[new_index..new_index + new_len].to_vec(),
            }),
        })
        .collect()
}

fn edits_overlap(left: &TextEdit, right: &TextEdit) -> bool {
    let left_insert = left.start == left.end;
    let right_insert = right.start == right.end;
    match (left_insert, right_insert) {
        (true, true) => left.start == right.start,
        (true, false) => left.start >= right.start && left.start < right.end,
        (false, true) => right.start >= left.start && right.start < left.end,
        (false, false) => left.start < right.end && right.start < left.end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_non_overlapping_line_edits_deterministically() {
        assert_eq!(
            merge_utf8_text_bytes(
                b"one\ntwo\nthree\n",
                b"ONE\ntwo\nthree\n",
                b"one\ntwo\nTHREE\n"
            ),
            TextMergeOutcome::Merged(b"ONE\ntwo\nTHREE\n".to_vec())
        );
    }

    #[test]
    fn deduplicates_equal_edits_and_rejects_overlapping_edits() {
        assert_eq!(
            merge_utf8_text_bytes(b"one\ntwo\n", b"ONE\ntwo\n", b"ONE\ntwo\n"),
            TextMergeOutcome::Merged(b"ONE\ntwo\n".to_vec())
        );
        assert_eq!(
            merge_utf8_text_bytes(b"one\ntwo\n", b"target\ntwo\n", b"source\ntwo\n"),
            TextMergeOutcome::Conflict
        );
        assert_eq!(
            merge_utf8_text_bytes(b"one\n", b"target\none\n", b"source\none\n"),
            TextMergeOutcome::Conflict
        );
    }

    #[test]
    fn classifies_nul_and_invalid_utf8_as_non_text() {
        assert_eq!(
            merge_utf8_text_bytes(b"a\0", b"b\0", b"c\0"),
            TextMergeOutcome::NonText
        );
        assert_eq!(
            merge_utf8_text_bytes(b"a", &[0xff], b"c"),
            TextMergeOutcome::NonText
        );
    }
}
