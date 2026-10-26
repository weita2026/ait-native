//! Consistent directory reads. Multiple Teams require explicit index selection.
use super::*;

pub enum DirectoryLookup {
    Missing,
    Ambiguous,
    Found(TeamDirectory),
}
pub struct TeamDirectory {
    pub team_index: u32,
    pub name: String,
    pub owner_agent_index: u32,
    pub target_member_count: u16,
    pub max_concurrent_members: u16,
    pub members: Vec<DirectoryMember>,
}
pub struct DirectoryMember {
    pub agent_index: u32,
    pub agent: Agent,
    /// Effective role currently loaded by the Agent, used by existing finish.
    pub role_revision_index: Option<u32>,
    pub role_archived: bool,
    /// Per-Team configuration; does not mutate or impersonate a loaded role.
    pub configured_role_index: Option<u32>,
    pub configured_role_revision_index: Option<u32>,
    pub configured_role_archived: bool,
}
pub fn read_directory(read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>) -> StoreResult<DirectoryLookup> {
    match read.record_count(teams_file())? {
        0 => Ok(DirectoryLookup::Missing),
        1 => read_directory_for_team(read, 0),
        _ => Ok(DirectoryLookup::Ambiguous),
    }
}
pub fn read_directory_for_team(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    team_index: u32,
) -> StoreResult<DirectoryLookup> {
    let team_count = read.record_count(teams_file())?;
    if team_index >= team_count {
        return Ok(DirectoryLookup::Missing);
    }
    let team = read.read_record(teams_file(), team_index)?;
    layout::validate_retired_team_metadata(team[0])?;
    if team[1] != 0 {
        return Err(invalid("Invalid AgentTeam metadata"));
    }
    if team[0] & 128 != 0 {
        return Ok(DirectoryLookup::Missing);
    }
    let target = u16::from_le_bytes(team[16..18].try_into().unwrap());
    let concurrent = u16::from_le_bytes(team[18..20].try_into().unwrap());
    if target == 0 || concurrent == 0 || concurrent > target {
        return Err(invalid("Invalid AgentTeam size"));
    }
    let (offset, len) = locator(&team)?;
    let name = String::from_utf8(read.read_payload(team_payload_file(), offset, len)?)
        .map_err(|_| invalid("Invalid AgentTeam name UTF-8"))?;
    if name.is_empty() {
        return Err(invalid("Empty AgentTeam name"));
    }
    let owner = u32_at(&team, 12);
    let mut agents = Vec::new();
    let mut sessions = BTreeSet::new();
    for i in 0..read.record_count(agents_file())? {
        let row = read.read_record(agents_file(), i)?;
        let (offset, len) = locator(&row)?;
        let agent = Agent::decode(&row, &read.read_payload(payload_file(), offset, len)?)?;
        if !agent.session.is_empty() && !sessions.insert(agent.session.clone()) {
            return Ok(DirectoryLookup::Ambiguous);
        }
        agents.push(agent);
    }
    if agents
        .get(owner as usize)
        .ok_or_else(|| invalid("Team owner reference is unresolved"))?
        .archived
    {
        return Ok(DirectoryLookup::Missing);
    }
    let file = membership_file_for_read(read)?;
    let mut active = BTreeSet::new();
    let mut members = Vec::new();
    let role_projection = |role: Option<u32>| -> StoreResult<(Option<u32>, bool)> {
        if let Some(role) = role {
            let row = read.read_record(roles_file(), role)?;
            Ok((Some(decode_role(&row)?), row[0] != 0))
        } else {
            Ok((None, false))
        }
    };
    for i in 0..read.record_count(file.clone())? {
        let row = read.read_record(file.clone(), i)?;
        if u32_at(&row, 4) != team_index || row[0] != 0 {
            continue;
        }
        let agent_index = u32_at(&row, 8);
        let agent = agents[agent_index as usize].clone();
        if agent.archived {
            return Err(invalid("Active membership references archived Agent"));
        }
        if !active.insert(agent_index) {
            return Ok(DirectoryLookup::Ambiguous);
        }
        let (role_revision_index, role_archived) = role_projection(agent.role)?;
        let configured_role_index = if row.len() == 16 {
            Membership::decode(&row)?.role
        } else {
            agent.role
        };
        let (configured_role_revision_index, configured_role_archived) =
            role_projection(configured_role_index)?;
        // Physical membership order is the stored seat order, not Agent ID.
        members.push(DirectoryMember {
            agent_index,
            agent,
            role_revision_index,
            role_archived,
            configured_role_index,
            configured_role_revision_index,
            configured_role_archived,
        });
    }
    Ok(DirectoryLookup::Found(TeamDirectory {
        team_index,
        name,
        owner_agent_index: owner,
        target_member_count: target,
        max_concurrent_members: concurrent,
        members,
    }))
}
