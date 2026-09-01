# CLAUDE

<!-- ait:workflow:start -->
## Effective Ait Workflow (Generated)

### Effective route

Route: mode=`solo_local`; sprint=`on`; scope=`local`; plan-binding=`required`; author-mode=`ai_with_human_review`.

Action required before mutation:

- review=`automatic`; reviewer=`unset` (configure user-name)

### Code-change path

1. Create a detailed card under `docs/sprints/` with one stable
   `[plan-ref: ...]` and one unchecked item carrying an exact `[ref: ...]`.
2. Select a safe absolute, absent-or-empty Task worktree path outside the
   canonical repository, then run `ait task start --from <sprint-card-path>#<exact-ref> --intent
   "<intent>" --edit-root <absolute-path> && cd <absolute-path>`. `--from` syncs and binds the initial card; do not
   pre-sync it or copy Plan IDs.
3. Work only in the returned `edit_root`. Intermediate `ait snapshot create
   --message "<message>"` checkpoints are optional.
4. For dirty work, run `ait task finish
   <task-or-change-id> --message "<message>" --local`; when already
   clean, omit `--message`. Successful Task finish output is authoritative
   proof of local apply, Task completion, worktree cleanup, and
   applicable bound-card closeout. Do not follow it with `status`, `diff`, or `audit`
   unless it fails, reports required action, state is unexpected, or evidence
   was requested.

After every context-window compaction, re-read the bound sprint card before
continuing.

The two `<absolute-path>` values must be identical. Do not omit `--edit-root`;
retain the returned Task ID and verify that its `edit_root` is the selected path.

### Conditional references

- Read `docs/plan.md` when it exists.
- For a regression, use `ait blame <path>` before choosing a repair.
- Sync authored Markdown other than the initial sprint card with
  `ait plan sync <markdown-file-or-dir> --local`; do not hide Markdown lineage in a code Snapshot.
- A Snapshot is a checkpoint, not a substitute for the listed closeout.
- Only when that question arises: `ait queue summary` shows actionable work,
  `ait task audit <task-id>` shows readiness, and `ait task list --all` plus
  `ait change list --all` show history.
<!-- ait:workflow:end -->

