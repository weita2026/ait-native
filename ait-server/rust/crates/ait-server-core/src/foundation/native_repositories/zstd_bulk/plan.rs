use super::*;

pub(in crate::foundation::native_repositories) fn zstd_bulk_plan_json(
    client: &mut postgres::Client,
    repo_name: &str,
    request: JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::ZstdBulkPlan,
    )?;
    let contract = RemoteSyncPlanJson::stateless();
    let plan_request = contract.zstd_bulk_plan_request(&request)?;

    let present_snapshot_set = plan_request
        .snapshot_ids
        .iter()
        .filter_map(|snapshot_id| {
            select_snapshot_row(client, repo_name, snapshot_id)
                .ok()
                .flatten()
                .map(|_| snapshot_id.clone())
        })
        .collect::<BTreeSet<_>>();
    let present_object_pack_set = plan_request
        .object_pack_ids
        .iter()
        .filter_map(|pack_id| {
            client
                .query_opt(
                    "select pack_id from packs where pack_id = $1 and pack_format = $2",
                    &[&pack_id, &PACK_FORMAT_ZSTD_CHUNKED_V1],
                )
                .ok()
                .flatten()
                .map(|_| pack_id.clone())
        })
        .collect::<BTreeSet<_>>();
    let present_tree_pack_set = plan_request
        .tree_pack_ids
        .iter()
        .filter_map(|pack_id| {
            client
                .query_opt(
                    "select pack_id from tree_packs where pack_id = $1",
                    &[&pack_id],
                )
                .ok()
                .flatten()
                .map(|_| pack_id.clone())
        })
        .collect::<BTreeSet<_>>();

    Ok(contract.zstd_bulk_plan_response(
        repo_name,
        &plan_request,
        &RemoteSyncZstdBulkPlanPresence {
            present_snapshot_ids: present_snapshot_set,
            present_object_pack_ids: present_object_pack_set,
            present_tree_pack_ids: present_tree_pack_set,
        },
    ))
}
