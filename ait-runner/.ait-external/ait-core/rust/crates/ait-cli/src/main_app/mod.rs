include!("imports.rs");
include!("cli_args.rs");
include!("dispatch.rs");
include!("external.rs");
include!("core_commands.rs");
include!("binary_db.rs");
include!("git.rs");
include!("repo_commands.rs");
include!("workflow.rs");
include!("worktree.rs");
include!("task.rs");
include!("change.rs");
include!("snapshot.rs");
include!("stash.rs");
include!("tag.rs");
include!("patchset.rs");
include!("review.rs");
include!("attest_policy.rs");
include!("plan.rs");
include!("render_helpers.rs");

#[cfg(test)]
mod tests;

pub fn entry() -> std::process::ExitCode {
    std::process::ExitCode::from(entry_with_args(std::env::args_os().collect()))
}

pub fn entry_with_args(args: Vec<std::ffi::OsString>) -> u8 {
    exit_code_value(dispatch_with_args(args))
}
