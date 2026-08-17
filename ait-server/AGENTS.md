# AGENTS

<!-- ait:workflow:start -->
## Effective Ait Workflow (Generated)

`ait init`, relevant `ait config set` / `ait config unset` changes, and default-remote setup
regenerate this block from `.ait/config.json` and sync it when the
configured target is available. The current values and commands are
authoritative; they replace stale context and generic examples.

- workflow mode: `solo_local`
- sprint mode: `on`
- default mutation scope: `local`
- task-land contract: `task-land-plan-closeout/v1`
- task-land readiness policy: `local_admission`
- task-land Plan closeout policy: `automatic_exact_local_when_final_task_completed`

Commands below already reflect these values. Do not mix local and remote
variants.

### Rules for every repository mutation

- Read this block at the start of a session. Read `docs/plan.md` when it exists,
  and use `ait config show` if runtime state may have changed.
- When a regression is found, run `ait blame <path>` (narrow with `--line` or
  `--start`/`--end`) to identify the responsible Snapshot or Plan revision
  before choosing the repair.
- Reconcile authored Markdown through `ait plan sync <markdown-file-or-dir> --local`. The initial
  sprint card is the command-spelling exception: `task start --from` performs
  that exact-file Plan sync before code work. Do not hide Markdown lineage
  inside a code snapshot.
- `ait workflow ready` and `ait workflow land` are text-only decision surfaces;
  never append or recommend `--json` for either command.
- Every code change must start with a new `ait task start`, be authored in its
  bound worktree, and finish through `ait task land <task-or-change-id>`.
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
3. Enter the task worktree emitted by `task start`, author the code there, and
   create a snapshot with `ait snapshot create --message "<message>"`.
4. Finish with `ait task land <task-or-change-id>`. A successful final
   local task land closes and syncs the exact bound sprint checklist item
   locally.

After every context-window compaction, re-read the bound sprint card before
continuing.

### Local land

- `task start`, its initial change, snapshots, and `task land` stay local unless
  a command explicitly requests remote promotion.
- `ait task land <task-or-change-id> --local` lands the code onto the local
  target line, completes the task, cleans the bound worktree, and (when bound)
  closes the local sprint checklist item.
<!-- ait:workflow:end -->

