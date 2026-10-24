use super::*;

impl RepositoryPackInventory {
    pub fn validate_zstd_only_formats(&self) -> Result<(), String> {
        if self.repo_name.trim().is_empty() {
            return Err("Repository pack inventory requires repo_name.".to_string());
        }

        for row in &self.object_packs {
            if row.pack_format != PackFormatKind::ZstdChunkedV1 {
                return Err(format!(
                    "Repository {} rejects unsupported object pack {} format {}.",
                    self.repo_name,
                    row.pack_id,
                    row.pack_format.persisted_name()
                ));
            }
        }

        for row in &self.tree_packs {
            if row.pack_format != TreePackFormatKind::ZstdChunkedTreeV1 {
                return Err(format!(
                    "Repository {} rejects unsupported tree pack {} format {}.",
                    self.repo_name,
                    row.pack_id,
                    row.pack_format.persisted_name()
                ));
            }
        }

        Ok(())
    }

    pub fn validate_zstd_only(&self) -> Result<(), String> {
        self.validate_zstd_only_formats()?;

        let object_packs = self.validate_object_packs()?;
        let tree_packs = self.validate_tree_packs()?;
        self.validate_blob_locators(&object_packs)?;
        let tree_locators = self.validate_tree_locators(&tree_packs)?;
        self.validate_snapshots(&tree_packs, &tree_locators)?;
        self.validate_line_heads()?;

        Ok(())
    }

    fn validate_object_packs(
        &self,
    ) -> Result<BTreeMap<String, &RepositoryObjectPackInventoryRow>, String> {
        let mut object_packs = BTreeMap::new();
        for row in &self.object_packs {
            require_non_empty(&row.pack_id, "object pack id")?;
            require_non_empty(&row.status, "object pack status")?;
            if row.status != "ready" {
                return Err(format!(
                    "Object pack {} is not ready for zstd-only inventory validation.",
                    row.pack_id
                ));
            }
            require_non_empty(&row.pack_path, "object pack path")?;
            require_non_empty(&row.pack_index_entry_name, "object pack index entry name")?;
            require_non_empty(&row.pack_index_checksum, "object pack index checksum")?;
            validate_optional_owner(
                &self.repo_name,
                "object pack",
                &row.pack_id,
                &row.repo_name,
                &row.repo_id,
            )?;

            if row.embedded_index.pack_id != row.pack_id {
                return Err(format!(
                    "Object pack {} embedded index pack id mismatch: {}.",
                    row.pack_id, row.embedded_index.pack_id
                ));
            }
            if row.embedded_index.pack_format != row.pack_format {
                return Err(format!(
                    "Object pack {} embedded index format mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.member_count != row.member_count {
                return Err(format!(
                    "Object pack {} embedded index member count mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.total_bytes != row.total_bytes {
                return Err(format!(
                    "Object pack {} embedded index byte total mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.entries.len() as i64 != row.member_count {
                return Err(format!(
                    "Object pack {} member count does not match embedded entry count.",
                    row.pack_id
                ));
            }

            let mut entry_names = BTreeSet::new();
            let mut blob_ids = BTreeSet::new();
            for entry in &row.embedded_index.entries {
                require_non_empty(&entry.entry_name, "object pack entry name")?;
                require_non_empty(&entry.blob_id, "object pack entry blob id")?;
                require_non_empty(&entry.entry_type, "object pack entry type")?;
                require_non_empty(&entry.checksum, "object pack entry checksum")?;
                if entry.chain_depth < 0 {
                    return Err(format!(
                        "Object pack {} entry {} has negative chain depth.",
                        row.pack_id, entry.entry_name
                    ));
                }
                if !entry_names.insert(entry.entry_name.as_str()) {
                    return Err(format!(
                        "Object pack {} has duplicate entry name {}.",
                        row.pack_id, entry.entry_name
                    ));
                }
                if !blob_ids.insert(entry.blob_id.as_str()) {
                    return Err(format!(
                        "Object pack {} has duplicate blob id {}.",
                        row.pack_id, entry.blob_id
                    ));
                }
            }

            if object_packs.insert(row.pack_id.clone(), row).is_some() {
                return Err(format!("Duplicate object pack id {}.", row.pack_id));
            }
        }
        Ok(object_packs)
    }

    fn validate_tree_packs(
        &self,
    ) -> Result<BTreeMap<String, &RepositoryTreePackInventoryRow>, String> {
        let mut tree_packs = BTreeMap::new();
        for row in &self.tree_packs {
            require_non_empty(&row.pack_id, "tree pack id")?;
            require_non_empty(&row.status, "tree pack status")?;
            if row.status != "ready" {
                return Err(format!(
                    "Tree pack {} is not ready for zstd-only inventory validation.",
                    row.pack_id
                ));
            }
            require_non_empty(&row.pack_path, "tree pack path")?;
            require_non_empty(&row.pack_index_entry_name, "tree pack index entry name")?;
            require_non_empty(&row.pack_index_checksum, "tree pack index checksum")?;
            validate_optional_owner(
                &self.repo_name,
                "tree pack",
                &row.pack_id,
                &row.repo_name,
                &row.repo_id,
            )?;

            if row.embedded_index.pack_id != row.pack_id {
                return Err(format!(
                    "Tree pack {} embedded index pack id mismatch: {}.",
                    row.pack_id, row.embedded_index.pack_id
                ));
            }
            if row.embedded_index.pack_format != row.pack_format {
                return Err(format!(
                    "Tree pack {} embedded index format mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.tree_count != row.tree_count {
                return Err(format!(
                    "Tree pack {} embedded index tree count mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.total_bytes != row.total_bytes {
                return Err(format!(
                    "Tree pack {} embedded index byte total mismatch.",
                    row.pack_id
                ));
            }
            if row.embedded_index.trees.len() as i64 != row.tree_count {
                return Err(format!(
                    "Tree pack {} tree count does not match embedded entry count.",
                    row.pack_id
                ));
            }

            let mut ordinals = BTreeSet::new();
            let mut tree_ids = BTreeSet::new();
            for entry in &row.embedded_index.trees {
                require_non_empty(&entry.tree_id, "tree pack entry tree id")?;
                require_non_empty(&entry.checksum, "tree pack entry checksum")?;
                if entry.entry_ordinal < 0 {
                    return Err(format!(
                        "Tree pack {} tree {} has negative entry ordinal.",
                        row.pack_id, entry.tree_id
                    ));
                }
                if entry.entry_count < 0 {
                    return Err(format!(
                        "Tree pack {} tree {} has negative entry count.",
                        row.pack_id, entry.tree_id
                    ));
                }
                if !ordinals.insert(entry.entry_ordinal) {
                    return Err(format!(
                        "Tree pack {} has duplicate entry ordinal {}.",
                        row.pack_id, entry.entry_ordinal
                    ));
                }
                if !tree_ids.insert(entry.tree_id.as_str()) {
                    return Err(format!(
                        "Tree pack {} has duplicate tree id {}.",
                        row.pack_id, entry.tree_id
                    ));
                }
            }

            if tree_packs.insert(row.pack_id.clone(), row).is_some() {
                return Err(format!("Duplicate tree pack id {}.", row.pack_id));
            }
        }
        Ok(tree_packs)
    }

    fn validate_blob_locators(
        &self,
        object_packs: &BTreeMap<String, &RepositoryObjectPackInventoryRow>,
    ) -> Result<(), String> {
        let mut blob_ids = BTreeSet::new();
        for locator in &self.blob_locators {
            require_non_empty(&locator.blob_id, "blob locator blob id")?;
            require_non_empty(&locator.sha256, "blob locator sha256")?;
            require_non_empty(&locator.pack_id, "blob locator pack id")?;
            require_non_empty(&locator.pack_entry_name, "blob locator pack entry name")?;
            require_non_empty(&locator.pack_entry_type, "blob locator pack entry type")?;
            if locator.size_bytes < 0 {
                return Err(format!("Blob {} has negative size.", locator.blob_id));
            }
            if locator.pack_chain_depth < 0 {
                return Err(format!(
                    "Blob {} has negative pack chain depth.",
                    locator.blob_id
                ));
            }
            if !blob_ids.insert(locator.blob_id.as_str()) {
                return Err(format!("Duplicate blob locator {}.", locator.blob_id));
            }

            let pack = object_packs.get(&locator.pack_id).ok_or_else(|| {
                format!(
                    "Blob {} references unknown object pack {}.",
                    locator.blob_id, locator.pack_id
                )
            })?;
            let entry = pack
                .embedded_index
                .entries
                .iter()
                .find(|entry| entry.blob_id == locator.blob_id)
                .ok_or_else(|| {
                    format!(
                        "Blob {} is missing from object pack {} embedded index.",
                        locator.blob_id, locator.pack_id
                    )
                })?;
            if entry.entry_name != locator.pack_entry_name {
                return Err(format!(
                    "Blob {} locator entry name does not match embedded index.",
                    locator.blob_id
                ));
            }
            if entry.entry_type != locator.pack_entry_type {
                return Err(format!(
                    "Blob {} locator entry type does not match embedded index.",
                    locator.blob_id
                ));
            }
            if entry.checksum != locator.sha256 {
                return Err(format!(
                    "Blob {} locator checksum does not match embedded index.",
                    locator.blob_id
                ));
            }
            if entry.base_blob_id != locator.pack_base_blob_id {
                return Err(format!(
                    "Blob {} locator base blob does not match embedded index.",
                    locator.blob_id
                ));
            }
            if entry.chain_depth != locator.pack_chain_depth {
                return Err(format!(
                    "Blob {} locator chain depth does not match embedded index.",
                    locator.blob_id
                ));
            }
        }
        Ok(())
    }

    fn validate_tree_locators(
        &self,
        tree_packs: &BTreeMap<String, &RepositoryTreePackInventoryRow>,
    ) -> Result<BTreeMap<String, &RepositoryTreeLocatorInventoryRow>, String> {
        let mut tree_locators = BTreeMap::new();
        for locator in &self.tree_locators {
            require_non_empty(&locator.tree_id, "tree locator tree id")?;
            require_non_empty(&locator.tree_pack_id, "tree locator pack id")?;
            require_non_empty(&locator.tree_pack_checksum, "tree locator checksum")?;
            if locator.entry_count < 0 {
                return Err(format!(
                    "Tree {} has negative entry count.",
                    locator.tree_id
                ));
            }
            let pack = tree_packs.get(&locator.tree_pack_id).ok_or_else(|| {
                format!(
                    "Tree {} references unknown tree pack {}.",
                    locator.tree_id, locator.tree_pack_id
                )
            })?;
            let entry = pack
                .embedded_index
                .trees
                .iter()
                .find(|entry| entry.tree_id == locator.tree_id)
                .ok_or_else(|| {
                    format!(
                        "Tree {} is missing from tree pack {} embedded index.",
                        locator.tree_id, locator.tree_pack_id
                    )
                })?;
            if entry.entry_count != locator.entry_count {
                return Err(format!(
                    "Tree {} locator entry count does not match embedded index.",
                    locator.tree_id
                ));
            }
            if entry.checksum != locator.tree_pack_checksum {
                return Err(format!(
                    "Tree {} locator checksum does not match embedded index.",
                    locator.tree_id
                ));
            }
            if tree_locators
                .insert(locator.tree_id.clone(), locator)
                .is_some()
            {
                return Err(format!("Duplicate tree locator {}.", locator.tree_id));
            }
        }
        Ok(tree_locators)
    }

    fn validate_snapshots(
        &self,
        tree_packs: &BTreeMap<String, &RepositoryTreePackInventoryRow>,
        tree_locators: &BTreeMap<String, &RepositoryTreeLocatorInventoryRow>,
    ) -> Result<(), String> {
        let snapshot_ids = self
            .snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id.as_str())
            .collect::<BTreeSet<_>>();
        if snapshot_ids.len() != self.snapshots.len() {
            return Err("Duplicate snapshot ids in repository pack inventory.".to_string());
        }

        for snapshot in &self.snapshots {
            require_non_empty(&snapshot.snapshot_id, "snapshot id")?;
            crate::snapshot_store::validate_snapshot_parent_set(
                Some(&snapshot.snapshot_id),
                &snapshot.parent_snapshot_ids,
                snapshot.primary_parent_snapshot_id.as_deref(),
                snapshot.parent_snapshot_id.as_deref(),
            )?;
            require_non_empty(&snapshot.root_tree_pack_id, "snapshot root tree pack id")?;
            require_non_empty(&snapshot.manifest_hash, "snapshot manifest hash")?;
            if snapshot.root_entry_ordinal < 0 {
                return Err(format!(
                    "Snapshot {} has negative root tree ordinal.",
                    snapshot.snapshot_id
                ));
            }
            if snapshot.file_count < 0 || snapshot.total_bytes < 0 {
                return Err(format!(
                    "Snapshot {} has negative file count or byte total.",
                    snapshot.snapshot_id
                ));
            }
            for parent_snapshot_id in &snapshot.parent_snapshot_ids {
                if !snapshot_ids.contains(parent_snapshot_id.as_str()) {
                    return Err(format!(
                        "Snapshot {} references unknown parent snapshot {}.",
                        snapshot.snapshot_id, parent_snapshot_id
                    ));
                }
            }

            let tree_pack = tree_packs.get(&snapshot.root_tree_pack_id).ok_or_else(|| {
                format!(
                    "Snapshot {} references unknown root tree pack {}.",
                    snapshot.snapshot_id, snapshot.root_tree_pack_id
                )
            })?;
            let root_entry = tree_pack
                .embedded_index
                .trees
                .iter()
                .find(|entry| entry.entry_ordinal == snapshot.root_entry_ordinal)
                .ok_or_else(|| {
                    format!(
                        "Snapshot {} root tree ordinal {} is not in tree pack {}.",
                        snapshot.snapshot_id,
                        snapshot.root_entry_ordinal,
                        snapshot.root_tree_pack_id
                    )
                })?;
            let root_locator = tree_locators.get(&root_entry.tree_id).ok_or_else(|| {
                format!(
                    "Snapshot {} root tree {} is missing a tree locator.",
                    snapshot.snapshot_id, root_entry.tree_id
                )
            })?;
            if root_locator.tree_pack_id != snapshot.root_tree_pack_id {
                return Err(format!(
                    "Snapshot {} root tree locator points at a different tree pack.",
                    snapshot.snapshot_id
                ));
            }
            if root_locator.entry_count != root_entry.entry_count
                || root_locator.tree_pack_checksum != root_entry.checksum
            {
                return Err(format!(
                    "Snapshot {} root tree locator does not match embedded index.",
                    snapshot.snapshot_id
                ));
            }
        }
        Ok(())
    }

    fn validate_line_heads(&self) -> Result<(), String> {
        let snapshot_ids = self
            .snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut line_names = BTreeSet::new();
        for line_head in &self.line_heads {
            require_non_empty(&line_head.line_name, "line name")?;
            if !line_names.insert(line_head.line_name.as_str()) {
                return Err(format!("Duplicate line head {}.", line_head.line_name));
            }
            if let Some(head_snapshot_id) = line_head.head_snapshot_id.as_deref() {
                if !head_snapshot_id.trim().is_empty()
                    && !snapshot_ids.contains(head_snapshot_id.trim())
                {
                    return Err(format!(
                        "Line {} references unknown head snapshot {}.",
                        line_head.line_name, head_snapshot_id
                    ));
                }
            }
        }
        Ok(())
    }
}
