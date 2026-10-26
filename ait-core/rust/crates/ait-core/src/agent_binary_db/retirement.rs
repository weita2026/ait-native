//! Explicit retirement of previously emitted Team bit0, with no policy inference.
use super::layout::{
    read_agents_under, read_rows, validate_retired_team_metadata, verify_live_role_pins,
};
use super::*;

#[derive(Clone)]
pub struct TeamMetadataRetirement {
    source: Vec<Vec<u8>>,
    target: Vec<Vec<u8>>,
    dependencies: Vec<(BinaryFileId, Vec<Vec<u8>>)>,
}
impl TeamMetadataRetirement {
    pub fn capture(db: &LocalBinaryDbFs) -> StoreResult<Self> {
        let read = BinaryDbReadTxn::new(db);
        let agents = read_agents_under(&read)?;
        let roles = read_rows(&read, roles_file())?;
        for row in &roles {
            let pin = decode_role(row)?;
            if PlanRevisionCodec::<1>::decode_record(
                &read.read_record(PlanRevisionCodec::<1>::record_file(), pin)?,
            )?
            .is_tombstone()
            {
                return Err(invalid("Team retirement role revision was reclaimed"));
            }
        }
        let source = read_rows(&read, teams_file())?;
        let mut target = source.clone();
        for row in &mut target {
            let archived = validate_retired_team_metadata(row[0])?;
            let owner = agents
                .get(u32_at(row, 12) as usize)
                .ok_or_else(|| invalid("Unresolved Team owner"))?;
            let (offset, len) = locator(row)?;
            let name = read.read_payload(team_payload_file(), offset, len)?;
            let size = u16::from_le_bytes(row[16..18].try_into().unwrap());
            let concurrent = u16::from_le_bytes(row[18..20].try_into().unwrap());
            if row[1] != 0
                || name.is_empty()
                || std::str::from_utf8(&name).is_err()
                || size == 0
                || concurrent == 0
                || concurrent > size
                || (!archived && owner.archived)
            {
                return Err(invalid("Invalid Team source for bit retirement"));
            }
            row[0] &= !1;
        }
        let file = membership_file_for_read(&read)?;
        let members = read_rows(&read, file.clone())?;
        let mut pairs = BTreeSet::new();
        for row in &members {
            let role = if row.len() == 16 {
                Membership::decode(row)?.role
            } else {
                agents[u32_at(row, 8) as usize].role
            };
            if row[0] == 0 && role.is_some_and(|role| roles[role as usize][0] != 0) {
                return Err(invalid(
                    "Active membership role is archived during bit retirement",
                ));
            }
            if row[0] == 0
                && (source[u32_at(row, 4) as usize][0] & 128 != 0
                    || agents[u32_at(row, 8) as usize].archived
                    || !pairs.insert((u32_at(row, 4), u32_at(row, 8))))
            {
                return Err(invalid("Invalid active membership during bit retirement"));
            }
        }
        Ok(Self {
            source,
            target,
            dependencies: vec![
                (agents_file(), read_rows(&read, agents_file())?),
                (roles_file(), roles),
                (file, members),
            ],
        })
    }
    pub fn apply(&self, db: &LocalBinaryDbFs) -> StoreResult<()> {
        let mut tx = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
        verify_live_role_pins(&tx)?;
        for (file, rows) in &self.dependencies {
            if tx.record_count(file.clone())? != rows.len() as u32 {
                return Err(invalid("Team retirement dependencies changed"));
            }
            for (i, row) in rows.iter().enumerate() {
                if tx.read_record(file.clone(), i as u32)? != *row {
                    return Err(invalid("Team retirement dependencies changed"));
                }
            }
        }
        let current = (0..tx.record_count(teams_file())?)
            .map(|i| tx.read_record(teams_file(), i))
            .collect::<StoreResult<Vec<_>>>()?;
        if current == self.target {
            tx.commit()?;
            return Ok(());
        }
        if current != self.source {
            return Err(invalid("Team retirement source changed"));
        }
        let mut bytes = 1u32.to_le_bytes().to_vec();
        for row in &self.target {
            bytes.extend(row);
        }
        tx.commit_with_atomic_file_replacement(
            teams_file(),
            &bytes,
            "team-reserved-bit-retirement",
        )?;
        Ok(())
    }
}
