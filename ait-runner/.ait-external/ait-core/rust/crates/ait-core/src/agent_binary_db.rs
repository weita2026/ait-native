//! Local Agent authority. Every publication replaces one complete fixed file,
//! after its payload/dependencies are durable. Bootstrap progresses through
//! valid prefixes (roles -> agents -> team -> memberships), so retries never
//! truncate records or recycle physical keys.
use crate::binary_db::{
    BinaryDbCommandScope, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId, BinaryPayloadFileId,
    LocalBinaryDbFs, StoreResult,
};
use crate::plan_binary_db::{PlanRevisionCodec, PLAN_REVISION_RECORD_SIZE};
use std::collections::BTreeSet;
pub mod single_team;

pub const NAMES: [&str; 7] = [
    "Newton", "Einstein", "Maxwell", "Curie", "Feynman", "Faraday", "Noether",
];
pub const ROLE_INDEXES: [u32; 7] = [0, 2, 1, 1, 1, 1, 1];
pub fn agents_file() -> BinaryFileId {
    BinaryFileId::new("agents.bin", 1, 16)
}
pub fn roles_file() -> BinaryFileId {
    BinaryFileId::new("agent_roles.bin", 1, 8)
}
pub fn teams_file() -> BinaryFileId {
    BinaryFileId::new("agent_teams.bin", 1, 20)
}
pub fn members_file() -> BinaryFileId {
    BinaryFileId::new("agent_team_membership.bin", 1, 16)
}
fn payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new("agents_payload.bin", 1)
}
fn team_payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new("agent_teams_payload.bin", 1)
}
fn invalid(message: &str) -> crate::binary_db::BinaryDbError {
    message.to_string().into()
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub archived: bool,
    pub role: Option<u32>,
    pub name: String,
    pub session: String,
}
impl Agent {
    pub fn decode(row: &[u8], payload: &[u8]) -> StoreResult<Self> {
        if row.len() != 16 || row[0] & 127 != 0 || row[1] & 254 != 0 || payload.len() < 2 {
            return Err(invalid("Invalid Agent record or payload"));
        }
        if usize::from(u16::from_le_bytes(row[2..4].try_into().unwrap())) != payload.len() {
            return Err(invalid("Agent payload length mismatch"));
        }
        let name_len = usize::from(u16::from_le_bytes(payload[..2].try_into().unwrap()));
        if name_len == 0 || name_len > payload.len() - 2 {
            return Err(invalid("Invalid Agent name length"));
        }
        let name = std::str::from_utf8(&payload[2..2 + name_len])
            .map_err(|_| invalid("Invalid Agent name UTF-8"))?
            .to_owned();
        let session = std::str::from_utf8(&payload[2 + name_len..])
            .map_err(|_| invalid("Invalid session UTF-8"))?
            .to_owned();
        if (row[1] & 1 != 0) == session.is_empty() {
            return Err(invalid("Agent session flag mismatch"));
        }
        Ok(Self {
            archived: row[0] != 0,
            role: u32_at(row, 12).checked_sub(1),
            name,
            session,
        })
    }
    fn payload(&self) -> StoreResult<Vec<u8>> {
        let name_len =
            u16::try_from(self.name.len()).map_err(|_| invalid("Agent name too long"))?;
        let mut out = name_len.to_le_bytes().to_vec();
        out.extend(self.name.as_bytes());
        out.extend(self.session.as_bytes());
        if self.name.is_empty() || out.len() > usize::from(u16::MAX) {
            return Err(invalid("Invalid Agent payload size"));
        }
        Ok(out)
    }
    fn encode(&self, offset: u64, len: u16) -> StoreResult<Vec<u8>> {
        let role = match self.role {
            Some(i) => i
                .checked_add(1)
                .ok_or_else(|| invalid("Agent role overflow"))?,
            None => 0,
        };
        let mut out = vec![
            if self.archived { 128 } else { 0 },
            u8::from(!self.session.is_empty()),
        ];
        out.extend(len.to_le_bytes());
        out.extend(offset.to_le_bytes());
        out.extend(role.to_le_bytes());
        Ok(out)
    }
}
fn locator(row: &[u8]) -> StoreResult<(u64, u32)> {
    let offset = u64::from_le_bytes(row[4..12].try_into().unwrap());
    let len = u32::from(u16::from_le_bytes(row[2..4].try_into().unwrap()));
    if offset < 4 {
        return Err(invalid("Agent payload locator overlaps header"));
    }
    Ok((offset, len))
}
pub fn decode_role(row: &[u8]) -> StoreResult<u32> {
    if row.len() != 8 || row[0] & 127 != 0 || row[1..4] != [0, 0, 0] {
        return Err(invalid("Invalid AgentRole record"));
    }
    Ok(u32_at(row, 4))
}
pub fn read_agents(db: &LocalBinaryDbFs) -> StoreResult<Vec<Agent>> {
    let read = BinaryDbReadTxn::new(db);
    (0..read.record_count(agents_file())?)
        .map(|i| {
            let row = read.read_record(agents_file(), i)?;
            let (offset, len) = locator(&row)?;
            Agent::decode(&row, &read.read_payload(payload_file(), offset, len)?)
        })
        .collect()
}
pub fn read_role_pins(db: &LocalBinaryDbFs) -> StoreResult<Vec<u32>> {
    let read = BinaryDbReadTxn::new(db);
    (0..read.record_count(roles_file())?)
        .map(|i| decode_role(&read.read_record(roles_file(), i)?))
        .collect()
}

/// Only the initial default team is provisioned by this version. Existing
/// non-default authority is refused instead of guessing from duplicate names.
pub fn ensure_default(db: &LocalBinaryDbFs, pins: [u32; 3]) -> StoreResult<()> {
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(roles_file())?;
    if count != 0 && count != 3 {
        return Err(invalid("Default bootstrap requires zero or three roles"));
    }
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    for (i, pin) in pins.into_iter().enumerate() {
        let row = write.read_record(
            BinaryFileId::new("plan_revision.bin", 1, PLAN_REVISION_RECORD_SIZE),
            pin,
        )?;
        let revision = PlanRevisionCodec::<1>::decode_record(&row)?;
        if revision.is_tombstone() {
            return Err(invalid("Role revision is tombstoned"));
        }
        let mut role = vec![0; 4];
        role.extend(pin.to_le_bytes());
        if count != 0 && write.read_record(roles_file(), i as u32)? != role {
            return Err(invalid(
                "Existing role pin differs; explicit role upgrade required",
            ));
        }
        bytes.extend(role);
    }
    if count == 0 {
        write.commit_with_atomic_file_replacement(roles_file(), &bytes, "agent-roles")?;
    } else {
        write.commit()?;
    }

    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(agents_file())?;
    if count != 0 && count != 7 {
        return Err(invalid("Default bootstrap requires zero or seven Agents"));
    }
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    let mut sessions = BTreeSet::new();
    for (i, name) in NAMES.into_iter().enumerate() {
        if count != 0 {
            let row = write.read_record(agents_file(), i as u32)?;
            let (offset, len) = locator(&row)?;
            let agent = Agent::decode(&row, &write.read_payload(payload_file(), offset, len)?)?;
            if agent.archived || agent.name != name || agent.role != Some(ROLE_INDEXES[i]) {
                return Err(invalid(
                    "Existing Agents do not match default team; no records changed",
                ));
            }
            if !agent.session.is_empty() && !sessions.insert(agent.session) {
                return Err(invalid("Duplicate Agent session binding"));
            }
        } else {
            let agent = Agent {
                archived: false,
                role: Some(ROLE_INDEXES[i]),
                name: name.into(),
                session: String::new(),
            };
            let payload = agent.payload()?;
            let range = write.append_payload(payload_file(), &payload)?;
            bytes.extend(agent.encode(range.payload_offset, payload.len() as u16)?);
        }
    }
    if count == 0 {
        write.commit_with_atomic_file_replacement(agents_file(), &bytes, "agents")?;
    } else {
        write.commit()?;
    }

    // Select legacy data only after checking the exact historical bootstrap.
    let migration = {
        let read = BinaryDbReadTxn::new(db);
        let legacy = membership_file_for_read(&read)? == legacy_members_file();
        if legacy {
            let agents = layout::read_agents_under(&read)?;
            if agents.len() != 7
                || agents.iter().enumerate().any(|(i, agent)| {
                    agent.archived || agent.name != NAMES[i] || agent.role != Some(ROLE_INDEXES[i])
                })
            {
                return Err(invalid("Legacy Agents differ from default"));
            }
            if read.record_count(teams_file())? != 1 {
                return Err(invalid("Legacy default requires one verified Team"));
            }
            let team = read.read_record(teams_file(), 0)?;
            let (offset, len) = locator(&team)?;
            if (team[..2] != [0, 0] && team[..2] != [1, 0])
                || team[12..] != [0, 0, 0, 0, 7, 0, 7, 0]
                || read.read_payload(team_payload_file(), offset, len)? != b"Default team"
                || read.record_count(legacy_members_file())? != 7
            {
                return Err(invalid("Legacy Team differs from default"));
            }
            for i in 0..7 {
                let row = read.read_record(legacy_members_file(), i)?;
                if row[..8] != [0; 8] || u32_at(&row, 8) != i {
                    return Err(invalid("Legacy membership differs from default"));
                }
            }
            // Capture while the validated source is still read-locked, so a
            // competing change cannot become a newly trusted legacy source.
            Some(LegacyMembershipMigration::capture(db)?)
        } else {
            None
        }
    };
    if let Some(migration) = migration {
        migration.apply(db)?;
    }
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(teams_file())?;
    if count > 1 {
        return Err(invalid(
            "Multiple teams require explicit selection; default bootstrap stopped",
        ));
    }
    if count == 0 {
        let name = b"Default team";
        let range = write.append_payload(team_payload_file(), name)?;
        let mut bytes = 1_u32.to_le_bytes().to_vec();
        bytes.extend([0, 0]);
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(range.payload_offset.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        bytes.extend(7_u16.to_le_bytes());
        bytes.extend(7_u16.to_le_bytes());
        write.commit_with_atomic_file_replacement(teams_file(), &bytes, "agent-team")?;
    } else {
        let row = write.read_record(teams_file(), 0)?;
        let (offset, len) = locator(&row)?;
        if (row[..2] != [0, 0] && row[..2] != [1, 0])
            || row[12..] != [0, 0, 0, 0, 7, 0, 7, 0]
            || write.read_payload(team_payload_file(), offset, len)? != b"Default team"
        {
            return Err(invalid("Existing team differs from default"));
        }
        if row[0] == 1 {
            let mut bytes = 1_u32.to_le_bytes().to_vec();
            let mut row = row;
            row[0] = 0;
            bytes.extend(row);
            write.commit_with_atomic_file_replacement(
                teams_file(),
                &bytes,
                "default-team-reserved-bits",
            )?;
        } else {
            write.commit()?;
        }
    }
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(members_file())?;
    if count != 0 && count != 7 {
        return Err(invalid("Existing memberships differ from default"));
    }
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    for i in 0_u32..7 {
        let mut row = vec![0; 8];
        row.extend(i.to_le_bytes());
        row.extend((ROLE_INDEXES[i as usize] + 1).to_le_bytes());
        if count != 0 && write.read_record(members_file(), i)? != row {
            return Err(invalid("Existing membership differs from default"));
        }
        bytes.extend(row);
    }
    if count == 0 {
        write.commit_with_atomic_file_replacement(members_file(), &bytes, "agent-members")?;
    } else {
        write.commit()?;
    }
    Ok(())
}

/// Caller supplies a verified native receipt. Compare-and-swap and uniqueness
/// are checked under the all-family write lock before publishing the locator.
/// Short-lived optimistic precondition, not persisted authority or history.
/// Conservatively reject any team record changes while the host is contacted.
pub struct TeamBindingPrecondition(Vec<(BinaryFileId, Vec<Vec<u8>>)>);
impl TeamBindingPrecondition {
    pub fn verify(&self, db: &LocalBinaryDbFs) -> StoreResult<()> {
        if self.0 != Self::capture(db)?.0 {
            return Err(invalid("Team changed during session preparation"));
        }
        Ok(())
    }

    pub fn capture(db: &LocalBinaryDbFs) -> StoreResult<Self> {
        let read = BinaryDbReadTxn::new(db);
        let mut records = Vec::new();
        let members = membership_file_for_read(&read)?;
        for file in [agents_file(), roles_file(), teams_file(), members] {
            let rows = (0..read.record_count(file.clone())?)
                .map(|i| read.read_record(file.clone(), i))
                .collect::<StoreResult<Vec<_>>>()?;
            records.push((file, rows));
        }
        Ok(Self(records))
    }
}
pub fn bind_checked(
    db: &LocalBinaryDbFs,
    index: u32,
    expected: &str,
    session: &str,
    guard: &TeamBindingPrecondition,
) -> StoreResult<()> {
    bind_inner(db, index, expected, session, Some(guard))
}

/// Publish an explicitly validated role revision under the same team precondition
/// used for native session preparation. This changes no record layout or binding.
pub fn upgrade_role_checked(
    db: &LocalBinaryDbFs,
    role: u32,
    expected: u32,
    revision: u32,
    guard: &TeamBindingPrecondition,
) -> StoreResult<()> {
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    for (file, rows) in &guard.0 {
        if write.record_count(file.clone())? != rows.len() as u32 {
            return Err(invalid("Team changed during role reload"));
        }
        for (i, row) in rows.iter().enumerate() {
            if write.read_record(file.clone(), i as u32)? != *row {
                return Err(invalid("Team changed during role reload"));
            }
        }
    }
    let target = PlanRevisionCodec::<1>::decode_record(
        &write.read_record(PlanRevisionCodec::<1>::record_file(), revision)?,
    )?;
    if target.is_tombstone() {
        return Err(invalid("Target role revision was reclaimed"));
    }
    let count = write.record_count(roles_file())?;
    if role >= count {
        return Err(invalid("Unknown role"));
    }
    let mut bytes = 1u32.to_le_bytes().to_vec();
    for i in 0..count {
        let mut row = write.read_record(roles_file(), i)?;
        if i == role {
            if row[0] != 0 || decode_role(&row)? != expected {
                return Err(invalid("Role pin changed or is archived"));
            }
            row[4..8].copy_from_slice(&revision.to_le_bytes());
        }
        bytes.extend(row);
    }
    write.commit_with_atomic_file_replacement(roles_file(), &bytes, "agent-role-upgrade")?;
    Ok(())
}
pub fn bind(db: &LocalBinaryDbFs, index: u32, expected: &str, session: &str) -> StoreResult<()> {
    bind_inner(db, index, expected, session, None)
}
fn bind_inner(
    db: &LocalBinaryDbFs,
    index: u32,
    expected: &str,
    session: &str,
    guard: Option<&TeamBindingPrecondition>,
) -> StoreResult<()> {
    if session.is_empty() {
        return Err(invalid("Empty native session receipt"));
    }
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    if let Some(guard) = guard {
        for (file, rows) in &guard.0 {
            if write.record_count(file.clone())? as usize != rows.len() {
                return Err(invalid("Team changed during session preparation"));
            }
            for (index, row) in rows.iter().enumerate() {
                if write.read_record(file.clone(), index as u32)? != *row {
                    return Err(invalid("Team changed during session preparation"));
                }
            }
        }
    }
    let count = write.record_count(agents_file())?;
    if index >= count {
        return Err(invalid("Agent index out of range"));
    }
    let mut rows = Vec::new();
    let mut target = None;
    for i in 0..count {
        let row = write.read_record(agents_file(), i)?;
        let (offset, len) = locator(&row)?;
        let agent = Agent::decode(&row, &write.read_payload(payload_file(), offset, len)?)?;
        if i != index && agent.session == session {
            return Err(invalid("Session already belongs to another Agent"));
        }
        if i == index {
            if agent.archived || agent.session != expected {
                return Err(invalid("Agent binding changed; reconcile before retrying"));
            }
            target = Some(agent);
        }
        rows.push(row);
    }
    let mut agent = target.unwrap();
    agent.session = session.into();
    let payload = agent.payload()?;
    let range = write.append_payload(payload_file(), &payload)?;
    rows[index as usize] = agent.encode(range.payload_offset, payload.len() as u16)?;
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend(row);
    }
    write.commit_with_atomic_file_replacement(agents_file(), &bytes, "agent-session")?;
    Ok(())
}

/// Role Markdown and its exact tree remain roots even after its Plan is pruned.
pub fn content_gc_trees<const W: u32>(
    content: &crate::content_binary_db::LocalContentBinaryDb<W>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    cache: &mut crate::content_binary_db::BinaryDbTreeReadCache,
) -> Result<Vec<String>, String> {
    use crate::content_binary_db::BinaryTreePackCodec;
    let plans = crate::plan_binary_db::BinaryDbPlanStore::<_, W>::new(content.db().clone());
    let mut trees = Vec::new();
    for i in 0..read.record_count(roles_file())? {
        let index = decode_role(&read.read_record(roles_file(), i)?)?;
        let (revision, _) = plans.read_plan_revision(read, index)?;
        if revision.is_tombstone() {
            return Err("Agent role revision was reclaimed".into());
        }
        let pack_index = revision
            .root_tree_pack_index()
            .ok_or("Agent role revision has no content root")?;
        let pack = BinaryTreePackCodec::<W>::decode_record(
            &read.read_record(BinaryTreePackCodec::<W>::record_file(), pack_index)?,
        )?;
        if !pack.is_ready() || pack.is_tombstone() || revision.root_entry_ordinal >= pack.tree_count
        {
            return Err("Agent role content root is invalid".into());
        }
        let tree = pack
            .first_tree_index
            .checked_add(revision.root_entry_ordinal)
            .ok_or("Role tree index overflow")?;
        trees.push(
            content
                .trees()
                .tree_view_at_with_cache(read, tree, cache)?
                .tree_id,
        );
    }
    Ok(trees)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::{AuthorityId, LocalStateScope, REPOSITORY_BINARY_DB_BIN_PATHS};
    use tempfile::TempDir;
    pub(super) fn fixture() -> (TempDir, LocalBinaryDbFs) {
        let temp = TempDir::new().unwrap();
        let db = LocalBinaryDbFs::new(
            temp.path().join("db"),
            temp.path().to_path_buf(),
            AuthorityId::new("agents-test"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS);
        let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
        for _ in 0..3 {
            write
                .append_record(
                    BinaryFileId::new("plan_revision.bin", 1, PLAN_REVISION_RECORD_SIZE),
                    &vec![0; PLAN_REVISION_RECORD_SIZE as usize],
                )
                .unwrap();
        }
        write.commit().unwrap();
        drop(write);
        (temp, db)
    }
    #[test]
    fn default_retry_keeps_indexes_and_neighbors() {
        let (_temp, db) = fixture();
        ensure_default(&db, [0, 1, 2]).unwrap();
        bind(&db, 0, "", "one").unwrap();
        let before = read_agents(&db).unwrap();
        bind(&db, 1, "", "longer-native-session-id").unwrap();
        ensure_default(&db, [0, 1, 2]).unwrap();
        let after = read_agents(&db).unwrap();
        assert_eq!(after.len(), 7);
        assert_eq!(after[0], before[0]);
        assert_eq!(&after[2..], &before[2..]);
        assert!(bind(&db, 1, "", "other").is_err());
        assert!(bind(&db, 2, "", "one").is_err());
        assert_eq!(read_agents(&db).unwrap(), after);
    }
    #[test]
    fn published_prefix_resumes_without_reusing_slots() {
        let (_temp, db) = fixture();
        let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
        let mut bytes = 1_u32.to_le_bytes().to_vec();
        for i in 0_u32..3 {
            bytes.extend([0; 4]);
            bytes.extend(i.to_le_bytes());
        }
        write
            .commit_with_atomic_file_replacement(roles_file(), &bytes, "test")
            .unwrap();
        drop(write);
        ensure_default(&db, [0, 1, 2]).unwrap();
        assert_eq!(read_role_pins(&db).unwrap(), vec![0, 1, 2]);
        assert_eq!(read_agents(&db).unwrap().len(), 7);
    }
    #[test]
    fn concurrent_binding_cannot_share_one_native_session() {
        let (_temp, db) = fixture();
        ensure_default(&db, [0, 1, 2]).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let db = db.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    bind(&db, index, "", "same-native-session").is_ok()
                })
            })
            .collect::<Vec<_>>();
        let successes = handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
        assert_eq!(
            read_agents(&db)
                .unwrap()
                .iter()
                .filter(|a| a.session == "same-native-session")
                .count(),
            1
        );
    }

    #[test]
    fn codec_rejects_reserved_flags_bad_lengths_and_utf8() {
        let a = Agent {
            archived: false,
            role: Some(0),
            name: "Curie".into(),
            session: "abc".into(),
        };
        let p = a.payload().unwrap();
        let r = a.encode(4, p.len() as u16).unwrap();
        assert_eq!(r.len(), 16);
        assert_eq!(r[12..], [1, 0, 0, 0]);
        assert_eq!(Agent::decode(&r, &p).unwrap(), a);
        let mut corrupt = r.clone();
        corrupt[0] = 1;
        assert!(Agent::decode(&corrupt, &p).is_err());
        corrupt = r.clone();
        corrupt[1] = 0;
        assert!(Agent::decode(&corrupt, &p).is_err());
        let mut malformed = p.clone();
        malformed[2] = 255;
        assert!(Agent::decode(&r, &malformed).is_err());
        assert!(Agent::decode(&r, &p[..p.len() - 1]).is_err());
        assert!(decode_role(&[0; 7]).is_err());
    }
}

mod layout;
use layout::{legacy_members_file, membership_file_for_read};
pub use layout::{validate_team_metadata, LegacyMembershipMigration, Membership};
mod lookup;
pub use lookup::{ensure_lookup_configurations, LookupConfiguration, LookupProfiles};
mod contact;
pub use contact::{
    read_directory, read_directory_for_team, DirectoryLookup, DirectoryMember, TeamDirectory,
};
#[cfg(test)]
mod lookup_tests;

mod retirement;
pub use retirement::TeamMetadataRetirement;
