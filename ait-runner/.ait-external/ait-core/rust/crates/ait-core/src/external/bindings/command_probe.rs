use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json_support::JsonValue;

use crate::external::bindings::model::ExternalBindingTool;
use crate::external::bindings::validator::{
    ExternalBindingToolProbe, ExternalBindingToolProbeRequest, ExternalBindingToolProbeResult,
};
use crate::external::{ExternalError, ExternalResult};
use crate::json_support::JsonCodec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindingCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl ExternalBindingCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalBindingCommandStatus {
    Exit(i32),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindingCommandOutput {
    pub status: ExternalBindingCommandStatus,
    pub stdout: String,
    pub stderr: String,
}

impl ExternalBindingCommandOutput {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            status: ExternalBindingCommandStatus::Exit(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn failure(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            status: ExternalBindingCommandStatus::Exit(code),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: ExternalBindingCommandStatus::NotFound,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn is_success(&self) -> bool {
        self.status == ExternalBindingCommandStatus::Exit(0)
    }
}

pub trait ExternalBindingCommandRunner {
    fn run_binding_command(
        &self,
        command: ExternalBindingCommand,
    ) -> ExternalResult<ExternalBindingCommandOutput>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessExternalBindingCommandRunner;

impl ExternalBindingCommandRunner for ProcessExternalBindingCommandRunner {
    fn run_binding_command(
        &self,
        command: ExternalBindingCommand,
    ) -> ExternalResult<ExternalBindingCommandOutput> {
        let mut process = Command::new(&command.program);
        process.args(&command.args);
        if let Some(cwd) = &command.cwd {
            process.current_dir(cwd);
        }
        match process.output() {
            Ok(output) => Ok(ExternalBindingCommandOutput {
                status: ExternalBindingCommandStatus::Exit(output.status.code().unwrap_or(1)),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(ExternalBindingCommandOutput::not_found())
            }
            Err(err) => Err(ExternalError::with_code(
                "external_binding_tool_run",
                format!(
                    "failed to run external binding tool {:?}: {err}",
                    command.program
                ),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandExternalBindingToolProbe<R = ProcessExternalBindingCommandRunner> {
    runner: R,
}

impl Default for CommandExternalBindingToolProbe<ProcessExternalBindingCommandRunner> {
    fn default() -> Self {
        Self::new(ProcessExternalBindingCommandRunner)
    }
}

impl<R> CommandExternalBindingToolProbe<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl<R> ExternalBindingToolProbe for CommandExternalBindingToolProbe<R>
where
    R: ExternalBindingCommandRunner,
{
    fn probe_binding_tool(
        &self,
        request: ExternalBindingToolProbeRequest<'_>,
    ) -> ExternalResult<ExternalBindingToolProbeResult> {
        match request.tool {
            ExternalBindingTool::Cargo => probe_cargo_metadata(
                &self.runner,
                request.binding_path,
                request.binding.package.as_deref(),
            ),
            ExternalBindingTool::Python => probe_python_metadata(
                &self.runner,
                request.binding_path,
                request
                    .binding
                    .module
                    .as_deref()
                    .or(request.binding.package.as_deref()),
            ),
            ExternalBindingTool::Node => probe_node_metadata(
                &self.runner,
                request.binding_path,
                request.binding.package.as_deref(),
            ),
            ExternalBindingTool::Go => probe_go_metadata(
                &self.runner,
                request.binding_path,
                request.binding.module.as_deref(),
            ),
        }
    }
}

fn probe_cargo_metadata<R>(
    runner: &R,
    binding_path: &Path,
    expected_package: Option<&str>,
) -> ExternalResult<ExternalBindingToolProbeResult>
where
    R: ExternalBindingCommandRunner,
{
    let manifest_path = binding_path.join("Cargo.toml");
    let output = runner.run_binding_command(ExternalBindingCommand::new("cargo").with_args([
        "metadata".to_string(),
        "--no-deps".to_string(),
        "--format-version".to_string(),
        "1".to_string(),
        "--manifest-path".to_string(),
        manifest_path.to_string_lossy().into_owned(),
    ]))?;
    command_output_to_probe_result(ExternalBindingTool::Cargo, output, |stdout| {
        validate_cargo_metadata(stdout, expected_package)
    })
}

fn probe_python_metadata<R>(
    runner: &R,
    binding_path: &Path,
    expected_module: Option<&str>,
) -> ExternalResult<ExternalBindingToolProbeResult>
where
    R: ExternalBindingCommandRunner,
{
    let script = r#"
import pathlib
import sys
import importlib

root = pathlib.Path(sys.argv[1])
module = sys.argv[2].strip() if len(sys.argv) > 2 else ""
if module:
    sys.path.insert(0, str(root))
    try:
        importlib.import_module(module)
    except Exception as err:
        print(f"failed to import Python binding module {module!r}: {err}", file=sys.stderr)
        raise SystemExit(1)
    raise SystemExit(0)
metadata = ["pyproject.toml", "setup.cfg", "setup.py"]
if any((root / name).is_file() for name in metadata):
    raise SystemExit(0)
if (root / "__init__.py").is_file():
    raise SystemExit(0)
if any(root.glob("*/__init__.py")):
    raise SystemExit(0)
print("no Python package metadata or package marker found", file=sys.stderr)
raise SystemExit(1)
"#;
    let output = run_python_command(runner, script, binding_path, expected_module)?;
    command_output_to_probe_result(ExternalBindingTool::Python, output, |_| Ok(()))
}

fn probe_node_metadata<R>(
    runner: &R,
    binding_path: &Path,
    expected_package: Option<&str>,
) -> ExternalResult<ExternalBindingToolProbeResult>
where
    R: ExternalBindingCommandRunner,
{
    let script = r#"
const fs = require("fs");
const path = require("path");
const root = process.argv[1];
const expectedPackage = (process.argv[2] || "").trim();
const packagePath = path.join(root, "package.json");
if (!fs.existsSync(packagePath)) {
  console.error("package.json is missing");
  process.exit(1);
}
let parsed;
try {
  parsed = JSON.parse(fs.readFileSync(packagePath, "utf8"));
} catch (err) {
  console.error(`package.json is invalid: ${err.message}`);
  process.exit(1);
}
if (!parsed.name || typeof parsed.name !== "string") {
  console.error("package.json must include a package name");
  process.exit(1);
}
if (expectedPackage && parsed.name !== expectedPackage) {
  console.error(`package.json name ${JSON.stringify(parsed.name)} does not match ${JSON.stringify(expectedPackage)}`);
  process.exit(1);
}
"#;
    let output = runner.run_binding_command(ExternalBindingCommand::new("node").with_args([
        "-e".to_string(),
        script.to_string(),
        binding_path.to_string_lossy().into_owned(),
        expected_package.unwrap_or("").to_string(),
    ]))?;
    command_output_to_probe_result(ExternalBindingTool::Node, output, |_| Ok(()))
}

fn probe_go_metadata<R>(
    runner: &R,
    binding_path: &Path,
    expected_module: Option<&str>,
) -> ExternalResult<ExternalBindingToolProbeResult>
where
    R: ExternalBindingCommandRunner,
{
    let output = runner.run_binding_command(
        ExternalBindingCommand::new("go")
            .with_args(["list", "-m", "-json"])
            .with_cwd(binding_path.to_path_buf()),
    )?;
    command_output_to_probe_result(ExternalBindingTool::Go, output, |stdout| {
        validate_go_metadata(stdout, expected_module)
    })
}

fn run_python_command<R>(
    runner: &R,
    script: &str,
    binding_path: &Path,
    expected_module: Option<&str>,
) -> ExternalResult<ExternalBindingCommandOutput>
where
    R: ExternalBindingCommandRunner,
{
    let python3 =
        runner.run_binding_command(ExternalBindingCommand::new("python3").with_args([
            "-c".to_string(),
            script.to_string(),
            binding_path.to_string_lossy().into_owned(),
            expected_module.unwrap_or("").to_string(),
        ]))?;
    if python3.status != ExternalBindingCommandStatus::NotFound {
        return Ok(python3);
    }
    runner.run_binding_command(ExternalBindingCommand::new("python").with_args([
        "-c".to_string(),
        script.to_string(),
        binding_path.to_string_lossy().into_owned(),
        expected_module.unwrap_or("").to_string(),
    ]))
}

fn command_output_to_probe_result(
    tool: ExternalBindingTool,
    output: ExternalBindingCommandOutput,
    validate_stdout: impl FnOnce(&str) -> Result<(), String>,
) -> ExternalResult<ExternalBindingToolProbeResult> {
    match output.status {
        ExternalBindingCommandStatus::NotFound => {
            Ok(ExternalBindingToolProbeResult::skipped(format!(
                "{} metadata validation tool is not available",
                tool.as_str()
            )))
        }
        ExternalBindingCommandStatus::Exit(_) if output.is_success() => {
            match validate_stdout(&output.stdout) {
                Ok(()) => Ok(ExternalBindingToolProbeResult::passed()),
                Err(message) => Ok(ExternalBindingToolProbeResult::failed(format!(
                    "{} metadata validation failed: {message}",
                    tool.as_str()
                ))),
            }
        }
        ExternalBindingCommandStatus::Exit(code) => {
            Ok(ExternalBindingToolProbeResult::failed(format!(
                "{} metadata validation exited with status {code}: {}",
                tool.as_str(),
                command_failure_message(&output)
            )))
        }
    }
}

fn validate_cargo_metadata(stdout: &str, expected_package: Option<&str>) -> Result<(), String> {
    let value = parse_stdout_json(stdout, "cargo metadata package list")?;
    let packages = value
        .get("packages")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "cargo metadata package list is missing \"packages\"".to_string())?;
    if packages.is_empty() {
        return Err("cargo metadata package list is empty".to_string());
    }
    if let Some(expected_package) = expected_package.filter(|value| !value.trim().is_empty()) {
        let found = packages
            .iter()
            .filter_map(|package| package.get("name").and_then(JsonValue::as_str))
            .any(|name| name == expected_package);
        if !found {
            return Err(format!(
                "cargo metadata package list does not contain package {expected_package:?}"
            ));
        }
    }
    Ok(())
}

fn validate_go_metadata(stdout: &str, expected_module: Option<&str>) -> Result<(), String> {
    let path = json_string_field(stdout, "Path", "go module path")?;
    if let Some(expected_module) = expected_module.filter(|value| !value.trim().is_empty()) {
        if path != expected_module {
            return Err(format!(
                "go module path {path:?} does not match {expected_module:?}"
            ));
        }
    }
    Ok(())
}

fn json_string_field(stdout: &str, field: &str, label: &str) -> Result<String, String> {
    let value = parse_stdout_json(stdout, label)?;
    match value.get(field).and_then(JsonValue::as_str) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        Some(_) => Err(format!("{label} is empty")),
        None => Err(format!("{label} is missing {field:?}")),
    }
}

fn parse_stdout_json(stdout: &str, label: &str) -> Result<JsonValue, String> {
    JsonCodec::parse_value_with_error_prefix(stdout, &format!("{label} did not produce valid JSON"))
        .map_err(String::from)
}

fn command_failure_message(output: &ExternalBindingCommandOutput) -> String {
    let stderr = tail_text(&output.stderr);
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = tail_text(&output.stdout);
    if !stdout.is_empty() {
        return stdout;
    }
    "no diagnostic output".to_string()
}

fn tail_text(value: &str) -> String {
    let value = value.trim();
    let char_count = value.chars().count();
    if char_count <= 400 {
        value.to_string()
    } else {
        value.chars().skip(char_count.saturating_sub(400)).collect()
    }
}
