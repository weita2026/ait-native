use crate::snapshot_store::{validate_snapshot_parent_set, SnapshotParentLink, SnapshotStore};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const DEFAULT_SNAPSHOT_DAG_MAX_DEPTH: usize = 1_000_000;
pub const DEFAULT_SNAPSHOT_DAG_MAX_RESULTS: usize = 1_000_000;
pub const DEFAULT_SNAPSHOT_DAG_BATCH_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotParentMode {
    AllParents,
    FirstParent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotDagLimitMode {
    Error,
    Truncate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotDagLimits {
    pub max_depth: usize,
    pub max_results: usize,
    pub batch_size: usize,
    pub limit_mode: SnapshotDagLimitMode,
}

impl Default for SnapshotDagLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_SNAPSHOT_DAG_MAX_DEPTH,
            max_results: DEFAULT_SNAPSHOT_DAG_MAX_RESULTS,
            batch_size: DEFAULT_SNAPSHOT_DAG_BATCH_SIZE,
            limit_mode: SnapshotDagLimitMode::Error,
        }
    }
}

impl SnapshotDagLimits {
    pub fn validate(self) -> Result<Self, String> {
        if self.max_results == 0 {
            return Err("Snapshot DAG max_results must be greater than zero.".to_string());
        }
        if self.batch_size == 0 {
            return Err("Snapshot DAG batch_size must be greater than zero.".to_string());
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDagTraversal {
    pub topological_snapshot_ids: Vec<String>,
    pub parent_snapshot_ids: BTreeMap<String, Vec<String>>,
    pub depth_by_snapshot_id: BTreeMap<String, usize>,
    pub truncated: bool,
}

impl SnapshotDagTraversal {
    pub fn contains(&self, snapshot_id: &str) -> bool {
        self.parent_snapshot_ids.contains_key(snapshot_id)
    }

    pub fn distance_from_head(&self, snapshot_id: &str) -> Option<usize> {
        self.depth_by_snapshot_id.get(snapshot_id).copied()
    }
}

#[derive(Clone, Debug, Default)]
pub struct SnapshotAncestorDistanceCache {
    parent_links: BTreeMap<String, Option<SnapshotParentLink>>,
}

impl SnapshotAncestorDistanceCache {
    fn parent_links<S>(
        &mut self,
        store: &S,
        snapshot_ids: &[String],
    ) -> Result<Vec<Option<SnapshotParentLink>>, String>
    where
        S: SnapshotStore + ?Sized,
    {
        let missing_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| !self.parent_links.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_snapshot_ids.is_empty() {
            let links = store.snapshot_parent_links(&missing_snapshot_ids)?;
            if links.len() != missing_snapshot_ids.len() {
                return Err(format!(
                    "Snapshot DAG parent batch returned {} rows for {} requested snapshots.",
                    links.len(),
                    missing_snapshot_ids.len()
                ));
            }
            for (snapshot_id, link) in missing_snapshot_ids.into_iter().zip(links) {
                self.parent_links.insert(snapshot_id, link);
            }
        }
        snapshot_ids
            .iter()
            .map(|snapshot_id| {
                self.parent_links.get(snapshot_id).cloned().ok_or_else(|| {
                    format!("Snapshot DAG distance cache lost requested snapshot {snapshot_id}.")
                })
            })
            .collect()
    }
}

pub fn snapshot_ancestor_closure<S>(
    store: &S,
    head_snapshot_ids: &[String],
    stop_at_snapshot_ids: &BTreeSet<String>,
    parent_mode: SnapshotParentMode,
    limits: SnapshotDagLimits,
) -> Result<SnapshotDagTraversal, String>
where
    S: SnapshotStore + ?Sized,
{
    let limits = limits.validate()?;
    let heads = canonical_snapshot_ids(head_snapshot_ids, "head")?;
    if heads.is_empty() {
        return Ok(SnapshotDagTraversal {
            topological_snapshot_ids: Vec::new(),
            parent_snapshot_ids: BTreeMap::new(),
            depth_by_snapshot_id: BTreeMap::new(),
            truncated: false,
        });
    }

    let mut pending = VecDeque::new();
    let mut discovered = BTreeSet::new();
    let mut depth_by_snapshot_id = BTreeMap::new();
    let mut requested_by = BTreeMap::<String, Option<String>>::new();
    let mut discovery_rank = BTreeMap::new();
    let mut next_rank = 0usize;
    let mut truncated = false;
    for head in heads {
        if stop_at_snapshot_ids.contains(&head) || discovered.contains(&head) {
            continue;
        }
        if discovered.len() >= limits.max_results
            && truncate_or_error(
                limits,
                &mut truncated,
                format!(
                    "Snapshot DAG traversal exceeded max_results {} while queuing head {head}.",
                    limits.max_results
                ),
            )?
        {
            continue;
        }
        discovered.insert(head.clone());
        depth_by_snapshot_id.insert(head.clone(), 0);
        requested_by.insert(head.clone(), None);
        discovery_rank.insert(head.clone(), next_rank);
        next_rank += 1;
        pending.push_back(head);
    }

    let mut parent_snapshot_ids = BTreeMap::new();
    while !pending.is_empty() {
        let mut batch = Vec::with_capacity(limits.batch_size.min(pending.len()));
        while batch.len() < limits.batch_size {
            let Some(snapshot_id) = pending.pop_front() else {
                break;
            };
            batch.push(snapshot_id);
        }
        let links = store.snapshot_parent_links(&batch)?;
        if links.len() != batch.len() {
            return Err(format!(
                "Snapshot DAG parent batch returned {} rows for {} requested snapshots.",
                links.len(),
                batch.len()
            ));
        }
        for (snapshot_id, link) in batch.into_iter().zip(links) {
            let link = require_parent_link(&snapshot_id, link, requested_by.get(&snapshot_id))?;
            let mut parents = selected_parents(&link, parent_mode)?;
            parents.retain(|parent| !stop_at_snapshot_ids.contains(parent));
            let depth = depth_by_snapshot_id
                .get(&snapshot_id)
                .copied()
                .ok_or_else(|| format!("Snapshot DAG lost depth for {snapshot_id}."))?;
            if !parents.is_empty()
                && depth >= limits.max_depth
                && truncate_or_error(
                    limits,
                    &mut truncated,
                    format!(
                        "Snapshot DAG traversal exceeded max_depth {} at {snapshot_id}.",
                        limits.max_depth
                    ),
                )?
            {
                parents.clear();
            }
            let mut included_parents = Vec::with_capacity(parents.len());
            for parent in parents {
                if !discovered.contains(&parent) {
                    if discovered.len() >= limits.max_results
                        && truncate_or_error(
                            limits,
                            &mut truncated,
                            format!(
                                "Snapshot DAG traversal exceeded max_results {} while reading parent {parent} of {snapshot_id}.",
                                limits.max_results
                            ),
                        )?
                    {
                        continue;
                    }
                    discovered.insert(parent.clone());
                    depth_by_snapshot_id.insert(parent.clone(), depth + 1);
                    requested_by.insert(parent.clone(), Some(snapshot_id.clone()));
                    discovery_rank.insert(parent.clone(), next_rank);
                    next_rank += 1;
                    pending.push_back(parent.clone());
                }
                included_parents.push(parent);
            }
            parent_snapshot_ids.insert(snapshot_id, included_parents);
        }
    }

    let topological_snapshot_ids = topological_order_with_rank(
        &parent_snapshot_ids,
        &BTreeSet::new(),
        Some(&discovery_rank),
    )?;
    Ok(SnapshotDagTraversal {
        topological_snapshot_ids,
        parent_snapshot_ids,
        depth_by_snapshot_id,
        truncated,
    })
}

pub fn snapshot_ancestor_closure_from_parent_map(
    parent_snapshot_ids: &BTreeMap<String, Vec<String>>,
    head_snapshot_ids: &[String],
    stop_at_snapshot_ids: &BTreeSet<String>,
    parent_mode: SnapshotParentMode,
    limits: SnapshotDagLimits,
) -> Result<SnapshotDagTraversal, String> {
    let limits = limits.validate()?;
    let heads = canonical_snapshot_ids(head_snapshot_ids, "head")?;
    let mut pending = VecDeque::new();
    let mut discovered = BTreeSet::new();
    let mut selected = BTreeMap::new();
    let mut depth_by_snapshot_id = BTreeMap::new();
    let mut discovery_rank = BTreeMap::new();
    let mut next_rank = 0usize;
    let mut truncated = false;
    for head in heads {
        if stop_at_snapshot_ids.contains(&head) || discovered.contains(&head) {
            continue;
        }
        if discovered.len() >= limits.max_results
            && truncate_or_error(
                limits,
                &mut truncated,
                format!(
                    "Snapshot DAG traversal exceeded max_results {} while queuing head {head}.",
                    limits.max_results
                ),
            )?
        {
            continue;
        }
        discovered.insert(head.clone());
        pending.push_back(head.clone());
        depth_by_snapshot_id.insert(head.clone(), 0);
        discovery_rank.insert(head, next_rank);
        next_rank += 1;
    }

    while let Some(snapshot_id) = pending.pop_front() {
        let parents = parent_snapshot_ids
            .get(&snapshot_id)
            .ok_or_else(|| format!("Snapshot DAG is missing snapshot {snapshot_id}."))?;
        validate_snapshot_parent_set(
            Some(&snapshot_id),
            parents,
            parents.first().map(String::as_str),
            parents.first().map(String::as_str),
        )?;
        let mut parents = match parent_mode {
            SnapshotParentMode::AllParents => parents.clone(),
            SnapshotParentMode::FirstParent => parents.first().cloned().into_iter().collect(),
        };
        parents.retain(|parent| !stop_at_snapshot_ids.contains(parent));
        let depth = depth_by_snapshot_id[&snapshot_id];
        if !parents.is_empty()
            && depth >= limits.max_depth
            && truncate_or_error(
                limits,
                &mut truncated,
                format!(
                    "Snapshot DAG traversal exceeded max_depth {} at {snapshot_id}.",
                    limits.max_depth
                ),
            )?
        {
            parents.clear();
        }
        let mut included_parents = Vec::with_capacity(parents.len());
        for parent in parents {
            if !discovered.contains(&parent) {
                if discovered.len() >= limits.max_results
                    && truncate_or_error(
                        limits,
                        &mut truncated,
                        format!(
                            "Snapshot DAG traversal exceeded max_results {} while reading parent {parent} of {snapshot_id}.",
                            limits.max_results
                        ),
                    )?
                {
                    continue;
                }
                if !parent_snapshot_ids.contains_key(&parent) {
                    return Err(format!(
                        "Snapshot DAG parent {parent} referenced by {snapshot_id} is missing."
                    ));
                }
                discovered.insert(parent.clone());
                depth_by_snapshot_id.insert(parent.clone(), depth + 1);
                discovery_rank.insert(parent.clone(), next_rank);
                next_rank += 1;
                pending.push_back(parent.clone());
            }
            included_parents.push(parent);
        }
        selected.insert(snapshot_id, included_parents);
    }

    let topological_snapshot_ids =
        topological_order_with_rank(&selected, &BTreeSet::new(), Some(&discovery_rank))?;
    Ok(SnapshotDagTraversal {
        topological_snapshot_ids,
        parent_snapshot_ids: selected,
        depth_by_snapshot_id,
        truncated,
    })
}

pub fn snapshot_descendant_closure<S>(
    store: &S,
    root_snapshot_ids: &[String],
    parent_mode: SnapshotParentMode,
    limits: SnapshotDagLimits,
) -> Result<SnapshotDagTraversal, String>
where
    S: SnapshotStore + ?Sized,
{
    let limits = limits.validate()?;
    let roots = canonical_snapshot_ids(root_snapshot_ids, "root")?;
    if roots.is_empty() {
        return Ok(SnapshotDagTraversal {
            topological_snapshot_ids: Vec::new(),
            parent_snapshot_ids: BTreeMap::new(),
            depth_by_snapshot_id: BTreeMap::new(),
            truncated: false,
        });
    }

    let root_links = store.snapshot_parent_links(&roots)?;
    if root_links.len() != roots.len() {
        return Err(format!(
            "Snapshot DAG root batch returned {} rows for {} requested snapshots.",
            root_links.len(),
            roots.len()
        ));
    }

    let mut discovered = BTreeSet::new();
    let mut depth_by_snapshot_id = BTreeMap::new();
    let mut discovery_rank = BTreeMap::new();
    let mut link_by_snapshot_id = BTreeMap::new();
    let mut frontier = BTreeSet::new();
    let mut next_rank = 0usize;
    let mut truncated = false;
    for (root, link) in roots.into_iter().zip(root_links) {
        let link = require_parent_link(&root, link, None)?;
        if discovered.contains(&root) {
            continue;
        }
        if discovered.len() >= limits.max_results
            && truncate_or_error(
                limits,
                &mut truncated,
                format!(
                    "Snapshot DAG traversal exceeded max_results {} while queuing root {root}.",
                    limits.max_results
                ),
            )?
        {
            continue;
        }
        discovered.insert(root.clone());
        depth_by_snapshot_id.insert(root.clone(), 0);
        discovery_rank.insert(root.clone(), next_rank);
        next_rank += 1;
        frontier.insert(root.clone());
        link_by_snapshot_id.insert(root, link);
    }

    let mut depth = 0usize;
    while !frontier.is_empty() {
        let at_depth_limit = depth >= limits.max_depth;
        let available = limits.max_results.saturating_sub(discovered.len());
        let mut candidates = BTreeMap::<String, SnapshotParentLink>::new();
        let mut cursor = 0usize;
        loop {
            let page = store.snapshot_parent_link_page(cursor, limits.batch_size)?;
            if page.links.len() > limits.batch_size {
                return Err(format!(
                    "Snapshot parent-link page returned {} rows for limit {}.",
                    page.links.len(),
                    limits.batch_size
                ));
            }
            for link in page.links {
                let child_snapshot_id = link.snapshot_id.clone();
                let parents = selected_parents(&link, parent_mode)?;
                if !parents.iter().any(|parent| frontier.contains(parent)) {
                    continue;
                }
                if let Some(child_depth) = depth_by_snapshot_id.get(&child_snapshot_id).copied() {
                    if child_depth <= depth {
                        return Err(format!(
                            "Cycle detected in Snapshot DAG at {child_snapshot_id}."
                        ));
                    }
                    continue;
                }
                if at_depth_limit
                    && truncate_or_error(
                        limits,
                        &mut truncated,
                        format!(
                            "Snapshot DAG traversal exceeded max_depth {} at descendant {child_snapshot_id}.",
                            limits.max_depth
                        ),
                    )? {
                        continue;
                    }
                if candidates.contains_key(&child_snapshot_id) {
                    continue;
                }
                if candidates.len() < available {
                    candidates.insert(child_snapshot_id, link);
                    continue;
                }
                if limits.limit_mode == SnapshotDagLimitMode::Error {
                    return Err(format!(
                        "Snapshot DAG traversal exceeded max_results {} while reading descendant {child_snapshot_id}.",
                        limits.max_results
                    ));
                }
                truncated = true;
                if let Some(last_snapshot_id) = candidates.keys().next_back().cloned() {
                    if child_snapshot_id < last_snapshot_id {
                        candidates.remove(&last_snapshot_id);
                        candidates.insert(child_snapshot_id, link);
                    }
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if next_cursor <= cursor {
                return Err(format!(
                    "Snapshot parent-link page cursor did not advance from {cursor} to {next_cursor}."
                ));
            }
            cursor = next_cursor;
        }

        if candidates.is_empty() {
            break;
        }
        frontier.clear();
        depth = depth.saturating_add(1);
        for (snapshot_id, link) in candidates {
            discovered.insert(snapshot_id.clone());
            depth_by_snapshot_id.insert(snapshot_id.clone(), depth);
            discovery_rank.insert(snapshot_id.clone(), next_rank);
            next_rank += 1;
            frontier.insert(snapshot_id.clone());
            link_by_snapshot_id.insert(snapshot_id, link);
        }
    }

    let mut parent_snapshot_ids = BTreeMap::new();
    for (snapshot_id, link) in link_by_snapshot_id {
        let depth = depth_by_snapshot_id
            .get(&snapshot_id)
            .copied()
            .ok_or_else(|| format!("Snapshot DAG lost depth for {snapshot_id}."))?;
        let mut parents = if depth == 0 {
            Vec::new()
        } else {
            selected_parents(&link, parent_mode)?
        };
        parents.retain(|parent| discovered.contains(parent));
        parent_snapshot_ids.insert(snapshot_id, parents);
    }
    let topological_snapshot_ids = topological_order_with_rank(
        &parent_snapshot_ids,
        &BTreeSet::new(),
        Some(&discovery_rank),
    )?;
    Ok(SnapshotDagTraversal {
        topological_snapshot_ids,
        parent_snapshot_ids,
        depth_by_snapshot_id,
        truncated,
    })
}

pub fn topological_snapshot_order(
    parent_snapshot_ids: &BTreeMap<String, Vec<String>>,
    known_external_snapshot_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    topological_order_with_rank(parent_snapshot_ids, known_external_snapshot_ids, None)
}

pub fn snapshot_is_ancestor<S>(
    store: &S,
    ancestor_snapshot_id: &str,
    descendant_snapshot_id: &str,
    limits: SnapshotDagLimits,
) -> Result<Option<usize>, String>
where
    S: SnapshotStore + ?Sized,
{
    snapshot_ancestor_distance(store, ancestor_snapshot_id, descendant_snapshot_id, limits)
}

pub fn snapshot_ancestor_distance<S>(
    store: &S,
    ancestor_snapshot_id: &str,
    descendant_snapshot_id: &str,
    limits: SnapshotDagLimits,
) -> Result<Option<usize>, String>
where
    S: SnapshotStore + ?Sized,
{
    snapshot_ancestor_distance_with_cache(
        store,
        ancestor_snapshot_id,
        descendant_snapshot_id,
        limits,
        &mut SnapshotAncestorDistanceCache::default(),
    )
}

pub fn snapshot_ancestor_distance_with_cache<S>(
    store: &S,
    ancestor_snapshot_id: &str,
    descendant_snapshot_id: &str,
    limits: SnapshotDagLimits,
    cache: &mut SnapshotAncestorDistanceCache,
) -> Result<Option<usize>, String>
where
    S: SnapshotStore + ?Sized,
{
    let limits = limits.validate()?;
    let descendants = canonical_snapshot_ids(&[descendant_snapshot_id.to_string()], "descendant")?;
    let descendant_snapshot_id = descendants
        .into_iter()
        .next()
        .expect("one canonical descendant");
    let ancestor_snapshot_id = ancestor_snapshot_id.to_string();

    let mut pending = VecDeque::from([descendant_snapshot_id.clone()]);
    let mut discovered = BTreeSet::from([descendant_snapshot_id.clone()]);
    let mut depth_by_snapshot_id = BTreeMap::from([(descendant_snapshot_id.clone(), 0usize)]);
    let mut requested_by = BTreeMap::from([(descendant_snapshot_id, None::<String>)]);
    let mut truncated = false;

    while !pending.is_empty() {
        let mut batch = Vec::with_capacity(limits.batch_size.min(pending.len()));
        while batch.len() < limits.batch_size {
            let Some(snapshot_id) = pending.pop_front() else {
                break;
            };
            batch.push(snapshot_id);
        }
        let links = cache.parent_links(store, &batch)?;
        for (snapshot_id, link) in batch.into_iter().zip(links) {
            let link = require_parent_link(&snapshot_id, link, requested_by.get(&snapshot_id))?;
            let mut parents = selected_parents(&link, SnapshotParentMode::AllParents)?;
            let depth = depth_by_snapshot_id
                .get(&snapshot_id)
                .copied()
                .ok_or_else(|| format!("Snapshot DAG lost depth for {snapshot_id}."))?;
            if snapshot_id == ancestor_snapshot_id {
                return Ok(Some(depth));
            }
            if !parents.is_empty()
                && depth >= limits.max_depth
                && truncate_or_error(
                    limits,
                    &mut truncated,
                    format!(
                        "Snapshot DAG traversal exceeded max_depth {} at {snapshot_id}.",
                        limits.max_depth
                    ),
                )?
            {
                parents.clear();
            }
            for parent in parents {
                if discovered.contains(&parent) {
                    continue;
                }
                if discovered.len() >= limits.max_results
                    && truncate_or_error(
                        limits,
                        &mut truncated,
                        format!(
                            "Snapshot DAG traversal exceeded max_results {} while reading parent {parent} of {snapshot_id}.",
                            limits.max_results
                        ),
                    )?
                {
                    continue;
                }
                discovered.insert(parent.clone());
                depth_by_snapshot_id.insert(parent.clone(), depth + 1);
                requested_by.insert(parent.clone(), Some(snapshot_id.clone()));
                pending.push_back(parent);
            }
        }
    }
    Ok(None)
}

pub fn snapshot_merge_bases<S>(
    store: &S,
    left_snapshot_id: &str,
    right_snapshot_id: &str,
    limits: SnapshotDagLimits,
) -> Result<Vec<String>, String>
where
    S: SnapshotStore + ?Sized,
{
    let left = snapshot_ancestor_closure(
        store,
        &[left_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        limits,
    )?;
    let right = snapshot_ancestor_closure(
        store,
        &[right_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        limits,
    )?;
    let common = left
        .parent_snapshot_ids
        .keys()
        .filter(|snapshot_id| right.parent_snapshot_ids.contains_key(*snapshot_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    if common.is_empty() {
        return Ok(Vec::new());
    }

    // Every ancestor of a common ancestor is itself common. A common node is
    // therefore a lowest/best merge base exactly when no other common node
    // names it as a parent. This retains every base in criss-cross histories.
    let mut has_common_child = BTreeSet::new();
    for snapshot_id in &common {
        let parents = left
            .parent_snapshot_ids
            .get(snapshot_id)
            .or_else(|| right.parent_snapshot_ids.get(snapshot_id))
            .ok_or_else(|| format!("Snapshot DAG lost common node {snapshot_id}."))?;
        for parent in parents {
            if common.contains(parent) {
                has_common_child.insert(parent.clone());
            }
        }
    }
    let mut bases = common
        .into_iter()
        .filter(|snapshot_id| !has_common_child.contains(snapshot_id))
        .collect::<Vec<_>>();
    bases.sort_by(|left_id, right_id| {
        let left_key = merge_base_sort_key(&left, &right, left_id);
        let right_key = merge_base_sort_key(&left, &right, right_id);
        left_key.cmp(&right_key)
    });
    Ok(bases)
}

pub fn snapshot_first_parent_chain<S>(
    store: &S,
    target_snapshot_id: &str,
    selected_target_parent_snapshot_id: Option<&str>,
    limits: SnapshotDagLimits,
) -> Result<Vec<String>, String>
where
    S: SnapshotStore + ?Sized,
{
    let Some(target_link) = store.snapshot_parent_link(target_snapshot_id)? else {
        return Err(format!("Unknown snapshot: {target_snapshot_id}"));
    };
    let Some(selected_parent) = selected_target_parent_snapshot_id else {
        return Ok(snapshot_ancestor_closure(
            store,
            &[target_snapshot_id.to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::FirstParent,
            limits,
        )?
        .topological_snapshot_ids);
    };
    if !target_link
        .parent_snapshot_ids
        .iter()
        .any(|parent| parent == selected_parent)
    {
        return Err(format!(
            "Snapshot {target_snapshot_id} does not have selected parent {selected_parent}; available parents: {}.",
            target_link.parent_snapshot_ids.join(", ")
        ));
    }
    let mut chain = snapshot_ancestor_closure(
        store,
        &[selected_parent.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::FirstParent,
        limits,
    )?
    .topological_snapshot_ids;
    chain.push(target_snapshot_id.to_string());
    Ok(chain)
}

fn truncate_or_error(
    limits: SnapshotDagLimits,
    truncated: &mut bool,
    message: String,
) -> Result<bool, String> {
    match limits.limit_mode {
        SnapshotDagLimitMode::Error => Err(message),
        SnapshotDagLimitMode::Truncate => {
            *truncated = true;
            Ok(true)
        }
    }
}

fn canonical_snapshot_ids(values: &[String], role: &str) -> Result<Vec<String>, String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (ordinal, value) in values.iter().enumerate() {
        if value.trim().is_empty() || value.trim() != value {
            return Err(format!(
                "Snapshot DAG {role} ordinal {ordinal} must be non-empty canonical text."
            ));
        }
        if seen.insert(value.clone()) {
            out.push(value.clone());
        }
    }
    Ok(out)
}

fn require_parent_link(
    snapshot_id: &str,
    link: Option<SnapshotParentLink>,
    requested_by: Option<&Option<String>>,
) -> Result<SnapshotParentLink, String> {
    let Some(link) = link else {
        return match requested_by.and_then(Option::as_deref) {
            Some(child) => Err(format!(
                "Snapshot DAG parent {snapshot_id} referenced by {child} is missing."
            )),
            None => Err(format!("Unknown snapshot: {snapshot_id}")),
        };
    };
    if link.snapshot_id != snapshot_id {
        return Err(format!(
            "Snapshot DAG parent batch returned {} for requested {snapshot_id}.",
            link.snapshot_id
        ));
    }
    validate_snapshot_parent_set(
        Some(snapshot_id),
        &link.parent_snapshot_ids,
        link.primary_parent_snapshot_id.as_deref(),
        link.parent_snapshot_id.as_deref(),
    )?;
    Ok(link)
}

fn selected_parents(
    link: &SnapshotParentLink,
    parent_mode: SnapshotParentMode,
) -> Result<Vec<String>, String> {
    validate_snapshot_parent_set(
        Some(&link.snapshot_id),
        &link.parent_snapshot_ids,
        link.primary_parent_snapshot_id.as_deref(),
        link.parent_snapshot_id.as_deref(),
    )?;
    Ok(match parent_mode {
        SnapshotParentMode::AllParents => link.parent_snapshot_ids.clone(),
        SnapshotParentMode::FirstParent => link
            .primary_parent_snapshot_id
            .clone()
            .into_iter()
            .collect(),
    })
}

fn topological_order_with_rank(
    parent_snapshot_ids: &BTreeMap<String, Vec<String>>,
    known_external_snapshot_ids: &BTreeSet<String>,
    discovery_rank: Option<&BTreeMap<String, usize>>,
) -> Result<Vec<String>, String> {
    let mut indegree = BTreeMap::<String, usize>::new();
    let mut children = BTreeMap::<String, Vec<String>>::new();
    for (snapshot_id, parents) in parent_snapshot_ids {
        validate_snapshot_parent_set(
            Some(snapshot_id),
            parents,
            parents.first().map(String::as_str),
            parents.first().map(String::as_str),
        )?;
        indegree.entry(snapshot_id.clone()).or_insert(0);
        for parent in parents {
            if known_external_snapshot_ids.contains(parent) {
                continue;
            }
            if !parent_snapshot_ids.contains_key(parent) {
                return Err(format!(
                    "Snapshot DAG parent {parent} referenced by {snapshot_id} is missing."
                ));
            }
            *indegree.entry(snapshot_id.clone()).or_insert(0) += 1;
            children
                .entry(parent.clone())
                .or_default()
                .push(snapshot_id.clone());
        }
    }
    for values in children.values_mut() {
        values.sort_by(|left, right| {
            stable_node_key(left, discovery_rank).cmp(&stable_node_key(right, discovery_rank))
        });
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(snapshot_id, _)| stable_node_key(snapshot_id, discovery_rank))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(parent_snapshot_ids.len());
    while let Some((_, snapshot_id)) = ready.pop_first() {
        ordered.push(snapshot_id.clone());
        for child in children.get(&snapshot_id).into_iter().flatten() {
            let count = indegree
                .get_mut(child)
                .ok_or_else(|| format!("Snapshot DAG lost indegree for {child}."))?;
            *count = count.saturating_sub(1);
            if *count == 0 {
                ready.insert(stable_node_key(child, discovery_rank));
            }
        }
    }
    if ordered.len() != parent_snapshot_ids.len() {
        let blocked = indegree
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(snapshot_id, _)| stable_node_key(snapshot_id, discovery_rank))
            .min()
            .map(|(_, snapshot_id)| snapshot_id)
            .unwrap_or_else(|| "unknown".to_string());
        return Err(format!("Cycle detected in Snapshot DAG at {blocked}."));
    }
    Ok(ordered)
}

fn stable_node_key(
    snapshot_id: &str,
    discovery_rank: Option<&BTreeMap<String, usize>>,
) -> (usize, String) {
    (
        discovery_rank
            .and_then(|ranks| ranks.get(snapshot_id).copied())
            .unwrap_or(usize::MAX),
        snapshot_id.to_string(),
    )
}

fn merge_base_sort_key(
    left: &SnapshotDagTraversal,
    right: &SnapshotDagTraversal,
    snapshot_id: &str,
) -> (usize, usize, String) {
    let left_distance = left.distance_from_head(snapshot_id).unwrap_or(usize::MAX);
    let right_distance = right.distance_from_head(snapshot_id).unwrap_or(usize::MAX);
    (
        left_distance.max(right_distance),
        left_distance.saturating_add(right_distance),
        snapshot_id.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot_store::{SnapshotParentLinkPage, SnapshotRecord, SnapshotStoreResult};

    #[derive(Default)]
    struct MemorySnapshotStore {
        parents: BTreeMap<String, Vec<String>>,
    }

    impl MemorySnapshotStore {
        fn diamond() -> Self {
            Self {
                parents: BTreeMap::from([
                    ("SNP-ROOT".to_string(), vec![]),
                    ("SNP-LEFT".to_string(), vec!["SNP-ROOT".to_string()]),
                    ("SNP-RIGHT".to_string(), vec!["SNP-ROOT".to_string()]),
                    (
                        "SNP-MERGE".to_string(),
                        vec!["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
                    ),
                ]),
            }
        }
    }

    impl SnapshotStore for MemorySnapshotStore {
        fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
            Ok(self.parents.contains_key(snapshot_id))
        }

        fn snapshot_parent_link(
            &self,
            snapshot_id: &str,
        ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
            Ok(self
                .parents
                .get(snapshot_id)
                .map(|parents| SnapshotParentLink {
                    snapshot_id: snapshot_id.to_string(),
                    parent_snapshot_ids: parents.clone(),
                    primary_parent_snapshot_id: parents.first().cloned(),
                    parent_snapshot_id: parents.first().cloned(),
                }))
        }

        fn snapshot_by_id(
            &self,
            _snapshot_id: &str,
        ) -> SnapshotStoreResult<Option<SnapshotRecord>> {
            Ok(None)
        }

        fn snapshot_parent_link_page(
            &self,
            cursor: usize,
            limit: usize,
        ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
            if limit == 0 {
                return Err("limit must be positive".to_string());
            }
            if cursor > self.parents.len() {
                return Err("cursor is out of range".to_string());
            }
            let links = self
                .parents
                .iter()
                .skip(cursor)
                .take(limit)
                .map(|(snapshot_id, parents)| SnapshotParentLink {
                    snapshot_id: snapshot_id.clone(),
                    parent_snapshot_ids: parents.clone(),
                    primary_parent_snapshot_id: parents.first().cloned(),
                    parent_snapshot_id: parents.first().cloned(),
                })
                .collect::<Vec<_>>();
            let next_cursor = cursor.saturating_add(limit).min(self.parents.len());
            Ok(SnapshotParentLinkPage {
                links,
                next_cursor: (next_cursor < self.parents.len()).then_some(next_cursor),
            })
        }

        fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
            Ok(Vec::new())
        }

        fn snapshot_total_bytes(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
            Ok(None)
        }

        fn snapshot_root_tree_pack_id(
            &self,
            _snapshot_id: &str,
        ) -> SnapshotStoreResult<Option<String>> {
            Ok(None)
        }

        fn snapshot_kind(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
            Ok(None)
        }

        fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
            snapshot_first_parent_chain(self, snapshot_id, None, SnapshotDagLimits::default())
        }

        fn set_snapshot_kind(
            &self,
            _snapshot_id: &str,
            _snapshot_kind: &str,
        ) -> SnapshotStoreResult<usize> {
            Ok(0)
        }
    }

    #[test]
    fn diamond_traversal_is_all_parent_topological_and_reports_shortest_depth() {
        let store = MemorySnapshotStore::diamond();
        let traversal = snapshot_ancestor_closure(
            &store,
            &["SNP-MERGE".to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )
        .expect("diamond traversal");
        assert_eq!(
            traversal.topological_snapshot_ids,
            vec!["SNP-ROOT", "SNP-LEFT", "SNP-RIGHT", "SNP-MERGE"]
        );
        assert_eq!(traversal.distance_from_head("SNP-MERGE"), Some(0));
        assert_eq!(traversal.distance_from_head("SNP-ROOT"), Some(2));
        assert_eq!(
            snapshot_is_ancestor(
                &store,
                "SNP-RIGHT",
                "SNP-MERGE",
                SnapshotDagLimits::default()
            )
            .expect("ancestor query"),
            Some(1)
        );
    }

    #[test]
    fn first_parent_is_explicit_and_target_parent_can_be_selected() {
        let store = MemorySnapshotStore::diamond();
        assert_eq!(
            snapshot_first_parent_chain(&store, "SNP-MERGE", None, SnapshotDagLimits::default())
                .unwrap(),
            vec!["SNP-ROOT", "SNP-LEFT", "SNP-MERGE"]
        );
        assert_eq!(
            snapshot_first_parent_chain(
                &store,
                "SNP-MERGE",
                Some("SNP-RIGHT"),
                SnapshotDagLimits::default()
            )
            .unwrap(),
            vec!["SNP-ROOT", "SNP-RIGHT", "SNP-MERGE"]
        );
    }

    #[test]
    fn descendant_traversal_is_paged_topological_and_first_parent_is_explicit() {
        let store = MemorySnapshotStore::diamond();
        let all = snapshot_descendant_closure(
            &store,
            &["SNP-ROOT".to_string()],
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                batch_size: 2,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            all.topological_snapshot_ids,
            vec!["SNP-ROOT", "SNP-LEFT", "SNP-RIGHT", "SNP-MERGE"]
        );
        assert_eq!(all.distance_from_head("SNP-MERGE"), Some(2));
        assert!(!all.truncated);

        let alternate_parent = snapshot_descendant_closure(
            &store,
            &["SNP-RIGHT".to_string()],
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )
        .unwrap();
        assert_eq!(
            alternate_parent.topological_snapshot_ids,
            vec!["SNP-RIGHT", "SNP-MERGE"]
        );
        let first_parent = snapshot_descendant_closure(
            &store,
            &["SNP-RIGHT".to_string()],
            SnapshotParentMode::FirstParent,
            SnapshotDagLimits::default(),
        )
        .unwrap();
        assert_eq!(first_parent.topological_snapshot_ids, vec!["SNP-RIGHT"]);
    }

    #[test]
    fn bounded_queries_truncate_stably_across_wide_and_deep_histories() {
        let mut wide = MemorySnapshotStore::default();
        wide.parents.insert("SNP-ROOT".to_string(), vec![]);
        for ordinal in (0..100).rev() {
            wide.parents.insert(
                format!("SNP-CHILD-{ordinal:03}"),
                vec!["SNP-ROOT".to_string()],
            );
        }
        let wide_result = snapshot_descendant_closure(
            &wide,
            &["SNP-ROOT".to_string()],
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                max_results: 4,
                batch_size: 7,
                limit_mode: SnapshotDagLimitMode::Truncate,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            wide_result.topological_snapshot_ids,
            vec![
                "SNP-ROOT",
                "SNP-CHILD-000",
                "SNP-CHILD-001",
                "SNP-CHILD-002"
            ]
        );
        assert!(wide_result.truncated);

        let mut deep = MemorySnapshotStore::default();
        deep.parents.insert("SNP-000".to_string(), vec![]);
        for ordinal in 1..=256 {
            deep.parents.insert(
                format!("SNP-{ordinal:03}"),
                vec![format!("SNP-{:03}", ordinal - 1)],
            );
        }
        let deep_result = snapshot_ancestor_closure(
            &deep,
            &["SNP-256".to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                max_depth: 16,
                max_results: 17,
                limit_mode: SnapshotDagLimitMode::Truncate,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap();
        assert_eq!(deep_result.parent_snapshot_ids.len(), 17);
        assert!(deep_result.truncated);
    }

    #[test]
    fn descendant_traversal_reports_cycles_and_hard_bounds() {
        let cycle = MemorySnapshotStore {
            parents: BTreeMap::from([
                ("SNP-A".to_string(), vec!["SNP-B".to_string()]),
                ("SNP-B".to_string(), vec!["SNP-A".to_string()]),
            ]),
        };
        let error = snapshot_descendant_closure(
            &cycle,
            &["SNP-A".to_string()],
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )
        .unwrap_err();
        assert!(error.contains("Cycle detected"));
        assert!(error.contains("SNP-A"));

        let error = snapshot_descendant_closure(
            &MemorySnapshotStore::diamond(),
            &["SNP-ROOT".to_string()],
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                max_depth: 1,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("max_depth 1"));
        assert!(error.contains("SNP-MERGE"));
    }

    #[test]
    fn malformed_missing_cycle_and_bounds_fail_with_named_snapshot() {
        let missing = BTreeMap::from([("SNP-CHILD".to_string(), vec!["SNP-MISSING".to_string()])]);
        let error = snapshot_ancestor_closure_from_parent_map(
            &missing,
            &["SNP-CHILD".to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )
        .unwrap_err();
        assert!(error.contains("SNP-MISSING"));
        assert!(error.contains("SNP-CHILD"));

        let cycle = BTreeMap::from([
            ("SNP-A".to_string(), vec!["SNP-B".to_string()]),
            ("SNP-B".to_string(), vec!["SNP-A".to_string()]),
        ]);
        let error = topological_snapshot_order(&cycle, &BTreeSet::new()).unwrap_err();
        assert!(error.contains("Cycle detected"));

        let error = snapshot_ancestor_closure(
            &MemorySnapshotStore::diamond(),
            &["SNP-MERGE".to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                max_results: 2,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("max_results 2"));

        let error = snapshot_ancestor_closure_from_parent_map(
            &MemorySnapshotStore::diamond().parents,
            &["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits {
                max_results: 1,
                ..SnapshotDagLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("max_results 1"));
        assert!(error.contains("SNP-RIGHT"));
    }

    #[test]
    fn merge_bases_return_every_best_criss_cross_base_deterministically() {
        let store = MemorySnapshotStore {
            parents: BTreeMap::from([
                ("SNP-ROOT".to_string(), vec![]),
                ("SNP-A1".to_string(), vec!["SNP-ROOT".to_string()]),
                ("SNP-B1".to_string(), vec!["SNP-ROOT".to_string()]),
                (
                    "SNP-A2".to_string(),
                    vec!["SNP-A1".to_string(), "SNP-B1".to_string()],
                ),
                (
                    "SNP-B2".to_string(),
                    vec!["SNP-B1".to_string(), "SNP-A1".to_string()],
                ),
                ("SNP-LEFT".to_string(), vec!["SNP-A2".to_string()]),
                ("SNP-RIGHT".to_string(), vec!["SNP-B2".to_string()]),
            ]),
        };

        assert_eq!(
            snapshot_merge_bases(
                &store,
                "SNP-LEFT",
                "SNP-RIGHT",
                SnapshotDagLimits::default()
            )
            .unwrap(),
            vec!["SNP-A1", "SNP-B1"]
        );
    }
}
