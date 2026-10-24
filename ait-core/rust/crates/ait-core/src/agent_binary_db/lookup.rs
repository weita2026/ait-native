//! Storage-only publication of complete precomputed member/role configurations.
//! This does not select a workflow, assign effective Agent roles or start sessions.
use super::layout::{read_agents_under, read_rows, verify_live_role_pins};
use super::*;

#[derive(Clone, Debug)]
pub struct LookupConfiguration {
    pub name: String,
    pub owner_agent_index: u32,
    pub max_concurrent_members: u16,
    /// Stored seat order, each tuple is (Agent physical index, role index).
    pub members: Vec<(u32, u32)>,
}
/// Initialize an empty Team catalog, or retry its exact durable prefix. Other
/// existing catalogs require explicit selection and are never overwritten.
/// The complete Team root precedes the complete Membership root; both replace
/// atomically under native locks. A pending Team has zero active members.
pub fn ensure_lookup_configurations(
    db: &LocalBinaryDbFs,
    configs: &[LookupConfiguration],
) -> StoreResult<Vec<u32>> {
    if configs.is_empty() {
        return Err(invalid("Empty lookup configuration set"));
    }
    let read = BinaryDbReadTxn::new(db);
    let agents = read_agents_under(&read)?;
    let roles = read_rows(&read, roles_file())?;
    let initial_teams = read_rows(&read, teams_file())?;
    let initial_members = read_rows(&read, members_file())?;
    for c in configs {
        let target =
            u16::try_from(c.members.len()).map_err(|_| invalid("Lookup team is too large"))?;
        if c.name.is_empty()
            || c.name.len() > u16::MAX as usize
            || target == 0
            || c.max_concurrent_members == 0
            || c.max_concurrent_members > target
        {
            return Err(invalid("Invalid lookup configuration"));
        }
        let mut indexes = BTreeSet::new();
        for &(agent, role) in &c.members {
            if agents.get(agent as usize).is_none_or(|a| a.archived) || !indexes.insert(agent) {
                return Err(invalid(
                    "Lookup member is unresolved, archived or duplicated",
                ));
            }
            let row = roles
                .get(role as usize)
                .ok_or_else(|| invalid("Lookup role is unresolved"))?;
            let pin = decode_role(row)?;
            if row[0] != 0
                || PlanRevisionCodec::<1>::decode_record(
                    &read.read_record(PlanRevisionCodec::<1>::record_file(), pin)?,
                )?
                .is_tombstone()
            {
                return Err(invalid("Lookup role is archived or reclaimed"));
            }
        }
        if !indexes.contains(&c.owner_agent_index) {
            return Err(invalid("Lookup owner must be an active member"));
        }
    }
    if !initial_teams.is_empty() && initial_teams.len() != configs.len() {
        return Err(invalid("Existing Team catalog differs from lookup scope"));
    }
    let dependencies = [
        (agents_file(), read_rows(&read, agents_file())?),
        (roles_file(), roles),
    ];
    let mut expected_members = 1u32.to_le_bytes().to_vec();
    for (team, c) in configs.iter().enumerate() {
        for &(agent, role) in &c.members {
            expected_members.extend(
                Membership {
                    closed: false,
                    team_index: team as u32,
                    agent_index: agent,
                    role: Some(role),
                }
                .encode()?,
            );
        }
    }
    if !initial_members.is_empty() && initial_members.concat() != expected_members[4..] {
        return Err(invalid(
            "Existing lookup memberships differ; no records changed",
        ));
    }
    // Validate existing Team payloads before appending any dependencies.
    for (i, row) in initial_teams.iter().enumerate() {
        let c = &configs[i];
        let (offset, len) = locator(row)?;
        if row[..2] != [0, 0]
            || u32_at(row, 12) != c.owner_agent_index
            || u16::from_le_bytes(row[16..18].try_into().unwrap()) as usize != c.members.len()
            || u16::from_le_bytes(row[18..20].try_into().unwrap()) != c.max_concurrent_members
            || read.read_payload(team_payload_file(), offset, len)? != c.name.as_bytes()
        {
            return Err(invalid("Existing lookup Team differs; no records changed"));
        }
    }
    if initial_teams.is_empty() && !initial_members.is_empty() {
        return Err(invalid("Memberships exist without lookup Teams"));
    }
    drop(read);
    let check_dependencies = |tx: &BinaryDbWriteTxn<'_, LocalBinaryDbFs, _>| -> StoreResult<()> {
        verify_live_role_pins(tx)?;
        for (file, rows) in &dependencies {
            if tx.record_count(file.clone())? != rows.len() as u32 {
                return Err(invalid("Lookup dependencies changed"));
            }
            for (i, row) in rows.iter().enumerate() {
                if tx.read_record(file.clone(), i as u32)? != *row {
                    return Err(invalid("Lookup dependencies changed"));
                }
            }
        }
        Ok(())
    };
    let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    check_dependencies(&tx)?;
    let team_count = tx.record_count(teams_file())?;
    let mut expected_teams = 1u32.to_le_bytes().to_vec();
    if team_count == 0 {
        if tx.record_count(members_file())? != 0 {
            return Err(invalid("Lookup catalog changed"));
        }
        for c in configs {
            let range = tx.append_payload(team_payload_file(), c.name.as_bytes())?;
            expected_teams.extend([0, 0]);
            expected_teams.extend((c.name.len() as u16).to_le_bytes());
            expected_teams.extend(range.payload_offset.to_le_bytes());
            expected_teams.extend(c.owner_agent_index.to_le_bytes());
            expected_teams.extend((c.members.len() as u16).to_le_bytes());
            expected_teams.extend(c.max_concurrent_members.to_le_bytes());
        }
        tx.commit_with_atomic_file_replacement(teams_file(), &expected_teams, "lookup-teams")?;
    } else {
        // A concurrent identical initializer may have published the prefix.
        if team_count as usize != configs.len() {
            return Err(invalid("Lookup catalog changed"));
        }
        for (i, c) in configs.iter().enumerate() {
            let row = tx.read_record(teams_file(), i as u32)?;
            let (offset, len) = locator(&row)?;
            if row[..2] != [0, 0]
                || u32_at(&row, 12) != c.owner_agent_index
                || u16::from_le_bytes(row[16..18].try_into().unwrap()) as usize != c.members.len()
                || u16::from_le_bytes(row[18..20].try_into().unwrap()) != c.max_concurrent_members
                || tx.read_payload(team_payload_file(), offset, len)? != c.name.as_bytes()
            {
                return Err(invalid("Lookup catalog changed"));
            }
            expected_teams.extend(row);
        }
        tx.commit()?;
    }
    drop(tx);
    let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    check_dependencies(&tx)?;
    let team_rows = (0..tx.record_count(teams_file())?)
        .map(|i| tx.read_record(teams_file(), i))
        .collect::<StoreResult<Vec<_>>>()?;
    if team_rows.concat() != expected_teams[4..] {
        return Err(invalid(
            "Lookup Teams changed before membership publication",
        ));
    }
    let count = tx.record_count(members_file())?;
    if count != 0 {
        let rows = (0..count)
            .map(|i| tx.read_record(members_file(), i))
            .collect::<StoreResult<Vec<_>>>()?;
        if rows.concat() != expected_members[4..] {
            return Err(invalid("Lookup memberships changed"));
        }
        tx.commit()?;
    } else {
        tx.commit_with_atomic_file_replacement(
            members_file(),
            &expected_members,
            "lookup-members",
        )?;
    }
    Ok((0..configs.len() as u32).collect())
}

/// Explicit per-repository keys returned by lookup publication. These are
/// caller context, never stored registration state or globally fixed indexes.
#[derive(Clone, Copy, Debug)]
pub struct LookupProfiles {
    pub unregistered_team_index: u32,
    pub registered_team_index: u32,
}
impl LookupProfiles {
    pub fn read(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        registered: bool,
    ) -> StoreResult<DirectoryLookup> {
        if self.unregistered_team_index == self.registered_team_index {
            return Err(invalid(
                "Distinct role profiles require distinct Team indexes",
            ));
        }
        read_directory_for_team(
            read,
            if registered {
                self.registered_team_index
            } else {
                self.unregistered_team_index
            },
        )
    }
}
