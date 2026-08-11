use super::*;
use serde::Deserialize;
use std::collections::BTreeSet;

#[test]
fn server_plan_schema_matches_ait_core_compact_v1_layout() {
    // Companion core fixtures live in ait-core under:
    // - rust/crates/ait-core/src/plan_binary_db/schema/codec.rs
    // - rust/crates/ait-core/src/plan_binary_db/tests.rs
    //
    // This server test intentionally does not import, shell out to, or
    // require a checked-out ait-core path.
    assert_eq!(plan_file().as_str(), super::super::schema::PLAN_BIN);
    assert_eq!(
        plan_payload_file().as_str(),
        super::super::schema::PLAN_PAYLOAD_BIN
    );
    assert_eq!(
        plan_revision_file().as_str(),
        super::super::schema::PLAN_REVISION_BIN
    );
    assert_eq!(
        plan_revision_payload_file().as_str(),
        super::super::schema::PLAN_REVISION_PAYLOAD_BIN
    );
    assert_eq!(
        plan_item_file().as_str(),
        super::super::schema::PLAN_ITEM_BIN
    );
    assert_eq!(
        plan_item_payload_file().as_str(),
        super::super::schema::PLAN_ITEM_PAYLOAD_BIN
    );
    assert_eq!(plan_file().record_size(), 48);
    assert_eq!(plan_revision_file().record_size(), 56);
    assert_eq!(plan_item_file().record_size(), 16);
    for file in [plan_file(), plan_revision_file(), plan_item_file()] {
        assert_eq!(file.family(), BinaryDbFileFamily::Plan);
    }
    for payload in [
        plan_payload_file(),
        plan_revision_payload_file(),
        plan_item_payload_file(),
    ] {
        assert_eq!(payload.family(), BinaryDbFileFamily::Plan);
    }
    assert_eq!(
        super::super::schema::plan_file_for(PLAN_LAYOUT_ID).unwrap(),
        plan_file()
    );
    assert_eq!(
        super::super::schema::plan_revision_file_for(PLAN_LAYOUT_ID).unwrap(),
        plan_revision_file()
    );
    assert_eq!(
        super::super::schema::plan_item_file_for(PLAN_LAYOUT_ID).unwrap(),
        plan_item_file()
    );
    let error = super::super::schema::plan_file_for(UNSUPPORTED_TEST_LAYOUT)
        .expect_err("unsupported layout should fail closed");
    assert!(error.contains(&format!(
        "unsupported compact Plan Binary DB layout {UNSUPPORTED_TEST_LAYOUT}"
    )));
}

#[test]
fn server_plan_status_uses_state_bits_and_preserves_published_flags() {
    let mut record = PlanRecord {
        plan_meta: 0b0000_0100,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 0,
        published_plan_index_plus1: 1,
        published_latest_revision_index_plus1: 1,
        created_at_s: 0,
        updated_at_s: 0,
        published_at_s: 0,
    };
    assert_eq!(
        plan_status_from_record(&record).expect("published draft should decode"),
        "draft"
    );

    record.plan_meta =
        plan_meta_with_status(record.plan_meta, "archived").expect("archived state should encode");
    assert_eq!(record.plan_meta, 0b0000_0101);
    assert_eq!(
        plan_status_from_record(&record).expect("published archived should decode"),
        "archived"
    );

    record.plan_meta = plan_meta_with_status(record.plan_meta, "superseded")
        .expect("superseded state should encode");
    assert_eq!(record.plan_meta, 0b0000_0110);
    assert_eq!(
        plan_status_from_record(&record).expect("published superseded should decode"),
        "superseded"
    );
}

#[test]
fn server_plan_compact_v1_golden_fixture_matches_core_wire_contract() {
    let plan = PlanRecord {
        plan_meta: PLAN_STATE_ARCHIVED_META,
        reserved0: 0,
        payload_len: 5,
        payload_offset: 0x0102_0304_0506_0708,
        latest_revision_index_plus1: 9,
        published_plan_index_plus1: 10,
        published_latest_revision_index_plus1: 11,
        created_at_s: 12,
        updated_at_s: 13,
        published_at_s: 14,
    };
    let plan_bytes = super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::encode_record(&plan)
        .expect("compact v1 plan record should encode");
    assert_eq!(plan_bytes.len(), PLAN_RECORD_SIZE as usize);
    assert_eq!(
        &plan_bytes[..24],
        &[1, 0, 5, 0, 8, 7, 6, 5, 4, 3, 2, 1, 9, 0, 0, 0, 10, 0, 0, 0, 11, 0, 0, 0]
    );
    assert_eq!(&plan_bytes[24..32], &12_u64.to_le_bytes());
    assert_eq!(&plan_bytes[32..40], &13_u64.to_le_bytes());
    assert_eq!(&plan_bytes[40..48], &14_u64.to_le_bytes());
    assert_eq!(
        super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_record(&plan_bytes)
            .expect("compact v1 plan record should decode"),
        plan
    );

    let revision = PlanRevisionRecord {
        revision_meta: 2,
        reserved0: 0,
        payload_len: 7,
        revision_number: 3,
        item_count: 4,
        payload_offset: 0x1112_1314_1516_1718,
        plan_index: 5,
        previous_revision_index_plus1: 6,
        item_start_index: 7,
        published_revision_index_plus1: 8,
        root_tree_pack_index_plus1: 9,
        root_entry_ordinal: 10,
        created_at_s: 11,
        published_at_s: 12,
    };
    let revision_bytes =
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_record(&revision)
            .expect("compact v1 plan revision record should encode");
    assert_eq!(revision_bytes.len(), PLAN_REVISION_RECORD_SIZE as usize);
    assert_eq!(
        &revision_bytes[..40],
        &[
            2, 0, 7, 0, 3, 0, 4, 0, 24, 23, 22, 21, 20, 19, 18, 17, 5, 0, 0, 0, 6, 0, 0, 0, 7, 0,
            0, 0, 8, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0
        ]
    );
    assert_eq!(&revision_bytes[40..48], &11_u64.to_le_bytes());
    assert_eq!(&revision_bytes[48..56], &12_u64.to_le_bytes());
    assert_eq!(
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(
            &revision_bytes,
        )
        .expect("compact v1 plan revision record should decode"),
        revision
    );

    let item_record = PlanItemRecord {
        item_meta: ITEM_STATE_OPEN_META | ITEM_HAS_REF_META | ITEM_TASKABLE_HINT_META,
        reserved0: 0,
        payload_len: 13,
        payload_offset: 21,
        line_number: 34,
    };
    let item_record_bytes =
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_record(&item_record)
            .expect("compact v1 plan item record should encode");
    assert_eq!(
        item_record_bytes,
        vec![13, 0, 13, 0, 21, 0, 0, 0, 0, 0, 0, 0, 34, 0, 0, 0],
        "compact v1 plan_item.bin record bytes changed"
    );
    assert_eq!(
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(
            &item_record_bytes,
        )
        .expect("compact v1 plan item record should decode"),
        item_record
    );

    let revision_payload = PlanRevisionPayload {
        title_snapshot: "Title".to_string(),
        summary: "Sum".to_string(),
        artifact_path: "docs/a.md".to_string(),
        artifact_selector: "root".to_string(),
        artifact_heading: "Heading".to_string(),
        artifact_blob_id: "BLB-123".to_string(),
    };
    let revision_payload_bytes =
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_payload(
            &revision_payload,
        )
        .expect("compact v1 plan revision payload should encode");
    assert_eq!(
        revision_payload_bytes,
        vec![
            5, 0, 3, 0, 9, 0, 4, 0, 7, 0, 84, 105, 116, 108, 101, 83, 117, 109, 100, 111, 99, 115,
            47, 97, 46, 109, 100, 114, 111, 111, 116, 72, 101, 97, 100, 105, 110, 103, 66, 76, 66,
            45, 49, 50, 51,
        ],
        "compact v1 plan_revision_payload.bin bytes changed"
    );
    assert_eq!(
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(
            &revision_payload_bytes,
        )
        .expect("compact v1 plan revision payload should decode"),
        revision_payload
    );

    let item_payload = PlanItemPayload {
        plan_item_ref: "SBDB/PARITY-11".to_string(),
        text: "Exact ref".to_string(),
        heading_path: vec!["Binary DB".to_string(), "Fixtures".to_string()],
    };
    let item_payload_bytes =
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_payload(&item_payload)
            .expect("compact v1 plan item payload should encode");
    assert_eq!(
        item_payload_bytes,
        vec![
            14, 0, 9, 0, 2, 0, 83, 66, 68, 66, 47, 80, 65, 82, 73, 84, 89, 45, 49, 49, 69, 120, 97,
            99, 116, 32, 114, 101, 102, 9, 0, 66, 105, 110, 97, 114, 121, 32, 68, 66, 8, 0, 70,
            105, 120, 116, 117, 114, 101, 115,
        ],
        "compact v1 plan_item_payload.bin bytes changed"
    );
    assert_eq!(
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(
            &item_payload_bytes,
        )
        .expect("compact v1 plan item payload should decode"),
        item_payload
    );
}

#[test]
fn server_plan_codecs_are_layout_scoped_and_preserve_v1_bytes() {
    let plan = PlanRecord {
        plan_meta: PLAN_STATE_DRAFT_META,
        reserved0: 0,
        payload_len: 5,
        payload_offset: 0x0102_0304_0506_0708,
        latest_revision_index_plus1: 9,
        published_plan_index_plus1: 10,
        published_latest_revision_index_plus1: 11,
        created_at_s: 12,
        updated_at_s: 13,
        published_at_s: 14,
    };
    let plan_bytes = super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::encode_record(&plan)
        .expect("plan record should encode");
    assert_eq!(
        plan_bytes,
        vec![
            0, 0, 5, 0, 8, 7, 6, 5, 4, 3, 2, 1, 9, 0, 0, 0, 10, 0, 0, 0, 11, 0, 0, 0, 12, 0, 0, 0,
            0, 0, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0, 0,
        ]
    );
    assert_eq!(
        super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_record(&plan_bytes)
            .expect("plan record should decode"),
        plan
    );

    let revision = PlanRevisionRecord {
        revision_meta: 2,
        reserved0: 0,
        payload_len: 7,
        revision_number: 3,
        item_count: 4,
        payload_offset: 0x1112_1314_1516_1718,
        plan_index: 5,
        previous_revision_index_plus1: 6,
        item_start_index: 7,
        published_revision_index_plus1: 8,
        root_tree_pack_index_plus1: 9,
        root_entry_ordinal: 10,
        created_at_s: 11,
        published_at_s: 12,
    };
    let revision_bytes =
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_record(&revision)
            .expect("revision record should encode");
    assert_eq!(revision_bytes.len(), PLAN_REVISION_RECORD_SIZE as usize);
    assert_eq!(
        super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(
            &revision_bytes,
        )
        .expect("revision record should decode"),
        revision
    );

    let item_record = PlanItemRecord {
        item_meta: ITEM_STATE_OPEN_META | ITEM_HAS_REF_META | ITEM_TASKABLE_HINT_META,
        reserved0: 0,
        payload_len: 13,
        payload_offset: 21,
        line_number: 34,
    };
    let item_record_bytes =
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_record(&item_record)
            .expect("item record should encode");
    assert_eq!(item_record_bytes.len(), PLAN_ITEM_RECORD_SIZE as usize);
    assert_eq!(
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(
            &item_record_bytes,
        )
        .expect("item record should decode"),
        item_record
    );

    let unsupported =
        super::super::codec::ServerPlanCodec::<UNSUPPORTED_TEST_LAYOUT>::decode_record(
            &[0; PLAN_RECORD_SIZE as usize],
        )
        .expect_err("unsupported layout codec should fail closed");
    assert!(unsupported.contains(&format!(
        "unsupported server Plan Binary DB codec layout {UNSUPPORTED_TEST_LAYOUT}"
    )));
}

#[test]
fn server_plan_item_payload_codec_round_trips_exact_ref_text_and_heading() {
    let payload = PlanItemPayload {
        plan_item_ref: "SBDB-PARITY-09/codec-shape".to_string(),
        text: "Use exact ref bytes".to_string(),
        heading_path: vec!["Binary DB".to_string(), "Codec".to_string()],
    };

    let encoded =
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_payload(&payload)
            .expect("item payload should encode");
    assert_eq!(
        &encoded[..6],
        &[
            payload.plan_item_ref.len() as u8,
            0,
            payload.text.len() as u8,
            0,
            payload.heading_path.len() as u8,
            0,
        ]
    );
    assert_eq!(
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&encoded)
            .expect("item payload should decode"),
        payload
    );

    let truncated = super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&[
        1, 0, 0, 0, 0, 0,
    ])
    .expect_err("truncated ref bytes should fail");
    assert!(truncated.contains("plan_item_ref_bytes bytes are truncated"));

    let invalid_utf8 =
        super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&[
            1, 0, 0, 0, 0, 0, 0xff,
        ])
        .expect_err("invalid UTF-8 should fail");
    assert!(invalid_utf8.contains("plan_item_ref is not valid UTF-8"));
}

#[derive(Debug, Deserialize)]
struct PlanGoldenFixture {
    version: String,
    layout_id: u32,
    cases: Vec<PlanGoldenCase>,
}

#[derive(Debug, Deserialize)]
struct PlanGoldenCase {
    id: String,
    kind: String,
    input: JsonValue,
    expected_bytes: Vec<u8>,
}

fn golden_u64(input: &JsonValue, field: &str) -> u64 {
    input[field]
        .as_u64()
        .unwrap_or_else(|| panic!("golden field {field} must be u64"))
}

fn golden_u32(input: &JsonValue, field: &str) -> u32 {
    u32::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u32"))
}

fn golden_u16(input: &JsonValue, field: &str) -> u16 {
    u16::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u16"))
}

fn golden_u8(input: &JsonValue, field: &str) -> u8 {
    u8::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u8"))
}

fn golden_hex_u64(input: &JsonValue, field: &str) -> u64 {
    u64::from_str_radix(
        input[field]
            .as_str()
            .unwrap_or_else(|| panic!("golden field {field} must be hex text")),
        16,
    )
    .unwrap_or_else(|_| panic!("golden field {field} must be valid u64 hex"))
}

fn golden_text(input: &JsonValue, field: &str) -> String {
    input[field]
        .as_str()
        .unwrap_or_else(|| panic!("golden field {field} must be text"))
        .to_string()
}

#[test]
fn server_plan_binary_db_complete_golden_fixture_matches_core_wire_contract() {
    let fixture: PlanGoldenFixture = serde_json::from_slice(SERVER_BINARY_DB_PLAN_GOLDEN_SOURCE)
        .expect("server Plan golden fixture must parse");
    assert_eq!(fixture.version, SERVER_BINARY_DB_PLAN_GOLDEN_VERSION);
    assert_eq!(fixture.layout_id, PLAN_LAYOUT_ID);
    assert_eq!(
        server_binary_db_plan_golden_checksum(),
        SERVER_BINARY_DB_PLAN_GOLDEN_CHECKSUM
    );

    let mut executed = BTreeSet::new();
    for case in fixture.cases {
        assert!(executed.insert(case.id.clone()), "duplicate {}", case.id);
        let input = &case.input;
        match case.kind.as_str() {
            "plan_record" => {
                let value = PlanRecord {
                    plan_meta: golden_u8(input, "plan_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    latest_revision_index_plus1: golden_u32(input, "latest_revision_index_plus1"),
                    published_plan_index_plus1: golden_u32(input, "published_plan_index_plus1"),
                    published_latest_revision_index_plus1: golden_u32(
                        input,
                        "published_latest_revision_index_plus1",
                    ),
                    created_at_s: golden_u64(input, "created_at_s"),
                    updated_at_s: golden_u64(input, "updated_at_s"),
                    published_at_s: golden_u64(input, "published_at_s"),
                };
                assert_eq!(
                    super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::encode_record(&value)
                        .expect("encode plan"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_record(
                        &case.expected_bytes,
                    )
                    .expect("decode plan golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_payload" => {
                let value = golden_text(input, "title");
                assert_eq!(value.as_bytes(), case.expected_bytes, "{} encode", case.id);
                assert_eq!(
                    super::super::codec::ServerPlanCodec::<PLAN_LAYOUT_ID>::decode_title_payload(
                        case.expected_bytes,
                    )
                    .expect("decode title golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_revision_record" => {
                let revision = PlanRevisionRecord {
                    revision_meta: golden_u8(input, "revision_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    revision_number: golden_u16(input, "revision_number"),
                    item_count: golden_u16(input, "item_count"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    plan_index: golden_u32(input, "plan_index"),
                    previous_revision_index_plus1: golden_u32(
                        input,
                        "previous_revision_index_plus1",
                    ),
                    item_start_index: golden_u32(input, "item_start_index"),
                    published_revision_index_plus1: golden_u32(
                        input,
                        "published_revision_index_plus1",
                    ),
                    root_tree_pack_index_plus1: golden_u32(input, "root_tree_pack_index_plus1"),
                    root_entry_ordinal: golden_u32(input, "root_entry_ordinal"),
                    created_at_s: golden_u64(input, "created_at_s"),
                    published_at_s: golden_u64(input, "published_at_s"),
                };
                assert_eq!(
                    super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_record(
                        &revision,
                    )
                    .expect("encode revision"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(
                        &case.expected_bytes,
                    )
                    .expect("decode revision golden"),
                    revision,
                    "{} decode",
                    case.id
                );
            }
            "plan_revision_payload" => {
                let value = PlanRevisionPayload {
                    title_snapshot: golden_text(input, "title_snapshot"),
                    summary: golden_text(input, "summary"),
                    artifact_path: golden_text(input, "artifact_path"),
                    artifact_selector: golden_text(input, "artifact_selector"),
                    artifact_heading: golden_text(input, "artifact_heading"),
                    artifact_blob_id: golden_text(input, "artifact_blob_id"),
                };
                assert_eq!(
                    super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_payload(
                        &value,
                    )
                    .expect("encode revision payload"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    super::super::codec::ServerPlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(
                        &case.expected_bytes,
                    )
                    .expect("decode revision payload golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_item_record" => {
                let value = PlanItemRecord {
                    item_meta: golden_u8(input, "item_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    line_number: golden_u32(input, "line_number"),
                };
                assert_eq!(
                    super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_record(
                        &value,
                    )
                    .expect("encode item"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(
                        &case.expected_bytes,
                    )
                    .expect("decode item golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_item_payload" => {
                let value = PlanItemPayload {
                    plan_item_ref: golden_text(input, "plan_item_ref"),
                    text: golden_text(input, "text"),
                    heading_path: input["heading_path"]
                        .as_array()
                        .expect("heading_path must be an array")
                        .iter()
                        .map(|part| {
                            part.as_str()
                                .expect("heading_path entry must be text")
                                .to_string()
                        })
                        .collect(),
                };
                assert_eq!(
                    super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::encode_payload(
                        &value,
                    )
                    .expect("encode item payload"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    super::super::codec::ServerPlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(
                        &case.expected_bytes,
                    )
                    .expect("decode item payload golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            kind => panic!("unsupported Plan golden case kind {kind}"),
        }
    }

    assert_eq!(
        executed,
        BTreeSet::from([
            "plan_record".to_string(),
            "plan_payload".to_string(),
            "plan_revision_record".to_string(),
            "plan_revision_payload".to_string(),
            "plan_item_record".to_string(),
            "plan_item_payload".to_string(),
        ])
    );
}
