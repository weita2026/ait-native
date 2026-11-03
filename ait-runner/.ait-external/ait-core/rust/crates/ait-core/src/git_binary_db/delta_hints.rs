use super::GitObjectKind;
use std::collections::BTreeMap;

/// Disposable path metadata. It retains no body bytes and changes no Git identity.
#[derive(Default)]
pub struct GitObjectDeltaHints {
    paths: BTreeMap<(u32, [u8; 20]), String>,
}

impl GitObjectDeltaHints {
    pub fn observe(
        &mut self,
        repository: u32,
        kind: GitObjectKind,
        oid: [u8; 20],
        body: &[u8],
    ) -> String {
        let prefix = format!("git/{repository}/");
        if kind == GitObjectKind::Commit {
            if let Some(line) = body
                .split(|b| *b == b'\n')
                .next()
                .and_then(|line| line.strip_prefix(b"tree "))
            {
                if let Some(tree) = oid_text(line) {
                    self.paths
                        .entry((repository, tree))
                        .or_insert_with(|| format!("{prefix}tree/"));
                }
            }
        }
        let hint = self
            .paths
            .get(&(repository, oid))
            .cloned()
            .unwrap_or_else(|| format!("{prefix}{}/", kind.name()));
        if kind == GitObjectKind::Tree {
            let parent = hint
                .strip_prefix(&format!("{prefix}tree/"))
                .unwrap_or("")
                .to_string();
            let mut at = 0;
            while at < body.len() {
                let Some(space) = body[at..].iter().position(|b| *b == b' ') else {
                    break;
                };
                let space = at + space;
                let Some(nul) = body[space + 1..].iter().position(|b| *b == 0) else {
                    break;
                };
                let nul = space + 1 + nul;
                let Some(raw_oid) = body.get(nul + 1..nul + 21) else {
                    break;
                };
                let Ok(name) = std::str::from_utf8(&body[space + 1..nul]) else {
                    break;
                };
                let child: [u8; 20] = raw_oid.try_into().expect("checked tree OID");
                let tree = &body[at..space] == b"40000" || &body[at..space] == b"040000";
                let child_hint = format!(
                    "{prefix}{}/{parent}{name}{}",
                    if tree { "tree" } else { "blob" },
                    if tree { "/" } else { "" }
                );
                self.paths.entry((repository, child)).or_insert(child_hint);
                at = nul + 21;
            }
        }
        hint
    }
}

fn oid_text(raw: &[u8]) -> Option<[u8; 20]> {
    if raw.len() != 40 {
        return None;
    }
    let text = std::str::from_utf8(raw).ok()?;
    let mut oid = [0; 20];
    for (index, byte) in oid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(oid)
}
