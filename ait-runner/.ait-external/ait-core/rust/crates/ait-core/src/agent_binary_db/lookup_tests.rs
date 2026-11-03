use super::layout::read_rows;
use super::*;
use crate::binary_db::BinaryDb;
use std::fs;
fn initialized() -> (tempfile::TempDir, LocalBinaryDbFs) {
    let (temp, db) = super::tests::fixture();
    ensure_default(&db, [0, 1, 2]).unwrap();
    (temp, db)
}
fn roots(db: &LocalBinaryDbFs) -> Vec<Vec<u8>> {
    [
        "agents.bin",
        "agents_payload.bin",
        "agent_roles.bin",
        "agent_teams.bin",
        "agent_teams_payload.bin",
        "agent_team_membership.bin",
    ]
    .iter()
    .map(|f| fs::read(db.authority_root().as_path().join(f)).unwrap_or_default())
    .collect()
}
fn legacy(db: &LocalBinaryDbFs) {
    let read = BinaryDbReadTxn::new(db);
    let rows = read_rows(&read, members_file()).unwrap();
    let mut bytes = 1u32.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend(&row[..12]);
    }
    let mut team = read.read_record(teams_file(), 0).unwrap();
    team[0] = 0;
    drop(read);
    let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General).unwrap();
    tx.commit_with_atomic_file_replacement(legacy_members_file(), &bytes, "legacy-fixture")
        .unwrap();
    drop(tx);
    let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General).unwrap();
    let mut bytes = 1u32.to_le_bytes().to_vec();
    bytes.extend(team);
    tx.commit_with_atomic_file_replacement(teams_file(), &bytes, "legacy-team")
        .unwrap();
}
#[test]
fn legacy_conversion_preserves_physical_keys_bindings_and_exact_retry() {
    let (_temp, db) = initialized();
    bind(&db, 2, "", "existing-curie").unwrap();
    legacy(&db);
    let before = roots(&db);
    let migration = LegacyMembershipMigration::capture(&db).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        membership_file_for_read(&read).unwrap(),
        legacy_members_file()
    );
    drop(read);
    migration.apply(&db).unwrap();
    let after = roots(&db);
    assert_eq!(&before[..3], &after[..3]);
    assert_eq!(before[4], after[4]);
    assert_eq!(after[5].len(), 4 + 7 * 16);
    let read = BinaryDbReadTxn::new(&db);
    for i in 0..7 {
        let row = read.read_record(members_file(), i).unwrap();
        assert_eq!(
            &row[..12],
            &before[5][4 + i as usize * 12..16 + i as usize * 12]
        );
        assert_eq!(
            Membership::decode(&row).unwrap().role,
            Some(ROLE_INDEXES[i as usize])
        );
    }
    assert_eq!(read.read_record(teams_file(), 0).unwrap()[0], 0);
    drop(read);
    migration.apply(&db).unwrap();
    assert_eq!(roots(&db), after);
    ensure_default(&db, [0, 1, 2]).unwrap();
    assert_eq!(roots(&db), after);
}
#[test]
fn migration_rejects_concurrent_team_and_dependency_changes() {
    let (_temp, db) = initialized();
    legacy(&db);
    let migration = LegacyMembershipMigration::capture(&db).unwrap();
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    let mut row = tx.read_record(teams_file(), 0).unwrap();
    row[0] = 1;
    let mut bytes = 1u32.to_le_bytes().to_vec();
    bytes.extend(row);
    tx.commit_with_atomic_file_replacement(teams_file(), &bytes, "interrupted-prefix")
        .unwrap();
    drop(tx);
    let before = roots(&db);
    assert!(migration.apply(&db).is_err());
    assert_eq!(roots(&db), before);
    let (_temp, db) = initialized();
    legacy(&db);
    let migration = LegacyMembershipMigration::capture(&db).unwrap();
    bind(&db, 1, "", "late-binding").unwrap();
    let before = roots(&db);
    assert!(migration.apply(&db).is_err());
    assert_eq!(roots(&db), before);
}
#[test]
fn concurrent_legacy_conversions_are_idempotent() {
    let (_temp, db) = initialized();
    legacy(&db);
    let migration = LegacyMembershipMigration::capture(&db).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let db = db.clone();
            let plan = migration.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                plan.apply(&db).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(read_agents(&db).unwrap().len(), 7);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(read.record_count(members_file()).unwrap(), 7);
}
#[test]
fn legacy_default_reuses_every_binding_and_rejects_unverified_rows() {
    let (_temp, db) = initialized();
    bind(&db, 0, "", "newton").unwrap();
    legacy(&db);
    let before = read_agents(&db).unwrap();
    ensure_default(&db, [0, 1, 2]).unwrap();
    assert_eq!(read_agents(&db).unwrap(), before);
    let (_temp, db) = initialized();
    legacy(&db);
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    let mut row = tx.read_record(legacy_members_file(), 1).unwrap();
    row[8..12].copy_from_slice(&0u32.to_le_bytes());
    tx.overwrite_record(legacy_members_file(), 1, &row).unwrap();
    tx.commit().unwrap();
    drop(tx);
    let before = roots(&db);
    assert!(ensure_default(&db, [0, 1, 2]).is_err());
    assert_eq!(roots(&db), before);
}
#[test]
fn ambiguous_width_requires_explicit_selection_and_invalid_roles_fail() {
    let (_temp, db) = initialized();
    legacy(&db);
    // Four closed zero-reference legacy rows are also three current rows.
    let mut bytes = 1u32.to_le_bytes().to_vec();
    bytes.extend([0; 48]);
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    tx.commit_with_atomic_file_replacement(legacy_members_file(), &bytes, "ambiguous-fixture")
        .unwrap();
    drop(tx);
    let read = BinaryDbReadTxn::new(&db);
    assert!(membership_file_for_read(&read).is_err());
    drop(read);
    let before = roots(&db);
    assert!(LegacyMembershipMigration::capture(&db).is_err());
    assert_eq!(roots(&db), before);
    let row = Membership {
        closed: false,
        team_index: 0,
        agent_index: 0,
        role: Some(u32::MAX),
    };
    assert!(row.encode().is_err());
    assert!(validate_team_metadata(2).is_err());
    assert!(validate_team_metadata(1).is_err());
    assert!(validate_team_metadata(129).is_err());
    assert!(validate_team_metadata(128).unwrap());
}
fn pool() -> (tempfile::TempDir, LocalBinaryDbFs) {
    let (temp, db) = initialized();
    bind(&db, 1, "", "einstein-old-reviewer").unwrap();
    // Existing Agent/role records survive; only fixtures remove their old Team.
    for f in [
        "agent_teams.bin",
        "agent_teams_payload.bin",
        "agent_team_membership.bin",
    ] {
        fs::remove_file(db.authority_root().as_path().join(f)).unwrap();
    }
    (temp, db)
}
fn configs() -> Vec<LookupConfiguration> {
    [false, true]
        .into_iter()
        .map(|registered| LookupConfiguration {
            name: format!("{registered} lookup"),
            owner_agent_index: 0,
            max_concurrent_members: 7,
            members: [2, 0, 1, 3, 4, 5, 6]
                .into_iter()
                .map(|a| {
                    (
                        a,
                        if a == 0 {
                            0
                        } else if a == 1 {
                            2
                        } else {
                            1
                        },
                    )
                })
                .collect(),
        })
        .collect()
}
#[test]
fn lookup_keeps_effective_roles_sessions_and_stored_seat_order() {
    let (_temp, db) = pool();
    let agents_before = roots(&db)[..3].to_vec();
    assert_eq!(
        ensure_lookup_configurations(&db, &configs()).unwrap(),
        vec![0, 1]
    );
    assert_eq!(&roots(&db)[..3], agents_before);
    let before = roots(&db);
    ensure_lookup_configurations(&db, &configs()).unwrap();
    assert_eq!(roots(&db), before);
    let read = BinaryDbReadTxn::new(&db);
    assert!(matches!(
        read_directory(&read).unwrap(),
        DirectoryLookup::Ambiguous
    ));
    for team in 0..2 {
        let DirectoryLookup::Found(directory) = read_directory_for_team(&read, team).unwrap()
        else {
            panic!("missing selected team")
        };
        assert_eq!(
            directory
                .members
                .iter()
                .map(|m| m.agent_index)
                .collect::<Vec<_>>(),
            vec![2, 0, 1, 3, 4, 5, 6]
        );
        let einstein = &directory.members[2];
        assert_eq!(einstein.agent.session, "einstein-old-reviewer");
        assert_eq!(einstein.agent.role, Some(2));
        assert_eq!(einstein.configured_role_index, Some(2));
    }
}
#[test]
fn lookup_retries_a_pending_team_prefix_and_refuses_conflicts_without_writes() {
    let (_temp, db) = pool();
    ensure_lookup_configurations(&db, &configs()).unwrap();
    fs::remove_file(
        db.authority_root()
            .as_path()
            .join("agent_team_membership.bin"),
    )
    .unwrap();
    let before = roots(&db);
    ensure_lookup_configurations(&db, &configs()).unwrap();
    assert_eq!(&roots(&db)[..5], &before[..5]);
    let before = roots(&db);
    let mut bad = configs();
    bad[0].members[2].1 = 1;
    assert!(ensure_lookup_configurations(&db, &bad).is_err());
    assert_eq!(roots(&db), before);
    let mut bad = configs();
    bad[0].members[0].1 = 99;
    assert!(ensure_lookup_configurations(&db, &bad).is_err());
    assert_eq!(roots(&db), before);
}
#[test]
fn concurrent_lookup_initializers_never_duplicate_or_rebind() {
    let (_temp, db) = pool();
    let before = roots(&db)[..3].to_vec();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let db = db.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                ensure_lookup_configurations(&db, &configs()).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(&roots(&db)[..3], before);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(read.record_count(teams_file()).unwrap(), 2);
    assert_eq!(read.record_count(members_file()).unwrap(), 14);
}

#[test]
fn explicit_conversion_resolves_valid_ambiguous_legacy_records() {
    let (_temp, db) = initialized();
    legacy(&db);
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    let row = tx.read_record(teams_file(), 0).unwrap();
    tx.append_record(teams_file(), &row).unwrap();
    tx.commit().unwrap();
    drop(tx);
    let mut bytes = 1u32.to_le_bytes().to_vec();
    for _ in 0..4 {
        bytes.extend([1, 0, 0, 0]);
        bytes.extend([0; 8]);
    }
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    tx.commit_with_atomic_file_replacement(legacy_members_file(), &bytes, "valid-ambiguous")
        .unwrap();
    drop(tx);
    let read = BinaryDbReadTxn::new(&db);
    assert!(membership_file_for_read(&read).is_err());
    drop(read);
    let plan = LegacyMembershipMigration::capture(&db).unwrap();
    plan.apply(&db).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(membership_file_for_read(&read).unwrap(), members_file());
    assert_eq!(read.record_count(members_file()).unwrap(), 4);
    for i in 0..4 {
        let row = read.read_record(members_file(), i).unwrap();
        assert_eq!(
            &row[..12],
            &bytes[4 + i as usize * 12..16 + i as usize * 12]
        );
    }
}

#[test]
fn legacy_contact_precondition_protects_binding_before_conversion() {
    let (_temp, db) = initialized();
    legacy(&db);
    let guard = TeamBindingPrecondition::capture(&db).unwrap();
    bind_checked(&db, 2, "", "legacy-curie", &guard).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let DirectoryLookup::Found(team) = read_directory(&read).unwrap() else {
        panic!("legacy contact missing")
    };
    assert_eq!(team.members[2].agent.session, "legacy-curie");
    drop(read);
    let guard = TeamBindingPrecondition::capture(&db).unwrap();
    ensure_default(&db, [0, 1, 2]).unwrap();
    assert!(guard.verify(&db).is_err());
    assert_eq!(read_agents(&db).unwrap()[2].session, "legacy-curie");
}

fn retired_flag(db: &LocalBinaryDbFs, team: u32, flag: u8) {
    let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General).unwrap();
    let mut row = tx.read_record(teams_file(), team).unwrap();
    row[0] = flag;
    tx.overwrite_record(teams_file(), team, &row).unwrap();
    tx.commit().unwrap();
}
#[test]
fn registration_selects_explicit_keys_and_ignores_retired_flags_and_names() {
    let (_temp, db) = pool();
    ensure_lookup_configurations(&db, &configs()).unwrap();
    // Reverse the old flags: registration, never either flag, selects the profile.
    retired_flag(&db, 0, 1);
    retired_flag(&db, 1, 0);
    let before = roots(&db);
    let profiles = LookupProfiles {
        unregistered_team_index: 0,
        registered_team_index: 1,
    };
    let read = BinaryDbReadTxn::new(&db);
    for registered in [false, true] {
        let DirectoryLookup::Found(team) = profiles.read(&read, registered).unwrap() else {
            panic!("missing profile")
        };
        assert_eq!(team.team_index, u32::from(registered));
        assert_eq!(team.members[2].configured_role_index, Some(2));
    }
    let invalid = LookupProfiles {
        unregistered_team_index: 1,
        registered_team_index: 1,
    };
    assert!(invalid.read(&read, false).is_err());
    let missing = LookupProfiles {
        unregistered_team_index: 99,
        registered_team_index: 1,
    };
    assert!(matches!(
        missing.read(&read, false).unwrap(),
        DirectoryLookup::Missing
    ));
    drop(read);
    assert_eq!(roots(&db), before);
}
#[test]
fn retired_bit_conversion_changes_only_one_byte_and_retries_exactly() {
    let (_temp, db) = pool();
    ensure_lookup_configurations(&db, &configs()).unwrap();
    retired_flag(&db, 1, 1);
    let before = roots(&db);
    let plan = TeamMetadataRetirement::capture(&db).unwrap();
    plan.apply(&db).unwrap();
    let after = roots(&db);
    for i in [0, 1, 2, 4, 5] {
        assert_eq!(before[i], after[i]);
    }
    let changed = before[3]
        .iter()
        .zip(&after[3])
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(changed, vec![24]);
    plan.apply(&db).unwrap();
    assert_eq!(roots(&db), after);
    ensure_lookup_configurations(&db, &configs()).unwrap();
    assert_eq!(roots(&db), after);
}
#[test]
fn concurrent_retirements_preserve_bindings_and_compare_exact_source() {
    let (_temp, db) = pool();
    ensure_lookup_configurations(&db, &configs()).unwrap();
    retired_flag(&db, 1, 1);
    let plan = TeamMetadataRetirement::capture(&db).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let db = db.clone();
            let p = plan.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                p.apply(&db).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        read_agents(&db).unwrap()[1].session,
        "einstein-old-reviewer"
    );
    retired_flag(&db, 1, 1);
    let plan = TeamMetadataRetirement::capture(&db).unwrap();
    bind(&db, 3, "", "late-curie").unwrap();
    let before = roots(&db);
    assert!(plan.apply(&db).is_err());
    assert_eq!(roots(&db), before);
    let plan = TeamMetadataRetirement::capture(&db).unwrap();
    retired_flag(&db, 0, 2);
    let before = roots(&db);
    assert!(plan.apply(&db).is_err());
    assert_eq!(roots(&db), before);
    assert!(TeamMetadataRetirement::capture(&db).is_err());
}
#[test]
fn archived_team_retirement_preserves_archival_and_closed_memberships() {
    let (_temp, db) = pool();
    ensure_lookup_configurations(&db, &configs()).unwrap();
    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
    for i in 7..14 {
        let mut row = tx.read_record(members_file(), i).unwrap();
        row[0] = 1;
        tx.overwrite_record(members_file(), i, &row).unwrap();
    }
    tx.commit().unwrap();
    drop(tx);
    retired_flag(&db, 1, 129);
    let before = roots(&db);
    TeamMetadataRetirement::capture(&db)
        .unwrap()
        .apply(&db)
        .unwrap();
    let after = roots(&db);
    assert_eq!(before[5], after[5]);
    assert_eq!(after[3][24], 128);
}
#[test]
fn default_bootstrap_emits_zero_reserved_bits_and_reuses_retired_team() {
    let (_temp, db) = initialized();
    bind(&db, 0, "", "existing-newton").unwrap();
    let before = roots(&db);
    assert_eq!(before[3][4], 0);
    retired_flag(&db, 0, 1);
    ensure_default(&db, [0, 1, 2]).unwrap();
    assert_eq!(roots(&db), before);
}
