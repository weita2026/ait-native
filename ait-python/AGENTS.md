# AGENTS

<!-- ait:workflow:start -->
## Effective Ait Workflow (Generated)

`ait init`, relevant `ait config set`/`unset` changes, and default-remote setup
regenerate this authoritative block from `.ait/config.json` and sync its
configured target when available.

### Effective workflow admission

Satisfied:

- entry: mode=`solo_local`; sprint=`on`; scopes=`local`
- entry: plan-binding=`required`
- entry: default-remote=`origin` (inactive); transport=`local-only`; server-use=`none`
- authoring: author-mode=`ai_with_human_review`; model=`unset` (optional)
- closeout: contract=`task-land-plan-closeout/v1`; readiness=`local_admission`; plan-closeout=`automatic_exact_local_when_final_task_completed`

Action required:

- closeout: review=`automatic`; reviewer=`unset` (configure user-name)

`task start` revalidates entry; Snapshot creation and `task finish` revalidate
authoring and closeout. Inspect configuration only for an action-required item,
an explicit configuration task, or a validator-reported mismatch.

### Rules for every repository mutation

- Read this block and `docs/plan.md` when it exists.
- When a regression is found, run `ait blame <path>` (narrow with `--line` or
  `--start`/`--end`) to identify the responsible Snapshot or Plan revision
  before choosing the repair.
- Reconcile authored Markdown through `ait plan sync <markdown-file-or-dir> --local`. The initial
  sprint card is the command-spelling exception: `task start --from` performs
  that exact-file Plan sync before code work. Do not hide Markdown lineage
  inside a code snapshot.
- `ait workflow ready` and `ait workflow finish` are text-only decision surfaces;
  never append or recommend `--json` for either command.
- Every code change must start with a new `ait task start`, be authored in its
  bound worktree, and finish through `ait task finish <task-or-change-id>`.
  There is no direct Snapshot-only closeout path.
- Prefer `ait queue summary` for current actionable inventory, `ait task list
  --all` and `ait change list --all` for history, and `ait task audit <task-id>`
  for one task's readiness.

### Task path: sprint mode is on

For changes classified as `normal_task` or `fully_governed`:

1. Write a detailed Markdown sprint card under `docs/sprints/` with one stable
   `[plan-ref: ...]` root and an unchecked checklist item carrying an exact
   `[ref: ...]`.
2. Start the task and first change with `ait task start --from
   <sprint-card-path>#<exact-ref> --intent "<intent>"`.
   `task start --from` owns exact-file Plan sync in the configured scope,
   post-sync item taskability validation, canonical Plan binding, Task/Change
   creation, bound-worktree bootstrap, and the printed `cd` hint. The task is
   local-only; do not run a separate pre-start Plan sync or copy Plan IDs.
3. Enter the task worktree emitted by `task start` and author the code there.
   `ait snapshot create --message "<message>"` remains available for optional
   intermediate checkpoints; final dirty work can be Snapshotted by Task finish.
4. Finish dirty work with `ait task finish <task-or-change-id> --message
   "<message>"`. If an explicit Snapshot already made the worktree clean, omit
   `--message`; finish reuses the current Line head. A successful final local
   Task finish closes and syncs the exact bound sprint checklist item locally.

After every context-window compaction, re-read the bound sprint card before
continuing.

### Local finish

- `task start`, its initial change, Snapshots, and `task finish` stay local unless
  a command explicitly requests remote promotion.
- `ait task finish <task-or-change-id> --message "<message>" --local` creates
  the final Snapshot for dirty work, applies it to the local target Line,
  completes the Task, cleans the bound worktree, and (when bound) closes the
  local sprint checklist item. Clean work omits `--message` and reuses the
  current Line-head Snapshot.
<!-- ait:workflow:end -->

