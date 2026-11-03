//! Bounded, one-time conversion of the original ait-core 100-Agent seed.
//! All edits occur in a captured generation; activation retains the source.
use super::generation_content_indexes::rebuild_content_identity_indexes;
use super::{
    capture_binary_db_generation_preserving_ordinals, seal_binary_db_generation,
    CaptureBinaryDbGenerationOptions, GenerationResult, PathBuf,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;

const SOURCE: [(&str, &str); 6] = [
    (
        "agents.bin",
        "c5173d735b52dcdbdd1631b58b4b5f137fdd4e441727951be147b75f77ecc886",
    ),
    (
        "agents_payload.bin",
        "b35b0ada457a83e6fef0baad54ab915bf8536da5b114e8a495d306d1f2200135",
    ),
    (
        "agent_roles.bin",
        "62ff5ba17cec3738fed32776d513eeb699a653b149d97cfb349bc46a928dcb44",
    ),
    (
        "agent_teams.bin",
        "c8159919c8b8434cb782616c1aee9a8d765555f3b7e5674764fca2f3667ff1f8",
    ),
    (
        "agent_teams_payload.bin",
        "d74c273b4ffee3722ab8ea2f2445a438ca3a59fb2c79446fa748bc9af82e1259",
    ),
    (
        "agent_team_membership.bin",
        "079dbc8522166b8ff6ebb653348648a379edccbf31db3829a887b05c23ba1bac",
    ),
];
const CHANGED: [&str; 4] = [
    "agents.bin",
    "agents_payload.bin",
    "agent_teams.bin",
    "agent_team_membership.bin",
];
const TARGET: [(&str, &str); 6] = [
    (
        "agents.bin",
        "e96ef850c7eb912117b2993bcfcfc91d899a1dbb95a229ae6c0028d3ad85a75c",
    ),
    (
        "agents_payload.bin",
        "6de0bcde8a76213ae894841cb72061a6f7cfa25ee35d35892cea199854dad34b",
    ),
    (
        "agent_roles.bin",
        "62ff5ba17cec3738fed32776d513eeb699a653b149d97cfb349bc46a928dcb44",
    ),
    (
        "agent_teams.bin",
        "1414bbd1c6c9846a7329442067706b21bbde21e754659724dcb570bc0593a7d5",
    ),
    (
        "agent_teams_payload.bin",
        "d74c273b4ffee3722ab8ea2f2445a438ca3a59fb2c79446fa748bc9af82e1259",
    ),
    (
        "agent_team_membership.bin",
        "e188ae795d53970a860224388a81dcd3d9974b6b30d2339a59c2cc89002d1aed",
    ),
];

#[derive(Clone, Debug)]
pub struct AgentPool50Options {
    pub repo_root: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct AgentPool50Report {
    pub generation_root: PathBuf,
    pub source_authority_fingerprint: String,
    pub content_fingerprint: String,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_seed(root: &std::path::Path) -> GenerationResult<Vec<Vec<u8>>> {
    let mut files = Vec::new();
    for &(name, expected) in &SOURCE {
        let bytes = fs::read(root.join(name)).map_err(|e| format!("{name}: {e}"))?;
        if hash(&bytes) != expected {
            return Err(format!(
                "Agent seed {name} differs from the authorized source; no conversion"
            ));
        }
        files.push(bytes);
    }
    Ok(files)
}

/// A retry is a no-op only for the exact validated target, never a partial
/// publication or an unrelated 50-Agent authority.
pub fn agent_pool_50_complete(repo_root: &std::path::Path) -> GenerationResult<bool> {
    let root = repo_root.canonicalize().map_err(|e| e.to_string())?;
    if root.file_name().and_then(|x| x.to_str()) != Some("ait-core") {
        return Err("The one-time Agent conversion is restricted to ait-core".into());
    }
    let authority = root.join(".ait/binary-db");
    let actual = SOURCE
        .iter()
        .map(|(name, _)| {
            fs::read(authority.join(name))
                .map(|bytes| hash(&bytes))
                .map_err(|e| e.to_string())
        })
        .collect::<GenerationResult<Vec<_>>>()?;
    if actual
        .iter()
        .zip(&SOURCE)
        .all(|(found, (_, expected))| found == expected)
    {
        return Ok(false);
    }
    if actual
        .iter()
        .zip(&TARGET)
        .all(|(found, (_, expected))| found == expected)
    {
        return Ok(true);
    }
    Err("Agent authority differs from both authorized source and validated target; no retry".into())
}

pub fn agent_pool_50_generation_valid(generation_root: &std::path::Path) -> GenerationResult<()> {
    for &(name, expected) in &TARGET {
        let bytes = fs::read(generation_root.join("local").join(name))
            .map_err(|e| format!("staged {name}: {e}"))?;
        if hash(&bytes) != expected {
            return Err(format!(
                "staged Agent file {name} differs from the validated target"
            ));
        }
    }
    super::validate_binary_db_generation(generation_root, "ait-core")
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated range"))
}

fn new_index(old: u32) -> Option<u32> {
    match old {
        0 => Some(0),
        6 => Some(1),
        1..=5 => Some(old + 1),
        7..=49 => Some(old),
        _ => None,
    }
}

fn convert(source: &[Vec<u8>]) -> GenerationResult<[Vec<u8>; 4]> {
    let (agents, payload, teams, members) = (&source[0], &source[1], &source[3], &source[5]);
    if agents.len() != 1604
        || teams.len() != 44
        || members.len() != 3204
        || agents[..4] != 1u32.to_le_bytes()
        || payload[..4] != 1u32.to_le_bytes()
        || teams[..4] != 1u32.to_le_bytes()
        || members[..4] != 1u32.to_le_bytes()
    {
        return Err("Agent seed layout does not match the authorized 100-Agent source".into());
    }
    let mut out_agents = 1u32.to_le_bytes().to_vec();
    let mut out_payload = 1u32.to_le_bytes().to_vec();
    let mut sessions = BTreeSet::new();
    for new in 0..50_u32 {
        let old = match new {
            0 => 0,
            1 => 6,
            2..=6 => new - 1,
            _ => new,
        };
        let at = 4 + old as usize * 16;
        let mut row = agents[at..at + 16].to_vec();
        let len = u16::from_le_bytes(row[2..4].try_into().unwrap()) as usize;
        let off = u64::from_le_bytes(row[4..12].try_into().unwrap()) as usize;
        let end = off.checked_add(len).ok_or("Agent payload range overflow")?;
        if off < 4 || end > payload.len() || len < 3 || row[0] != 0 || row[1] & !1 != 0 {
            return Err("Invalid retained Agent payload or metadata".into());
        }
        let value = &payload[off..end];
        let name_len = u16::from_le_bytes(value[..2].try_into().unwrap()) as usize;
        if name_len == 0
            || name_len + 2 > value.len()
            || std::str::from_utf8(&value[2..2 + name_len]).is_err()
        {
            return Err("Invalid retained Agent name".into());
        }
        let session = std::str::from_utf8(&value[2 + name_len..])
            .map_err(|_| "Invalid retained Agent session")?;
        if row[1] != u8::from(!session.is_empty())
            || (!session.is_empty() && !sessions.insert(session.to_owned()))
        {
            return Err("Invalid or duplicate retained Agent session".into());
        }
        let next_offset = out_payload.len() as u64;
        out_payload.extend(value);
        row[4..12].copy_from_slice(&next_offset.to_le_bytes());
        if new == 0 {
            row[12..16].copy_from_slice(&1u32.to_le_bytes()); // coordinator role 0
        } else if new == 1 {
            row[12..16].copy_from_slice(&3u32.to_le_bytes()); // reviewer role 2
        }
        out_agents.extend(row);
    }
    for old in 50..100 {
        let at = 4 + old * 16;
        let row = &agents[at..at + 16];
        if row[1] != 0 || row[0] != 0 {
            return Err("Removed Agent is bound or archived; conversion stopped".into());
        }
    }
    let mut out_teams = teams.clone();
    for team in 0..2 {
        let at = 4 + team * 20;
        if out_teams[at] != 0
            || u32_at(&out_teams, at + 12) != 0
            || out_teams[at + 16..at + 20] != [100, 0, 100, 0]
        {
            return Err("Existing Team does not match the authorized source".into());
        }
        out_teams[at + 16..at + 20].copy_from_slice(&[50, 0, 50, 0]);
    }
    let mut seen = [BTreeSet::new(), BTreeSet::new()];
    let mut reordered = [Vec::new(), Vec::new()];
    for i in 0..200 {
        let at = 4 + i * 16;
        let row = &members[at..at + 16];
        let team = u32_at(row, 4) as usize;
        let old = u32_at(row, 8);
        if team > 1 || old >= 100 || row[0] != 0 || row[1..4] != [0; 3] || !seen[team].insert(old) {
            return Err("Membership source is not the exact two-team seed".into());
        }
        if let Some(new) = new_index(old) {
            let mut copy = row.to_vec();
            copy[8..12].copy_from_slice(&new.to_le_bytes());
            if new == 0 {
                copy[12..16].copy_from_slice(&1u32.to_le_bytes());
            } else if new == 1 {
                copy[12..16].copy_from_slice(&3u32.to_le_bytes());
            }
            reordered[team].push(copy);
        }
    }
    if seen.iter().any(|team| team.len() != 100) || reordered.iter().any(|team| team.len() != 50) {
        return Err("Membership closure is incomplete".into());
    }
    let mut out_members = 1u32.to_le_bytes().to_vec();
    for team in &mut reordered {
        team.sort_by_key(|row| u32_at(row, 8));
        for row in team {
            out_members.extend_from_slice(row);
        }
    }
    Ok([out_agents, out_payload, out_teams, out_members])
}

pub fn stage_agent_pool_50(options: AgentPool50Options) -> GenerationResult<AgentPool50Report> {
    let root = options
        .repo_root
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if root.file_name().and_then(|x| x.to_str()) != Some("ait-core") {
        return Err("The one-time Agent conversion is restricted to ait-core".into());
    }
    if agent_pool_50_complete(&root)? {
        return Err("Agent pool is already converted; no generation needed".into());
    }
    let source = read_seed(&root.join(".ait/binary-db"))?;
    let converted = convert(&source)?;
    let capture =
        capture_binary_db_generation_preserving_ordinals(CaptureBinaryDbGenerationOptions {
            repo_root: root,
            output_root: options.output_root.clone(),
            jobs: 1,
        })?;
    let stage = options.output_root.join("local");
    read_seed(&stage)?; // The captured source must still be the approved source.
    for (name, bytes) in CHANGED.into_iter().zip(converted) {
        let path = stage.join(name);
        let mut file = fs::File::create(&path).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    // Identity indexes are derived acceleration. Rebuild in the private
    // generation before full validation; the captured source is untouched.
    rebuild_content_identity_indexes(&stage)?;
    let fingerprint = seal_binary_db_generation(&options.output_root)?;
    agent_pool_50_generation_valid(&options.output_root)?;
    Ok(AgentPool50Report {
        generation_root: options.output_root,
        source_authority_fingerprint: capture.source_authority_fingerprint,
        content_fingerprint: fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<Vec<u8>> {
        let mut agents = 1u32.to_le_bytes().to_vec();
        let mut payload = 1u32.to_le_bytes().to_vec();
        for i in 0..100 {
            let name = format!("Agent{i}");
            let mut value = (name.len() as u16).to_le_bytes().to_vec();
            value.extend(name.as_bytes());
            let mut row = [0u8; 16];
            row[2..4].copy_from_slice(&(value.len() as u16).to_le_bytes());
            row[4..12].copy_from_slice(&(payload.len() as u64).to_le_bytes());
            agents.extend(row);
            payload.extend(value);
        }
        let mut teams = 1u32.to_le_bytes().to_vec();
        for _ in 0..2 {
            let mut row = [0u8; 20];
            row[16..20].copy_from_slice(&[100, 0, 100, 0]);
            teams.extend(row);
        }
        let mut members = 1u32.to_le_bytes().to_vec();
        for team in 0..2u32 {
            for agent in 0..100u32 {
                let mut row = [0u8; 16];
                row[4..8].copy_from_slice(&team.to_le_bytes());
                row[8..12].copy_from_slice(&agent.to_le_bytes());
                row[12..16].copy_from_slice(&2u32.to_le_bytes());
                members.extend(row);
            }
        }
        vec![agents, payload, vec![], teams, vec![], members]
    }
    #[test]
    fn index_mapping_is_bijective_over_retained_pool() {
        let mapped = (0..50)
            .map(|i| new_index(i).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(mapped, (0..50).collect());
        assert!((50..100).all(|i| new_index(i).is_none()));
    }
    #[test]
    fn conversion_keeps_exact_references_and_pins_lead_and_reviewer() {
        let output = convert(&fixture()).unwrap();
        assert_eq!(
            output.iter().map(Vec::len).collect::<Vec<_>>(),
            [804, 444, 44, 1604]
        );
        for (index, expected) in [(0, "Agent0"), (1, "Agent6"), (2, "Agent1")] {
            let row = &output[0][4 + index * 16..4 + (index + 1) * 16];
            let off = u64::from_le_bytes(row[4..12].try_into().unwrap()) as usize;
            let len = u16::from_le_bytes(row[2..4].try_into().unwrap()) as usize;
            let payload = &output[1][off..off + len];
            assert_eq!(&payload[2..], expected.as_bytes());
        }
        assert_eq!(u32_at(&output[0], 4 + 12), 1);
        assert_eq!(u32_at(&output[0], 4 + 16 + 12), 3);
        for team in 0..2 {
            assert_eq!(
                &output[2][4 + team * 20 + 16..4 + team * 20 + 20],
                &[50, 0, 50, 0]
            );
            for agent in 0..50 {
                let row =
                    &output[3][4 + (team * 50 + agent) * 16..4 + (team * 50 + agent + 1) * 16];
                assert_eq!(u32_at(row, 4), team as u32);
                assert_eq!(u32_at(row, 8), agent as u32);
                if agent == 0 || agent == 1 {
                    assert_eq!(u32_at(row, 12), if agent == 0 { 1 } else { 3 });
                }
            }
        }
    }
    #[test]
    fn bound_removed_agent_stops_conversion() {
        let mut source = fixture();
        source[0][4 + 50 * 16 + 1] = 1;
        assert!(convert(&source).unwrap_err().contains("bound or archived"));
    }
}
