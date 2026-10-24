use super::{
    close_plan_item_checkbox, extract_plan_items, extract_plan_section, find_plan_item,
    find_plan_item_in_items, list_plan_section_refs, normalize_plan_items, NormalizedPlanItemSeed,
    PlanChecklistCloseoutStatus,
};

const SAMPLE_MARKDOWN: &str = "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [x] Tune busy timeout [ref: runtime/busy-timeout]\n- plain list item without ref\n\n### Follow-up Work [plan-ref: runtime/follow-up]\n\n1. Capture retry behavior [ref: runtime/retry-behavior]\n- item without checkbox [ref: runtime/no-checkbox]\n";

#[test]
fn extracts_plan_items_with_checkbox_state_heading_path_and_line_numbers() {
    let items = extract_plan_items(Some(SAMPLE_MARKDOWN));
    assert_eq!(items.len(), 4);

    assert_eq!(items[0].plan_item_ref, "runtime/startup-only-init");
    assert_eq!(items[0].checkbox_state.as_str(), "open");
    assert_eq!(
        items[0].heading_path,
        vec!["Runtime Stability", "Stabilize Runtime Execution Tasks"]
    );
    assert_eq!(items[0].line_number, 5);

    assert_eq!(items[1].plan_item_ref, "runtime/busy-timeout");
    assert_eq!(items[1].checkbox_state.as_str(), "done");
    assert_eq!(
        items[1].heading_path,
        vec!["Runtime Stability", "Stabilize Runtime Execution Tasks"]
    );
    assert_eq!(items[1].line_number, 6);

    assert_eq!(items[2].plan_item_ref, "runtime/retry-behavior");
    assert_eq!(items[2].checkbox_state.as_str(), "none");
    assert_eq!(
        items[2].heading_path,
        vec![
            "Runtime Stability",
            "Stabilize Runtime Execution Tasks",
            "Follow-up Work"
        ]
    );
    assert_eq!(items[2].line_number, 11);

    assert_eq!(items[3].plan_item_ref, "runtime/no-checkbox");
    assert_eq!(items[3].checkbox_state.as_str(), "none");
    assert_eq!(
        items[3].heading_path,
        vec![
            "Runtime Stability",
            "Stabilize Runtime Execution Tasks",
            "Follow-up Work"
        ]
    );
    assert_eq!(items[3].line_number, 12);
}

#[test]
fn lists_plan_section_refs_with_title_level_and_line_numbers() {
    let refs = list_plan_section_refs(Some(SAMPLE_MARKDOWN));
    assert_eq!(refs.len(), 2);

    assert_eq!(refs[0].plan_ref, "runtime/tasks");
    assert_eq!(refs[0].heading_title, "Stabilize Runtime Execution Tasks");
    assert_eq!(refs[0].heading_level, 2);
    assert_eq!(refs[0].line_number, 3);

    assert_eq!(refs[1].plan_ref, "runtime/follow-up");
    assert_eq!(refs[1].heading_title, "Follow-up Work");
    assert_eq!(refs[1].heading_level, 3);
    assert_eq!(refs[1].line_number, 9);
}

#[test]
fn extracts_plan_section_with_rebased_item_line_numbers() {
    let section =
        extract_plan_section(Some(SAMPLE_MARKDOWN), Some("runtime/tasks")).expect("section");

    assert_eq!(section.plan_ref, "runtime/tasks");
    assert_eq!(section.heading_title, "Stabilize Runtime Execution Tasks");
    assert_eq!(section.heading_level, 2);
    assert_eq!(section.line_number, 3);
    assert_eq!(
        section.section_markdown,
        "## Stabilize Runtime Execution Tasks [plan-ref: runtime/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [x] Tune busy timeout [ref: runtime/busy-timeout]\n- plain list item without ref\n\n### Follow-up Work [plan-ref: runtime/follow-up]\n\n1. Capture retry behavior [ref: runtime/retry-behavior]\n- item without checkbox [ref: runtime/no-checkbox]"
    );
    assert_eq!(section.items.len(), 4);
    assert_eq!(section.items[0].line_number, 5);
    assert_eq!(section.items[3].line_number, 12);
}

#[test]
fn fenced_code_examples_do_not_define_plan_sections_or_items() {
    let markdown = "# Real [plan-ref: real/root]\n\
- [ ] Real [ref: real/item]\n\
```markdown\n\
## Backtick example [plan-ref: example/backtick]\n\
- [ ] Duplicate-looking example [ref: after/item]\n\
~~~~\n\
## A different marker does not close the fence [plan-ref: example/still-fenced]\n\
````   \n\
## After [plan-ref: after/root]\n\
- [ ] After [ref: after/item]\n\
   ~~~~ example\n\
### Tilde example [plan-ref: example/tilde]\n\
- [ ] Another duplicate-looking example [ref: after/item]\n\
   ~~~\n\
### A shorter closing fence is still content [plan-ref: example/short-close]\n\
   ~~~~~\n\
- [ ] Tail [ref: after/tail]\n";

    let refs = list_plan_section_refs(Some(markdown));
    assert_eq!(
        refs.iter()
            .map(|section| (section.plan_ref.as_str(), section.line_number))
            .collect::<Vec<_>>(),
        vec![("real/root", 1), ("after/root", 9)]
    );

    let items = extract_plan_items(Some(markdown));
    assert_eq!(
        items
            .iter()
            .map(|item| (item.plan_item_ref.as_str(), item.line_number))
            .collect::<Vec<_>>(),
        vec![("real/item", 2), ("after/item", 10), ("after/tail", 17)]
    );
    assert_eq!(items[2].heading_path, vec!["Real", "After"]);

    let section = extract_plan_section(Some(markdown), Some("after/root")).expect("real section");
    assert_eq!(section.line_number, 9);
    assert_eq!(section.items.len(), 2);
    assert_eq!(section.items[0].line_number, 10);
    assert_eq!(section.items[1].line_number, 17);
    assert!(extract_plan_section(Some(markdown), Some("example/tilde")).is_none());

    let closeout = close_plan_item_checkbox(markdown, "after/item");
    assert_eq!(closeout.status, PlanChecklistCloseoutStatus::Updated);
    assert_eq!(closeout.line_number, Some(10));
    assert!(closeout.markdown.contains("- [x] After [ref: after/item]"));
    assert!(closeout
        .markdown
        .contains("- [ ] Duplicate-looking example [ref: after/item]"));
}

#[test]
fn unclosed_and_over_indented_fences_follow_the_bounded_commonmark_contract() {
    let unclosed = "# Real [plan-ref: real/root]\n\
~~~text\n\
## Hidden [plan-ref: hidden/root]\n\
- [ ] Hidden [ref: hidden/item]\n";
    assert_eq!(list_plan_section_refs(Some(unclosed)).len(), 1);
    assert!(extract_plan_items(Some(unclosed)).is_empty());

    let over_indented = "    ```markdown\n\
## Visible [plan-ref: visible/root]\n\
- [ ] Visible [ref: visible/item]\n";
    assert_eq!(list_plan_section_refs(Some(over_indented)).len(), 1);
    assert_eq!(extract_plan_items(Some(over_indented)).len(), 1);
}

#[test]
fn finds_plan_item_in_markdown_with_normalized_lookup_ref() {
    let item = find_plan_item(Some(SAMPLE_MARKDOWN), Some(" runtime/busy-timeout "))
        .expect("matching item");

    assert_eq!(item.plan_item_ref, "runtime/busy-timeout");
    assert_eq!(item.text, "Tune busy timeout");
    assert_eq!(item.checkbox_state.as_str(), "done");
    assert_eq!(
        item.heading_path,
        vec!["Runtime Stability", "Stabilize Runtime Execution Tasks"]
    );
    assert_eq!(item.line_number, 6);
    assert!(find_plan_item(Some(SAMPLE_MARKDOWN), Some("runtime/missing")).is_none());
    assert!(find_plan_item(Some(SAMPLE_MARKDOWN), Some("   ")).is_none());
}

#[test]
fn normalize_plan_items_rejects_duplicate_refs() {
    let err = normalize_plan_items(&[
        NormalizedPlanItemSeed {
            plan_item_ref: "runtime/a".to_string(),
            text: "A".to_string(),
            checkbox_state: "open".to_string(),
            heading_path: vec![],
            line_number: 1,
        },
        NormalizedPlanItemSeed {
            plan_item_ref: "runtime/a".to_string(),
            text: "B".to_string(),
            checkbox_state: "done".to_string(),
            heading_path: vec![],
            line_number: 2,
        },
    ])
    .expect_err("duplicate refs should fail");

    assert_eq!(err, "Duplicate plan_item_ref in plan revision: runtime/a");
}

#[test]
fn normalize_plan_items_rejects_invalid_checkbox_states() {
    let err = normalize_plan_items(&[NormalizedPlanItemSeed {
        plan_item_ref: "runtime/a".to_string(),
        text: "A".to_string(),
        checkbox_state: "invalid".to_string(),
        heading_path: vec![],
        line_number: 1,
    }])
    .expect_err("invalid checkbox states should fail");

    assert_eq!(
        err,
        "Unsupported checkbox_state for plan item runtime/a: invalid. Expected open, done, or none."
    );
}

#[test]
fn finds_plan_item_in_normalized_items_and_preserves_validation() {
    let items = [
        NormalizedPlanItemSeed {
            plan_item_ref: "runtime/a".to_string(),
            text: "A".to_string(),
            checkbox_state: "open".to_string(),
            heading_path: vec!["Runtime".to_string()],
            line_number: 3,
        },
        NormalizedPlanItemSeed {
            plan_item_ref: "runtime/b".to_string(),
            text: "B".to_string(),
            checkbox_state: "none".to_string(),
            heading_path: vec!["Runtime".to_string()],
            line_number: 4,
        },
    ];

    let item = find_plan_item_in_items(&items, Some("runtime/b"))
        .expect("valid items")
        .expect("matching item");
    assert_eq!(item.plan_item_ref, "runtime/b");
    assert_eq!(item.checkbox_state.as_str(), "none");
    assert_eq!(item.line_number, 4);
    assert!(find_plan_item_in_items(&items, Some("runtime/missing"))
        .expect("valid items")
        .is_none());
    assert!(find_plan_item_in_items(&items, Some("   "))
        .expect("blank ref returns none")
        .is_none());

    let err = find_plan_item_in_items(
        &[NormalizedPlanItemSeed {
            plan_item_ref: "runtime/a".to_string(),
            text: "A".to_string(),
            checkbox_state: "invalid".to_string(),
            heading_path: vec![],
            line_number: 1,
        }],
        Some("runtime/a"),
    )
    .expect_err("invalid items should still fail");
    assert_eq!(
        err,
        "Unsupported checkbox_state for plan item runtime/a: invalid. Expected open, done, or none."
    );
}

#[test]
fn closes_only_the_exact_open_plan_item_checkbox_and_preserves_layout() {
    let markdown =
        "# Sprint\r\n\r\n- [ ] First [ref: sprint/first]\r\n- [ ] Second [ref: sprint/second]\r\n";
    let closeout = close_plan_item_checkbox(markdown, "sprint/second");
    assert_eq!(closeout.status, PlanChecklistCloseoutStatus::Updated);
    assert_eq!(closeout.line_number, Some(4));
    assert_eq!(
        closeout.markdown,
        "# Sprint\r\n\r\n- [ ] First [ref: sprint/first]\r\n- [x] Second [ref: sprint/second]\r\n"
    );
}

#[test]
fn checklist_closeout_is_idempotent_and_rejects_unsafe_matches() {
    let done = "- [X] Done [ref: sprint/done]\n";
    assert_eq!(
        close_plan_item_checkbox(done, "sprint/done").status,
        PlanChecklistCloseoutStatus::AlreadyDone
    );
    let non_checkbox = "- Note [ref: sprint/note]\n";
    assert_eq!(
        close_plan_item_checkbox(non_checkbox, "sprint/note").status,
        PlanChecklistCloseoutStatus::NotCheckbox
    );
    let duplicate = "- [ ] First [ref: sprint/dup]\n- [ ] Second [ref: sprint/dup]\n";
    let ambiguous = close_plan_item_checkbox(duplicate, "sprint/dup");
    assert_eq!(ambiguous.status, PlanChecklistCloseoutStatus::Ambiguous);
    assert_eq!(ambiguous.markdown, duplicate);
    assert_eq!(
        close_plan_item_checkbox(duplicate, "sprint/missing").status,
        PlanChecklistCloseoutStatus::Missing
    );
}
