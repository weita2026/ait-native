# AGENTS

<!-- ait:workflow:start -->
## Effective Ait Workflow (Generated)

### Effective route

Route: mode=`solo_local`; sprint=`off`; scope=`local`; plan-binding=`off`; author-mode=`ai_with_human_review`.

Action required before mutation:

- review=`automatic`; reviewer=`unset` (configure user-name)

Plan prose defaults: language=`zh-Hant-TW` (BCP 47); style=`concise`.
Write new Plan and sprint prose in the specified language and style.
Concise means short sentences without repetition; detailed includes useful
background and rationale. Always retain goals, scope, necessary decisions,
acceptance criteria, and verification. Keep commands, paths, identifiers,
and binding markers unchanged. Preserve an existing document's language
unless translation is requested. Explicit user instructions take precedence.

### Code-change path

1. Run `ait task start --title "<title>" --intent "<intent>"`; sprint
   mode is off, so `--from` is unavailable.
2. Work only in the returned `edit_root`. Intermediate `ait snapshot create
   <task-id> --message "<message>"` checkpoints are optional.
3. For dirty work, run `ait task finish
   <task-id> --message "<message>" --local`; when already
   clean, omit `--message`. Successful Task finish output is authoritative
   proof of local apply, Task completion, worktree cleanup, and
   applicable bound-card closeout. Do not follow it with `status`, `diff`, or `audit`
   unless it fails, reports required action, state is unexpected, or evidence
   was requested.

If the caller already chose a safe absolute worktree path, add `--edit-root
<absolute-path>` to Task start; otherwise omit it and use the returned `edit_root`.

### Conditional references

- Read `docs/plan.md` when it exists.
- For a regression, use `ait blame <path>` before choosing a repair.
- Sync authored Markdown with `ait plan sync <markdown-file-or-dir> --local`; do not hide
  Markdown lineage in a code Snapshot.
- A Snapshot is a checkpoint, not a substitute for the listed closeout.
- Only when that question arises: `ait queue summary` shows actionable work,
  `ait task audit <task-id>` shows readiness, and `ait task list --all` plus
  `ait snapshot list --all` show history.
- Change IDs are internal to normal Task work. Do not create another Change
  for checkpoints, review corrections, or checklist steps. Patchset CI uses
  the public `TASK_ID/P-##` Patchset reference. If a Task reports ambiguous work, inspect `ait task
  audit <task-id>`.
<!-- ait:workflow:end -->

