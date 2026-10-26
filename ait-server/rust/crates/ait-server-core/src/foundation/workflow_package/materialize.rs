use super::*;
use crate::foundation::pack_substrate::{
    read_pack_index, read_tree_pack_index, read_tree_pack_tree, read_tree_pack_tree_by_ordinal,
    PackEntryArchive, MAX_DELTA_CHAIN_READ_DEPTH, PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::foundation::snapshot_materialization::{
    prepare_exact_snapshot_paths, validate_snapshot_tree_file, write_snapshot_tree_file,
};

/// Existing immutable main-seed content. Package members always take precedence;
/// this interface reads content only and cannot publish workflow metadata.
pub trait WorkflowPackageBaseContent {
    fn snapshot(&self, id: &str) -> Result<Value, String>;
    fn root_tree(&self, pack: &str, ordinal: u32) -> Result<String, String>;
    fn tree(&self, id: &str) -> Result<Value, String>;
    fn blob(&self, id: &str) -> Result<Vec<u8>, String>;
}

struct FileEntry {
    path: String,
    mode: String,
    blob: String,
    size: u64,
}

#[derive(Default)]
struct PackageReadCache {
    locators: BTreeMap<String, Value>,
    tree_packs: BTreeMap<String, String>,
    trees: BTreeMap<String, Value>,
    archives: BTreeMap<String, PackEntryArchive>,
    delta_bases: BTreeMap<String, Vec<u8>>,
}

impl WorkflowPackage {
    pub fn materialize_final_tree(
        &self,
        destination: &Path,
        base: &impl WorkflowPackageBaseContent,
    ) -> Result<(), String> {
        self.manifest.validate()?;
        let mut cache = self.prepare_read_cache()?;
        let revision = &self.manifest.revision_snapshot_id;
        let snapshot = match self
            .manifest
            .content
            .snapshots
            .iter()
            .find(|row| row["snapshot_id"].as_str() == Some(revision))
        {
            Some(row) => row.clone(),
            None => base.snapshot(revision)?,
        };
        let pack = text(&snapshot, "root_tree_pack_id")?;
        let ordinal = u32::try_from(number(&snapshot, "root_entry_ordinal")?)
            .map_err(|_| "Root tree ordinal exceeds u32")?;
        let root = if self
            .manifest
            .content
            .tree_packs
            .iter()
            .any(|row| row["pack_id"].as_str() == Some(pack))
        {
            let path = self.member_path("trees", pack)?;
            text(
                &read_tree_pack_tree_by_ordinal(path_text(&path)?, ordinal as usize)?,
                "tree_id",
            )?
            .to_string()
        } else {
            base.root_tree(pack, ordinal)?
        };
        let mut files = Vec::new();
        self.collect_files(
            &root,
            "",
            base,
            &mut BTreeSet::new(),
            &mut files,
            &mut cache,
        )?;
        let bytes = files.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.size)
                .ok_or("Package tree byte count overflows")
        })?;
        if number(&snapshot, "file_count")? != files.len() as u64
            || number(&snapshot, "total_bytes")? != bytes
        {
            return Err("Package final Snapshot counts differ from its exact tree".into());
        }
        prepare_exact_snapshot_paths(
            destination,
            files
                .iter()
                .map(|entry| (entry.path.as_str(), entry.mode.as_str())),
        )?;
        for entry in files {
            let bytes = self.read_blob(&entry.blob, base, &mut BTreeSet::new(), &mut cache)?;
            if bytes.len() as u64 != entry.size {
                return Err(format!("Package tree size differs for {}", entry.path));
            }
            write_snapshot_tree_file(destination, &entry.path, &entry.mode, &bytes)?;
        }
        Ok(())
    }

    /// Validate the required original and tested content closure without a
    /// worktree or authoritative writes. History-only members count too.
    pub fn validate_content_closure(
        &self,
        base: &impl WorkflowPackageBaseContent,
    ) -> Result<(), String> {
        let mut cache = self.prepare_read_cache()?;
        let mut blobs = cache.locators.keys().cloned().collect::<BTreeSet<_>>();
        let mut shapes = BTreeMap::<String, Vec<FileEntry>>::new();
        let mut snapshots = self
            .manifest
            .content
            .snapshots
            .iter()
            .map(|row| Ok((text(row, "snapshot_id")?.to_string(), row.clone())))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut required = BTreeSet::from([
            self.manifest.base_snapshot_id.clone(),
            self.manifest.revision_snapshot_id.clone(),
        ]);
        for entry in &self.manifest.history_entries {
            required.insert(entry.pre_land_target_snapshot_id.clone());
            required.insert(entry.landed_snapshot_id.clone());
            required.extend(
                entry
                    .snapshots
                    .iter()
                    .map(|snapshot| snapshot.snapshot_id.clone()),
            );
            for change in &entry.changes {
                required.insert(
                    change.change["fork_snapshot_id"]
                        .as_str()
                        .ok_or("Change fork missing")?
                        .to_string(),
                );
            }
        }
        for id in required {
            if let std::collections::btree_map::Entry::Vacant(entry) = snapshots.entry(id.clone()) {
                entry.insert(base.snapshot(&id)?);
            }
        }
        for snapshot in snapshots.values() {
            let pack = text(snapshot, "root_tree_pack_id")?;
            let ordinal = u32::try_from(number(snapshot, "root_entry_ordinal")?)
                .map_err(|_| "Root ordinal exceeds u32")?;
            let root = if self
                .manifest
                .content
                .tree_packs
                .iter()
                .any(|p| p["pack_id"].as_str() == Some(pack))
            {
                let path = self.member_path("trees", pack)?;
                text(
                    &read_tree_pack_tree_by_ordinal(path_text(&path)?, ordinal as usize)?,
                    "tree_id",
                )?
                .to_string()
            } else {
                base.root_tree(pack, ordinal)?
            };
            let mut files = Vec::new();
            self.collect_files(
                &root,
                "",
                base,
                &mut BTreeSet::new(),
                &mut files,
                &mut cache,
            )?;
            let size = files.iter().try_fold(0u64, |n, f| {
                n.checked_add(f.size).ok_or("Snapshot size overflows")
            })?;
            if number(snapshot, "file_count")? != files.len() as u64
                || number(snapshot, "total_bytes")? != size
            {
                return Err("Original/tested Snapshot counts differ from its exact tree".into());
            }
            for file in files {
                blobs.insert(file.blob.clone());
                shapes.entry(file.blob.clone()).or_default().push(file);
            }
        }
        for blob in blobs {
            let bytes = self.read_blob(&blob, base, &mut BTreeSet::new(), &mut cache)?;
            for shape in shapes.remove(&blob).into_iter().flatten() {
                if shape.size != bytes.len() as u64 {
                    return Err("Original/tested Tree Blob size differs".into());
                }
                validate_snapshot_tree_file(&shape.path, &shape.mode, &bytes)?;
            }
        }
        Ok(())
    }

    fn prepare_read_cache(&self) -> Result<PackageReadCache, String> {
        let mut cache = PackageReadCache::default();
        // Validate each declared archive's real identity before interpreting any
        // locator, rather than trusting a filename or a client pack-index copy.
        for (rows, directory, tree) in [
            (&self.manifest.content.object_packs, "objects", false),
            (&self.manifest.content.tree_packs, "trees", true),
        ] {
            for row in rows {
                let id = text(row, "pack_id")?;
                let path = self.member_path(directory, id)?;
                let index = if tree {
                    read_tree_pack_index(path_text(&path)?)?
                } else {
                    read_pack_index(path_text(&path)?)?
                };
                if text(&index, "pack_id")? != id {
                    return Err("Package pack identity differs from its declared ID".into());
                }
                if tree {
                    for entry in index["trees"]
                        .as_array()
                        .ok_or("Tree pack entries missing")?
                    {
                        cache
                            .tree_packs
                            .entry(text(entry, "tree_id")?.into())
                            .or_insert_with(|| id.to_string());
                    }
                } else {
                    for entry in index["entries"]
                        .as_array()
                        .ok_or("Object pack entries missing")?
                    {
                        let blob = text(entry, "blob_id")?;
                        cache.locators.entry(blob.into()).or_insert_with(|| serde_json::json!({"blob_id":blob,"pack_id":id,"pack_entry_name":format!("blobs/{blob}"),"size_bytes":entry["uncompressed_byte_length"],"sha256":entry["checksum"],"pack_base_blob_id":entry["base_blob_id"]}));
                    }
                }
                if let Some(expected) = row.get("pack_index") {
                    // Existing zstd-bulk typed rows omit reader-derived fields.
                    // Match that complete semantic projection, or the complete
                    // native index, rather than accepting arbitrary subsets.
                    if expected != &index && expected != &bulk_pack_index_projection(&index, tree)?
                    {
                        return Err("Package pack_index differs from its immutable bytes".into());
                    }
                }
            }
        }
        for locator in &self.manifest.content.blob_locators {
            cache
                .locators
                .insert(text(locator, "blob_id")?.into(), locator.clone());
        }
        for locator in &self.manifest.content.tree_locators {
            cache.tree_packs.insert(
                text(locator, "tree_id")?.into(),
                text(locator, "tree_pack_id")?.into(),
            );
        }
        Ok(cache)
    }

    fn member_path(&self, directory: &str, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
            return Err("Package pack ID is not a filename".into());
        }
        let path = self.inputs.join(directory).join(id);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Missing package pack {id}: {error}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Package pack is not a regular file".into());
        }
        Ok(path)
    }

    fn collect_files(
        &self,
        id: &str,
        prefix: &str,
        base: &impl WorkflowPackageBaseContent,
        active: &mut BTreeSet<String>,
        files: &mut Vec<FileEntry>,
        cache: &mut PackageReadCache,
    ) -> Result<(), String> {
        if active.len() >= 1024 || !active.insert(id.to_string()) {
            return Err("Package tree closure is cyclic or too deep".into());
        }
        let rows = if let Some(rows) = cache.trees.get(id) {
            rows.clone()
        } else {
            let rows = if let Some(pack) = cache.tree_packs.get(id) {
                read_tree_pack_tree(path_text(&self.member_path("trees", pack)?)?, id)?
            } else {
                base.tree(id)?
            };
            cache.trees.insert(id.to_string(), rows.clone());
            rows
        };
        let mut names = BTreeSet::new();
        for row in rows
            .as_array()
            .ok_or("Package tree entries must be an array")?
        {
            let name = text(row, "entry_name")?;
            if name.is_empty()
                || name.contains(['/', '\\'])
                || matches!(name, "." | "..")
                || !names.insert(name)
            {
                return Err("Package tree contains duplicate or invalid entry names".into());
            }
            let path = if prefix.is_empty() {
                name.into()
            } else {
                format!("{prefix}/{name}")
            };
            match text(row, "entry_type")? {
                "tree" => {
                    self.collect_files(text(row, "target_id")?, &path, base, active, files, cache)?
                }
                "blob" => files.push(FileEntry {
                    path,
                    blob: text(row, "target_id")?.into(),
                    mode: text(row, "mode")?.into(),
                    size: number(row, "size_bytes")?,
                }),
                _ => return Err("Package tree has an unsupported entry type".into()),
            }
        }
        active.remove(id);
        Ok(())
    }

    fn read_blob(
        &self,
        id: &str,
        base: &impl WorkflowPackageBaseContent,
        active: &mut BTreeSet<String>,
        cache: &mut PackageReadCache,
    ) -> Result<Vec<u8>, String> {
        if let Some(bytes) = cache.delta_bases.get(id) {
            return Ok(bytes.clone());
        }
        if active.len() >= MAX_DELTA_CHAIN_READ_DEPTH || !active.insert(id.to_string()) {
            return Err("Package blob delta closure is cyclic or too deep".into());
        }
        let locator = cache.locators.get(id).cloned();
        let bytes = if let Some(locator) = &locator {
            let mut bases = BTreeMap::new();
            if let Some(base_id) = locator
                .get("pack_base_blob_id")
                .filter(|value| !value.is_null())
            {
                let base_id = base_id
                    .as_str()
                    .ok_or("Package delta base ID must be a string")?;
                let bytes = self.read_blob(base_id, base, active, cache)?;
                cache.delta_bases.insert(base_id.to_string(), bytes.clone());
                bases.insert(base_id.to_string(), bytes);
            }
            let pack = text(locator, "pack_id")?;
            if !cache.archives.contains_key(pack) {
                let path = self.member_path("objects", pack)?;
                cache.archives.insert(
                    pack.into(),
                    PackEntryArchive::open_with_format(
                        path_text(&path)?,
                        PACK_FORMAT_ZSTD_CHUNKED_V1,
                    )?,
                );
            }
            let bytes = cache
                .archives
                .get_mut(pack)
                .expect("opened package archive")
                .read_entry(
                    text(locator, "pack_entry_name")?,
                    Some(&bases),
                    MAX_DELTA_CHAIN_READ_DEPTH,
                )?;
            if bytes.len() as u64 != number(locator, "size_bytes")?
                || format!("{:x}", Sha256::digest(&bytes)) != text(locator, "sha256")?
            {
                return Err(format!("Package blob {id} checksum or size differs"));
            }
            bytes
        } else {
            base.blob(id)?
        };
        active.remove(id);
        Ok(bytes)
    }
}

fn bulk_pack_index_projection(index: &Value, tree: bool) -> Result<Value, String> {
    let (count, members, fields): (&str, &str, &[&str]) = if tree {
        (
            "tree_count",
            "trees",
            &["tree_id", "entry_ordinal", "entry_count", "checksum"],
        )
    } else {
        (
            "member_count",
            "entries",
            &[
                "entry_name",
                "blob_id",
                "entry_type",
                "checksum",
                "chain_depth",
            ],
        )
    };
    let mut projected = Map::new();
    for field in ["pack_id", "pack_format", count, "total_bytes"] {
        projected.insert(
            field.into(),
            index
                .get(field)
                .ok_or("Native pack index field is missing")?
                .clone(),
        );
    }
    let rows = index[members]
        .as_array()
        .ok_or("Native pack index members are missing")?
        .iter()
        .map(|row| {
            let mut entry = Map::new();
            for field in fields {
                entry.insert(
                    (*field).into(),
                    row.get(*field)
                        .ok_or("Native pack member field is missing")?
                        .clone(),
                );
            }
            if !tree {
                if let Some(base) = row.get("base_blob_id").filter(|value| !value.is_null()) {
                    entry.insert("base_blob_id".into(), base.clone());
                }
            }
            Ok(Value::Object(entry))
        })
        .collect::<Result<Vec<_>, String>>()?;
    projected.insert(members.into(), Value::Array(rows));
    Ok(Value::Object(projected))
}

fn text<'a>(row: &'a Value, key: &str) -> Result<&'a str, String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("Package content requires {key}"))
}
fn number(row: &Value, key: &str) -> Result<u64, String> {
    row.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Package content requires unsigned {key}"))
}
fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Package pack path is not UTF-8".into())
}
