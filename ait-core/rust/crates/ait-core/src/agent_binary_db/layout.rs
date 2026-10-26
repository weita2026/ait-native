//! Current fixed codecs and an explicitly selected, retryable legacy conversion.
use super::*;
use crate::binary_db::{BinaryDb, BinaryDbFsyncPolicy};

/// Current writes require all low metadata bits to remain reserved.
pub fn validate_team_metadata(meta: u8) -> StoreResult<bool> {
    if meta & 127 != 0 {
        return Err(invalid("Invalid AgentTeam reserved bits"));
    }
    Ok(meta & 128 != 0)
}
/// The retired bit may be read solely for compatibility. It carries no policy.
pub(super) fn validate_retired_team_metadata(meta: u8) -> StoreResult<bool> {
    validate_team_metadata(meta & !1)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Membership {
    pub closed: bool,
    pub team_index: u32,
    pub agent_index: u32,
    pub role: Option<u32>,
}
impl Membership {
    pub fn decode(row: &[u8]) -> StoreResult<Self> {
        if row.len() != 16 || row[0] > 1 || row[1..4] != [0; 3] {
            return Err(invalid("Invalid AgentTeamMembership record"));
        }
        Ok(Self {
            closed: row[0] == 1,
            team_index: u32_at(row, 4),
            agent_index: u32_at(row, 8),
            role: u32_at(row, 12).checked_sub(1),
        })
    }
    pub fn encode(&self) -> StoreResult<Vec<u8>> {
        let mut row = vec![u8::from(self.closed), 0, 0, 0];
        row.extend(self.team_index.to_le_bytes());
        row.extend(self.agent_index.to_le_bytes());
        row.extend(
            match self.role {
                Some(i) => i
                    .checked_add(1)
                    .ok_or_else(|| invalid("Membership role overflow"))?,
                None => 0,
            }
            .to_le_bytes(),
        );
        Ok(row)
    }
}
pub(super) fn legacy_members_file() -> BinaryFileId {
    BinaryFileId::new("agent_team_membership.bin", 1, 12)
}
pub(super) fn read_rows(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    file: BinaryFileId,
) -> StoreResult<Vec<Vec<u8>>> {
    (0..read.record_count(file.clone())?)
        .map(|i| read.read_record(file.clone(), i))
        .collect()
}
fn validate_rows(
    rows: &[Vec<u8>],
    legacy: bool,
    teams: u32,
    agents: u32,
    roles: u32,
) -> StoreResult<()> {
    for row in rows {
        if row.len() != if legacy { 12 } else { 16 } || row[0] > 1 || row[1..4] != [0; 3] {
            return Err(invalid("Invalid membership metadata or width"));
        }
        if u32_at(row, 4) >= teams
            || u32_at(row, 8) >= agents
            || (!legacy && u32_at(row, 12) > roles)
        {
            return Err(invalid("Membership reference is unresolved"));
        }
    }
    Ok(())
}
/// Compatibility reads admit a width only when its entire reference closure
/// validates uniquely. Byte length alone is not a format discriminator.
/// Ambiguous nonempty data requires explicit legacy selection; no writes occur.
pub(super) fn membership_file_for_read(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> StoreResult<BinaryFileId> {
    let counts = (
        read.record_count(teams_file())?,
        read.record_count(agents_file())?,
        read.record_count(roles_file())?,
    );
    let candidate = |file, legacy| -> StoreResult<Vec<Vec<u8>>> {
        let rows = read_rows(read, file)?;
        validate_rows(&rows, legacy, counts.0, counts.1, counts.2)?;
        Ok(rows)
    };
    match (
        candidate(members_file(), false),
        candidate(legacy_members_file(), true),
    ) {
        (Ok(new), Ok(old)) if new.is_empty() && old.is_empty() => Ok(members_file()),
        (Ok(_), Ok(_)) => Err(invalid(
            "Ambiguous membership width; explicit legacy conversion required",
        )),
        (Ok(_), Err(_)) => Ok(members_file()),
        (Err(_), Ok(_)) => Ok(legacy_members_file()),
        (Err(_), Err(_)) => Err(invalid("Invalid membership file or unresolved reference")),
    }
}
fn write_rows<B: BinaryDb, F: BinaryDbFsyncPolicy>(
    write: &BinaryDbWriteTxn<'_, B, F>,
    file: BinaryFileId,
) -> StoreResult<Vec<Vec<u8>>> {
    (0..write.record_count(file.clone())?)
        .map(|i| write.read_record(file.clone(), i))
        .collect()
}
fn bytes(rows: &[Vec<u8>]) -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    for row in rows {
        out.extend(row);
    }
    out
}
fn matches<B: BinaryDb, F: BinaryDbFsyncPolicy>(
    write: &BinaryDbWriteTxn<'_, B, F>,
    file: BinaryFileId,
    rows: &[Vec<u8>],
) -> bool {
    write_rows(write, file).is_ok_and(|current| current == rows)
}

/// Explicit source selector: capture exact legacy-12 records. Team metadata
/// carries no registration policy. Reuse this plan after an uncertain response.
/// No ordinal, Agent role, session, payload, or Plan pin is changed.
#[derive(Clone)]
pub struct LegacyMembershipMigration {
    pub(super) source_members: Vec<Vec<u8>>,
    target_members: Vec<Vec<u8>>,
    source_teams: Vec<Vec<u8>>,
    dependencies: Vec<(BinaryFileId, Vec<Vec<u8>>)>,
}
impl LegacyMembershipMigration {
    pub fn capture(db: &LocalBinaryDbFs) -> StoreResult<Self> {
        let read = BinaryDbReadTxn::new(db);
        let source_teams = read_rows(&read, teams_file())?;
        let agents = read_agents_under(&read)?;
        let roles = read_rows(&read, roles_file())?;
        for row in &roles {
            let pin = decode_role(row)?;
            if PlanRevisionCodec::<1>::decode_record(
                &read.read_record(PlanRevisionCodec::<1>::record_file(), pin)?,
            )?
            .is_tombstone()
            {
                return Err(invalid("Legacy role revision was reclaimed"));
            }
        }
        let source_members = read_rows(&read, legacy_members_file())?;
        if source_members.is_empty() {
            return Err(invalid("No legacy memberships to convert"));
        }
        validate_rows(
            &source_members,
            true,
            source_teams.len() as u32,
            agents.len() as u32,
            roles.len() as u32,
        )?;
        for row in &source_teams {
            validate_retired_team_metadata(row[0])?;
            if row[1] != 0 || u32_at(row, 12) as usize >= agents.len() {
                return Err(invalid("Invalid legacy Team reference"));
            }
            if row[0] & 128 == 0 && agents[u32_at(row, 12) as usize].archived {
                return Err(invalid("Active legacy Team owner is archived"));
            }
            let (offset, len) = locator(row)?;
            let name = read.read_payload(team_payload_file(), offset, len)?;
            if name.is_empty() || std::str::from_utf8(&name).is_err() {
                return Err(invalid("Invalid legacy Team name"));
            }
            let target = u16::from_le_bytes(row[16..18].try_into().unwrap());
            let concurrent = u16::from_le_bytes(row[18..20].try_into().unwrap());
            if target == 0 || concurrent == 0 || concurrent > target {
                return Err(invalid("Invalid legacy Team size"));
            }
        }
        let mut pairs = BTreeSet::new();
        let mut target_members = Vec::new();
        for row in &source_members {
            let agent = &agents[u32_at(row, 8) as usize];
            if let Some(role) = agent.role {
                if role as usize >= roles.len() {
                    return Err(invalid("Unresolved legacy Agent role"));
                }
                if row[0] == 0 && roles[role as usize][0] != 0 {
                    return Err(invalid("Active legacy role is archived"));
                }
            }
            if row[0] == 0
                && (agent.archived
                    || source_teams[u32_at(row, 4) as usize][0] & 128 != 0
                    || !pairs.insert((u32_at(row, 4), u32_at(row, 8))))
            {
                return Err(invalid("Invalid active legacy membership"));
            }
            let mut converted = row.clone();
            converted.extend(
                match agent.role {
                    Some(r) => r
                        .checked_add(1)
                        .ok_or_else(|| invalid("Legacy role overflow"))?,
                    None => 0,
                }
                .to_le_bytes(),
            );
            target_members.push(converted);
        }
        Ok(Self {
            source_members,
            target_members,
            source_teams,
            dependencies: vec![
                (agents_file(), read_rows(&read, agents_file())?),
                (roles_file(), roles),
            ],
        })
    }
    fn check<B: BinaryDb, F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, B, F>,
    ) -> StoreResult<bool> {
        for (file, rows) in &self.dependencies {
            if !matches(tx, file.clone(), rows) {
                return Err(invalid("Legacy conversion dependencies changed"));
            }
        }
        verify_live_role_pins(tx)?;
        if !matches(tx, teams_file(), &self.source_teams) {
            return Err(invalid("Legacy Team source changed"));
        }
        let current = matches(tx, members_file(), &self.target_members);
        if !current && !matches(tx, legacy_members_file(), &self.source_members) {
            return Err(invalid("Legacy membership source changed"));
        }
        Ok(current)
    }
    pub fn apply(&self, db: &LocalBinaryDbFs) -> StoreResult<()> {
        let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
        let current = self.check(&tx)?;
        if current {
            tx.commit()?;
        } else {
            tx.commit_with_atomic_file_replacement(
                members_file(),
                &bytes(&self.target_members),
                "legacy-membership-16",
            )?;
        }
        Ok(())
    }
}
pub(super) fn read_agents_under(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
) -> StoreResult<Vec<Agent>> {
    let mut agents = Vec::new();
    let mut sessions = BTreeSet::new();
    let role_count = read.record_count(roles_file())?;
    for i in 0..read.record_count(agents_file())? {
        let row = read.read_record(agents_file(), i)?;
        let (offset, len) = locator(&row)?;
        let agent = Agent::decode(&row, &read.read_payload(payload_file(), offset, len)?)?;
        if agent.role.is_some_and(|role| role >= role_count) {
            return Err(invalid("Agent role reference is unresolved"));
        }
        if !agent.session.is_empty() && !sessions.insert(agent.session.clone()) {
            return Err(invalid("Duplicate Agent session binding"));
        }
        agents.push(agent);
    }
    Ok(agents)
}

pub(super) fn verify_live_role_pins<B: BinaryDb, F: BinaryDbFsyncPolicy>(
    tx: &BinaryDbWriteTxn<'_, B, F>,
) -> StoreResult<()> {
    for i in 0..tx.record_count(roles_file())? {
        let pin = decode_role(&tx.read_record(roles_file(), i)?)?;
        if PlanRevisionCodec::<1>::decode_record(
            &tx.read_record(PlanRevisionCodec::<1>::record_file(), pin)?,
        )?
        .is_tombstone()
        {
            return Err(invalid("Role revision was reclaimed during publication"));
        }
    }
    Ok(())
}
