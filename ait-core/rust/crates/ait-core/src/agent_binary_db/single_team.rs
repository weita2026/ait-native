//! One local team. Agent records own identity, role index and session binding.
use super::*;

pub const BASE_SEATS: [(u32, u32); 4] = [(0, 0), (7, 3), (2, 1), (1, 2)];

pub fn read(db: &LocalBinaryDbFs) -> StoreResult<Vec<Agent>> {
    let agents = read_agents(db)?;
    let mut sessions = BTreeSet::new();
    for agent in &agents {
        if !agent.session.is_empty() && !sessions.insert(agent.session.clone()) {
            return Err(invalid("Session belongs to multiple Agents"));
        }
    }
    Ok(agents)
}

/// Read only the existing bindings needed by standalone role pruning. Keep
/// workflow admission and a writable database handle out of this guard.
pub fn read_for_role_prune(
    repo_root: &std::path::Path,
    repo_name: &str,
) -> Result<Vec<Agent>, String> {
    use crate::binary_db::{AuthorityId, BinaryDbReadLockSet, LocalStateScope, StorePath};
    use crate::binary_db_generation::{
        binary_db_activation_lock_root, configured_repo_name, require_direct_authority_directory,
    };
    use std::fs;

    let root = repo_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let _activation = BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&root))
        .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    let authority = root.join(".ait/binary-db");
    require_direct_authority_directory(&authority)?;
    if authority
        .canonicalize()
        .map_err(|error| error.to_string())?
        != authority
        || configured_repo_name(&root)? != repo_name
    {
        return Err("Role-prune guard requires the canonical repository authority".into());
    }
    const FILES: &[&str] = &["agents.bin", "agents_payload.bin"];
    for leaf in FILES {
        match fs::symlink_metadata(authority.join(leaf)) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                return Err(format!(
                    "Role-prune Agent file {leaf} must be a direct file"
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let db = LocalBinaryDbFs::new(
        StorePath::from(authority),
        StorePath::from(root),
        AuthorityId::new(format!("local:{repo_name}")),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(FILES)
    .with_declared_index_paths(&[]);
    read(&db).map_err(|error| error.to_string())
}

pub fn ensure_pool(db: &LocalBinaryDbFs) -> StoreResult<()> {
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(agents_file())?;
    if count > ROSTER.len() as u32 {
        return Err(invalid(
            "Agent pool exceeds 50; explicit conversion is required",
        ));
    }
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    for (i, name) in ROSTER.iter().enumerate() {
        if (i as u32) < count {
            let row = write.read_record(agents_file(), i as u32)?;
            let (offset, len) = locator(&row)?;
            // Physical keys own identity. Existing names are display values,
            // including repository customizations; refresh only missing seats.
            Agent::decode(&row, &write.read_payload(payload_file(), offset, len)?)?;
            bytes.extend(row);
        } else {
            let agent = Agent {
                archived: false,
                role: None,
                name: (*name).into(),
                session: String::new(),
            };
            let payload = agent.payload()?;
            let range = write.append_payload(payload_file(), &payload)?;
            bytes.extend(agent.encode(range.payload_offset, payload.len() as u16)?);
        }
    }
    if count < ROSTER.len() as u32 {
        write.commit_with_atomic_file_replacement(agents_file(), &bytes, "agent-pool")?;
    } else {
        write.commit()?;
    }
    Ok(())
}

/// Session and responsibility are published together under the native write lock.
pub fn assign(
    db: &LocalBinaryDbFs,
    index: u32,
    expected: &Agent,
    role: u32,
    session: &str,
) -> StoreResult<()> {
    if session.is_empty() || role == u32::MAX {
        return Err(invalid("Invalid Agent assignment"));
    }
    let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
    let count = write.record_count(agents_file())?;
    if index >= count {
        return Err(invalid("Agent index out of range"));
    }
    let mut rows = Vec::new();
    for i in 0..count {
        let row = write.read_record(agents_file(), i)?;
        let (offset, len) = locator(&row)?;
        let agent = Agent::decode(&row, &write.read_payload(payload_file(), offset, len)?)?;
        if i != index && agent.session == session {
            return Err(invalid("Session already belongs to another Agent"));
        }
        if i == index && (agent.archived || agent != *expected) {
            return Err(invalid("Agent binding changed; reconcile before retrying"));
        }
        rows.push(row);
    }
    if expected.role == Some(role) && expected.session == session {
        return write.commit().map(|_| ());
    }
    let mut agent = expected.clone();
    agent.role = Some(role);
    agent.session = session.into();
    let (offset, len) = locator(&rows[index as usize])?;
    rows[index as usize] = if expected.session == session {
        agent.encode(offset, len as u16)?
    } else {
        let payload = agent.payload()?;
        let range = write.append_payload(payload_file(), &payload)?;
        agent.encode(range.payload_offset, payload.len() as u16)?
    };
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend(row);
    }
    write.commit_with_atomic_file_replacement(agents_file(), &bytes, "agent-assignment")?;
    Ok(())
}

pub const ROSTER: [&str; 50] = [
    "Newton",
    "Einstein",
    "Maxwell",
    "Curie",
    "Feynman",
    "Faraday",
    "Noether",
    "Niels Bohr",
    "Werner Heisenberg",
    "Erwin Schrodinger",
    "Max Planck",
    "Paul Dirac",
    "Wolfgang Pauli",
    "Louis de Broglie",
    "Enrico Fermi",
    "Ernest Rutherford",
    "J. J. Thomson",
    "James Chadwick",
    "Lise Meitner",
    "Otto Hahn",
    "Pierre Curie",
    "Henri Becquerel",
    "Wilhelm Rontgen",
    "Satyendra Nath Bose",
    "Subrahmanyan Chandrasekhar",
    "C. V. Raman",
    "Abdus Salam",
    "Steven Weinberg",
    "Sheldon Glashow",
    "Murray Gell-Mann",
    "George Zweig",
    "Robert Oppenheimer",
    "Hans Bethe",
    "John Bardeen",
    "Walter Brattain",
    "William Shockley",
    "Robert Millikan",
    "Albert Michelson",
    "Edward Morley",
    "Joseph Fourier",
    "Thomas Young",
    "Augustin Fresnel",
    "Christiaan Huygens",
    "Johannes Kepler",
    "Galileo Galilei",
    "Robert Hooke",
    "Henry Cavendish",
    "Charles Coulomb",
    "Andre-Marie Ampere",
    "Alessandro Volta",
];

#[cfg(test)]
#[path = "role_prune_tests.rs"]
mod role_prune_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pool_refresh_adds_missing_contacts_and_preserves_custom_names_roles_and_sessions() {
        let (_temp, db) = super::super::tests::fixture();
        let contacts = [
            Agent {
                archived: false,
                role: Some(0),
                name: "Repository PM".into(),
                session: "existing-caller".into(),
            },
            Agent {
                archived: false,
                role: Some(2),
                name: "Einstein".into(),
                session: "existing-reviewer".into(),
            },
        ];
        let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General).unwrap();
        let mut bytes = 1_u32.to_le_bytes().to_vec();
        for contact in &contacts {
            let payload = contact.payload().unwrap();
            let range = tx.append_payload(payload_file(), &payload).unwrap();
            bytes.extend(
                contact
                    .encode(range.payload_offset, payload.len() as u16)
                    .unwrap(),
            );
        }
        tx.commit_with_atomic_file_replacement(agents_file(), &bytes, "partial-directory-fixture")
            .unwrap();
        ensure_pool(&db).unwrap();
        let agents = read(&db).unwrap();
        assert_eq!(agents.len(), 50);
        assert_eq!(&agents[..2], &contacts);
        for (index, agent) in agents.iter().enumerate().skip(2) {
            assert_eq!(agent.name, ROSTER[index]);
            assert!(agent.session.is_empty());
        }
        ensure_pool(&db).unwrap();
        assert_eq!(read(&db).unwrap(), agents);
    }
}
