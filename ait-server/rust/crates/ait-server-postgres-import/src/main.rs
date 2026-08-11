use ait_server_postgres_import::{
    activate_generation, audit_generation, stage_from_postgres, upgrade_u64_seconds,
    ActivateRequest, AuditGenerationRequest, StageRequest, UpgradeU64SecondsRequest,
};
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("ait-server-postgres-import: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    run_with_arguments(env::args().skip(1))
}

fn run_with_arguments(arguments: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().ok_or_else(usage)?;
    let mut options = std::collections::BTreeMap::new();
    while let Some(flag) = arguments.next() {
        if !flag.starts_with("--") {
            return Err(format!("unexpected argument {flag:?}\n{}", usage()));
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {flag}\n{}", usage()))?;
        if options.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate option {flag}\n{}", usage()));
        }
    }

    match command.as_str() {
        "audit-generation" => {
            require_exact_options(&options, &["--generation", "--report"])?;
            let report = audit_generation(AuditGenerationRequest {
                generation: PathBuf::from(required(&options, "--generation")?),
                report_path: PathBuf::from(required(&options, "--report")?),
            })?;
            println!(
                "audited immutable Binary generation: repositories={}, tasks={}, jobs={}, fingerprint={}, report={}",
                report.repository_count,
                report.task_count,
                report.worker_job_count,
                report.source_fingerprint,
                report.report_path.display()
            );
        }
        "stage" => {
            require_exact_options(
                &options,
                &[
                    "--dsn",
                    "--source-manifest",
                    "--source-root",
                    "--repository-order",
                    "--legacy-alias-manifest",
                    "--legacy-alias-root",
                    "--legacy-id-index-manifest",
                    "--legacy-id-index-root",
                    "--recovery-job-manifest",
                    "--staged-generation",
                    "--report",
                ],
            )?;
            let report = stage_from_postgres(StageRequest {
                dsn: required(&options, "--dsn")?.to_string(),
                source_manifest: PathBuf::from(required(&options, "--source-manifest")?),
                source_root: options.get("--source-root").map(PathBuf::from),
                repository_order: options.get("--repository-order").map(PathBuf::from),
                legacy_alias_manifest: options.get("--legacy-alias-manifest").map(PathBuf::from),
                legacy_alias_root: options.get("--legacy-alias-root").map(PathBuf::from),
                legacy_id_index_manifest: options
                    .get("--legacy-id-index-manifest")
                    .map(PathBuf::from),
                legacy_id_index_root: options.get("--legacy-id-index-root").map(PathBuf::from),
                recovery_job_manifest: options.get("--recovery-job-manifest").map(PathBuf::from),
                staged_generation: PathBuf::from(required(&options, "--staged-generation")?),
                report_path: PathBuf::from(required(&options, "--report")?),
            })?;
            println!(
                "staged PostgreSQL-free generation: repositories={}, jobs={}, report={}",
                report.repository_count,
                report.worker_job_count,
                report.report_path.display()
            );
        }
        "upgrade-u64-seconds" => {
            require_exact_options(
                &options,
                &[
                    "--source-selector",
                    "--source-generation",
                    "--staged-generation",
                    "--report",
                ],
            )?;
            let result = upgrade_u64_seconds(UpgradeU64SecondsRequest {
                source_selector: required(&options, "--source-selector")?.to_string(),
                source_generation: PathBuf::from(required(&options, "--source-generation")?),
                staged_generation: PathBuf::from(required(&options, "--staged-generation")?),
                report_path: PathBuf::from(required(&options, "--report")?),
            })?;
            println!(
                "staged u64-second Binary v0 generation: repositories={}, tasks={}, jobs={}, bytes={}->{}, source_fingerprint={}, target_fingerprint={}, generation={}, report={}",
                result.repository_count,
                result.task_count,
                result.worker_job_count,
                result.source_bytes,
                result.target_bytes,
                result.source_fingerprint,
                result.target_fingerprint,
                result.staged_generation.display(),
                result.report_path.display()
            );
        }
        "activate" => {
            require_exact_options(&options, &["--staged-generation", "--activation-pointer"])?;
            let result = activate_generation(ActivateRequest {
                staged_generation: PathBuf::from(required(&options, "--staged-generation")?),
                activation_pointer: PathBuf::from(required(&options, "--activation-pointer")?),
            })?;
            println!(
                "activated PostgreSQL-free generation: pointer={}, generation={}",
                result.activation_pointer.display(),
                result.staged_generation.display()
            );
        }
        _ => return Err(format!("unknown command {command:?}\n{}", usage())),
    }
    Ok(())
}

fn required<'a>(
    options: &'a std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, String> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required option {name}\n{}", usage()))
}

fn require_exact_options(
    options: &std::collections::BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), String> {
    let unknown = options
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "unsupported option(s): {}\n{}",
            unknown.join(", "),
            usage()
        ))
    }
}

fn usage() -> String {
    [
        "usage:",
        "  ait-server-postgres-import audit-generation --generation <generation-root> --report <new-report.json>",
        "  ait-server-postgres-import stage --dsn <postgres-dsn> --source-manifest <manifest.json> [--source-root <authority-root>] [--repository-order <order.json>] [--legacy-alias-manifest <manifest.json> [--legacy-alias-root <explicit-generation-root>] [--legacy-id-index-manifest <manifest.json> [--legacy-id-index-root <schema1-generation-root>]]] [--recovery-job-manifest <frozen-manifest.json>] --staged-generation <empty-path> --report <report.json>",
        "  ait-server-postgres-import upgrade-u64-seconds --source-selector u32-time-v0 --source-generation <active-u32-generation> --staged-generation <new-inactive-sibling> --report <new-report.json>",
        "  ait-server-postgres-import activate --staged-generation <validated-path> --activation-pointer <pointer-file>",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_surface_contains_only_reusable_migration_operations() {
        let help = usage();
        for command in [
            "audit-generation",
            "stage",
            "upgrade-u64-seconds",
            "activate",
        ] {
            assert!(
                help.contains(command),
                "missing supported command {command}"
            );
        }
        for command in [
            "replay-fresh",
            "inspect-plan-lineage-repair",
            "repair-plan-lineage",
        ] {
            assert!(!help.contains(command), "retired command remains in usage");
            let error = run_with_arguments([command.to_string()])
                .expect_err("retired command must not have a hidden dispatch path");
            assert!(error.starts_with("unknown command"), "{error}");
        }
    }
}
