use ait_cli::init_surface::{init_repo, InitRequest};
use assert_cmd::Command;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::{Compression, GzBuilder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use tar::{Archive, Builder as TarBuilder, Header as TarHeader};
use tempfile::TempDir;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

fn public_source(repositories: &[&str]) -> Value {
    let subtrees = repositories
        .iter()
        .map(|repository| {
            let transforms = match *repository {
                "ait-runner" => vec!["runner-core-path/v1"],
                "ait-python" => vec!["python-core-path/v1"],
                _ => Vec::new(),
            };
            json!({
                "source_repository": repository,
                "path": repository,
                "transforms": transforms,
            })
        })
        .collect::<Vec<_>>();
    let mut transforms = Vec::new();
    if repositories.contains(&"ait-runner") {
        transforms.push(json!({
            "id": "runner-core-path/v1",
            "source_repository": "ait-runner",
            "path": "Cargo.toml",
            "from": ".ait-external/ait-core/rust/crates/ait-core",
            "to": "../ait-core/rust/crates/ait-core",
        }));
    }
    if repositories.contains(&"ait-python") {
        transforms.push(json!({
            "id": "python-core-path/v1",
            "source_repository": "ait-python",
            "path": "pyproject.toml",
            "from": ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
            "to": "../ait-core/rust/crates/ait-py/Cargo.toml",
        }));
    }
    json!({
        "model": "release-monorepo",
        "identity": "weita2026/ait-native",
        "product_document": "docs/distribution.md",
        "family_manifest": "ait-release-family.json",
        "mapping_manifest": "ait-monorepo-source.json",
        "build_entrypoints": {
            "unix": "build-release.sh",
            "windows": "build-release.ps1",
            "implementation": "build-release.mjs",
        },
        "subtrees": subtrees,
        "transforms": transforms,
    })
}

fn github_distribution(components: &[&str], targets: &[&str]) -> Value {
    json!({
        "channel": "github",
        "role": "product",
        "identity": "weita2026/ait-native",
        "components": components,
        "targets": targets,
    })
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("ait-cli")
        .expect("ait-cli binary")
        .current_dir(root)
        .args(args)
        .output()
        .expect("ait-cli command executes")
}

fn run_json(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("ait-cli JSON output")
}

fn initialize_internal_repo(root: &Path, name: &str, default_line: &str) {
    init_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some(name.to_string()),
        default_line: default_line.to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for row in fs::read_dir(source).unwrap() {
        let row = row.unwrap();
        let source_path = row.path();
        let destination_path = destination.join(row.file_name());
        if row.file_type().unwrap().is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).unwrap();
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn tar_gz_members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
    archive
        .entries()
        .expect("read tar entries")
        .map(|entry| {
            let mut entry = entry.expect("valid tar entry");
            let path = entry
                .path()
                .expect("valid tar path")
                .to_string_lossy()
                .into_owned();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).expect("read tar member");
            (path, content)
        })
        .collect()
}

fn tar_gz_directory_entries(bytes: &[u8]) -> BTreeMap<String, bool> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
    archive
        .entries()
        .expect("read tar entries")
        .map(|entry| {
            let entry = entry.expect("valid tar entry");
            let path = entry
                .path()
                .expect("valid tar path")
                .to_string_lossy()
                .into_owned();
            (path, entry.header().entry_type().is_dir())
        })
        .collect()
}

fn assert_regular_file_parents_are_directories(bytes: &[u8]) {
    let entries = tar_gz_directory_entries(bytes);
    for (path, is_directory) in &entries {
        if *is_directory {
            continue;
        }
        let mut parent = Path::new(path).parent().map(Path::to_path_buf);
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            let directory = directory.to_string_lossy();
            assert_eq!(
                entries.get(directory.as_ref()),
                Some(&true),
                "missing directory entry {directory:?} required by {path:?}"
            );
            parent = Path::new(directory.as_ref())
                .parent()
                .map(Path::to_path_buf);
        }
    }

    let mut archive = Archive::new(GzDecoder::new(bytes));
    let mut archive_mtime = None;
    for entry in archive.entries().expect("read tar entries") {
        let entry = entry.expect("valid tar entry");
        let header = entry.header();
        let mtime = header.mtime().expect("valid tar mtime");
        assert_eq!(*archive_mtime.get_or_insert(mtime), mtime);
        if header.entry_type().is_dir() {
            assert_eq!(header.mode().expect("valid directory mode"), 0o755);
            assert_eq!(header.uid().expect("valid directory uid"), 0);
            assert_eq!(header.gid().expect("valid directory gid"), 0);
            assert_eq!(header.size().expect("valid directory size"), 0);
        }
    }
}

fn zip_members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid ZIP archive");
    let mut members = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("valid ZIP member");
        let mut content = Vec::new();
        entry.read_to_end(&mut content).expect("read ZIP member");
        assert!(members.insert(entry.name().to_string(), content).is_none());
    }
    members
}

fn ar_members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    assert!(bytes.starts_with(b"!<arch>\n"));
    let mut offset = 8;
    let mut members = BTreeMap::new();
    while offset < bytes.len() {
        assert!(offset + 60 <= bytes.len());
        let header = &bytes[offset..offset + 60];
        assert_eq!(&header[58..60], b"`\n");
        let name = std::str::from_utf8(&header[..16])
            .expect("ASCII ar member name")
            .trim()
            .trim_end_matches('/')
            .to_string();
        let size = std::str::from_utf8(&header[48..58])
            .expect("ASCII ar member size")
            .trim()
            .parse::<usize>()
            .expect("decimal ar member size");
        offset += 60;
        assert!(offset + size <= bytes.len());
        assert!(members
            .insert(name, bytes[offset..offset + size].to_vec())
            .is_none());
        offset += size;
        if !size.is_multiple_of(2) {
            assert_eq!(bytes[offset], b'\n');
            offset += 1;
        }
    }
    members
}

fn fixture_zip(entries: &BTreeMap<String, (Vec<u8>, u32)>) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap();
    for (path, (bytes, mode)) in entries {
        archive
            .start_file(
                path,
                FileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .last_modified_time(timestamp)
                    .unix_permissions(*mode),
            )
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

fn fixture_tar_gz(entries: &BTreeMap<String, (Vec<u8>, u32)>) -> Vec<u8> {
    let encoder = GzBuilder::new()
        .mtime(1)
        .operating_system(255)
        .write(Vec::new(), Compression::default());
    let mut archive = TarBuilder::new(encoder);
    for (path, (bytes, mode)) in entries {
        let mut header = TarHeader::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(1);
        header.set_cksum();
        archive
            .append_data(&mut header, path, bytes.as_slice())
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

fn fixture_wheel(target: &str) -> (String, Vec<u8>, String, Vec<u8>) {
    let tags = match target {
        "aarch64-apple-darwin" => "cp311-abi3-macosx_11_0_arm64",
        "x86_64-apple-darwin" => "cp311-abi3-macosx_11_0_x86_64",
        "aarch64-unknown-linux-gnu" => "cp311-abi3-manylinux_2_28_aarch64",
        "x86_64-unknown-linux-gnu" => "cp311-abi3-manylinux_2_28_x86_64",
        "aarch64-pc-windows-msvc" => "cp311-abi3-win_arm64",
        "x86_64-pc-windows-msvc" => "cp311-abi3-win_amd64",
        _ => panic!("unexpected target {target}"),
    };
    let filename = format!("ait_python-1.0.0rc2-{tags}.whl");
    let extension_path = format!(
        "ait_py/ait_py.abi3{}",
        if target.ends_with("windows-msvc") {
            ".pyd"
        } else {
            ".so"
        }
    );
    let extension_bytes = format!("binding:{target}\n").into_bytes();
    let dist_info = "ait_python-1.0.0rc2.dist-info";
    let record_path = format!("{dist_info}/RECORD");
    let mut entries = BTreeMap::from([
        (
            "ait_py/__init__.py".to_string(),
            (b"from ait_python import *\n".to_vec(), 0o644),
        ),
        (
            extension_path.clone(),
            (extension_bytes.clone(), 0o755),
        ),
        (
            "ait_python/__init__.py".to_string(),
            (b"__all__ = []\n".to_vec(), 0o644),
        ),
        (
            format!("{dist_info}/METADATA"),
            (
                b"Metadata-Version: 2.4\nName: ait-python\nVersion: 1.0.0rc2\nSummary: Native Python binding\nLicense-Expression: Apache-2.0\nLicense-File: LICENSE\nLicense-File: NOTICE\nRequires-Python: >=3.11\n\nNative binding fixture.\n"
                    .to_vec(),
                0o644,
            ),
        ),
        (
            format!("{dist_info}/WHEEL"),
            (
                format!(
                    "Wheel-Version: 1.0\nGenerator: fixture\nRoot-Is-Purelib: false\nTag: {tags}\n"
                )
                .into_bytes(),
                0o644,
            ),
        ),
        (
            format!("{dist_info}/licenses/LICENSE"),
            (b"ait-python:license\n".to_vec(), 0o644),
        ),
        (
            format!("{dist_info}/licenses/NOTICE"),
            (b"ait-python:notice\n".to_vec(), 0o644),
        ),
    ]);
    let mut record = entries
        .iter()
        .map(|(path, (bytes, _))| {
            format!(
                "{path},sha256={},{}",
                URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)),
                bytes.len()
            )
        })
        .collect::<Vec<_>>();
    record.push(format!("{record_path},,"));
    entries.insert(
        record_path,
        ((record.join("\n") + "\n").into_bytes(), 0o644),
    );
    (
        filename,
        fixture_zip(&entries),
        extension_path,
        extension_bytes,
    )
}

fn fixture_npm_envelope() -> Vec<u8> {
    let mut optional_dependencies = serde_json::Map::new();
    let mut payloads = Vec::new();
    for target in TARGETS {
        let (os, cpu, libc) = match *target {
            "aarch64-apple-darwin" => ("darwin", "arm64", None),
            "x86_64-apple-darwin" => ("darwin", "x64", None),
            "aarch64-unknown-linux-gnu" => ("linux", "arm64", Some("glibc")),
            "x86_64-unknown-linux-gnu" => ("linux", "x64", Some("glibc")),
            "aarch64-pc-windows-msvc" => ("win32", "arm64", None),
            "x86_64-pc-windows-msvc" => ("win32", "x64", None),
            _ => unreachable!(),
        };
        let package = format!("@wa120/ait-native-{os}-{cpu}");
        optional_dependencies.insert(package.clone(), json!("1.0.0-rc.2"));
        payloads.push(json!({
            "target": target,
            "os": os,
            "cpu": cpu,
            "libc": libc,
            "component": "ait-node",
            "package": package,
            "version": "1.0.0-rc.2",
            "binding_repository": "ait-core",
            "binding_snapshot": "SNP-111111111111",
            "license": "Apache-2.0",
            "addon": "native/ait_napi.node",
        }));
    }
    let package_json = json!({
        "name": "@wa120/ait-native",
        "version": "1.0.0-rc.2",
        "description": "Agent-first, language-neutral workflow for verified repository changes",
        "homepage": "https://ait-native.dev/",
        "license": "Apache-2.0",
        "repository": {
            "type": "git",
            "url": "git+https://github.com/weita2026/ait-native.git",
            "directory": "ait-node"
        },
        "type": "module",
        "engines": {"node": ">=20"},
        "bin": {"ait": "bin/ait.mjs"},
        "exports": {
            ".": {
                "types": "./src/index.d.ts",
                "import": "./src/index.js",
                "default": "./src/index.js"
            }
        },
        "types": "./src/index.d.ts",
        "files": ["bin/ait.mjs", "lib", "src", "LICENSE", "NOTICE"],
        "optionalDependencies": optional_dependencies,
        "scripts": {
            "native:build": "node scripts/native-build.mjs build",
            "test": "node --test",
            "check": "node --check src/runtime.js"
        }
    });
    let payload_contract = json!({
        "schema": "ait.node.napi-platform-packages/v2",
        "family_version": "1.0.0-rc.2",
        "top_level_package": "@wa120/ait-native",
        "payloads": payloads
    });
    let entries = BTreeMap::from([
        (
            "package/LICENSE".to_string(),
            (b"ait-node:license\n".to_vec(), 0o644),
        ),
        (
            "package/NOTICE".to_string(),
            (b"ait-node:notice\n".to_vec(), 0o644),
        ),
        (
            "package/README.md".to_string(),
            (
                b"# ait-native\n\nAIT turns an ordinary coding request into an isolated, sprint-bound repository change with validation evidence. It is for individual developers and maintainers who use coding agents.\n\nOfficial website: <https://ait-native.dev/>\n\n## Install and initialize\n\n```sh\nnpm install --global @wa120/ait-native@1.0.0-rc.2\nait init\n```\n\n## What initialization provides\n\nRepository-local authority, a generated AGENTS.md workflow, and an inactive server boundary.\n\n## Local and reviewed closeout\n\nAuthors run `ait workflow ready <change-id> --apply`; reviewers run `ait workflow finish <change-id> --apply`.\n\n## Upgrading from 0.x\n\nThere is no `ait install` command in 1.0. Install or upgrade `ait-native` through your selected package manager, then run `ait init` only for a new 1.0 repository authority.\n"
                    .to_vec(),
                0o644,
            ),
        ),
        (
            "package/bin/ait.mjs".to_string(),
            (
                b"#!/usr/bin/env node\nimport { NativeRuntime } from '../src/index.js';\nprocess.exitCode = new NativeRuntime().runCli(process.argv.slice(2));\n".to_vec(),
                0o755,
            ),
        ),
        (
            "package/lib/npm-payload-contract.json".to_string(),
            (serde_json::to_vec_pretty(&payload_contract).unwrap(), 0o644),
        ),
        (
            "package/package.json".to_string(),
            (serde_json::to_vec_pretty(&package_json).unwrap(), 0o644),
        ),
        (
            "package/src/agent.js".to_string(),
            (b"export class AgentClient {}\n".to_vec(), 0o644),
        ),
        (
            "package/src/contract.js".to_string(),
            (b"export const LANGUAGE_BINDING_CONTRACT = 'ait.language.binding.v1';\n".to_vec(), 0o644),
        ),
        (
            "package/src/errors.js".to_string(),
            (b"export class NativeBridgeError extends Error {}\n".to_vec(), 0o644),
        ),
        (
            "package/src/index.d.ts".to_string(),
            (b"export interface NativeAddon { runCli(args: string[]): number; }\nexport declare class NativeRuntime {}\nexport declare class AgentClient {}\n".to_vec(), 0o644),
        ),
        (
            "package/src/index.js".to_string(),
            (b"export { NativeRuntime } from './runtime.js';\nexport { AgentClient } from './agent.js';\n".to_vec(), 0o644),
        ),
        (
            "package/src/runtime.js".to_string(),
            (b"const addonPath = 'native/ait_napi.node';\nexport class NativeRuntime { runCli(args = []) { const addon = require(addonPath); return addon.runCli(args); } }\n".to_vec(), 0o644),
        ),
    ]);
    fixture_tar_gz(&entries)
}

fn fixture_npm_addon(target: &str) -> (String, Vec<u8>, Vec<u8>) {
    let (os, cpu, libc) = match target {
        "aarch64-apple-darwin" => ("darwin", "arm64", None),
        "x86_64-apple-darwin" => ("darwin", "x64", None),
        "aarch64-unknown-linux-gnu" => ("linux", "arm64", Some("glibc")),
        "x86_64-unknown-linux-gnu" => ("linux", "x64", Some("glibc")),
        "aarch64-pc-windows-msvc" => ("win32", "arm64", None),
        "x86_64-pc-windows-msvc" => ("win32", "x64", None),
        _ => panic!("unexpected target {target}"),
    };
    let package = format!("@wa120/ait-native-{os}-{cpu}");
    let addon = "native/ait_napi.node";
    let addon_bytes = format!("direct-node-api-addon:{target}\n").into_bytes();
    let license_bytes = b"ait-node:license\n".to_vec();
    let notice_bytes = b"ait-node:notice\n".to_vec();
    let mut package_json = json!({
        "name": package,
        "version": "1.0.0-rc.2",
        "description": format!("Implementation-only AIT Node-API addon for {target}"),
        "license": "Apache-2.0",
        "repository": {
            "type": "git",
            "url": "git+https://github.com/weita2026/ait-native.git",
            "directory": "ait-node"
        },
        "os": [os],
        "cpu": [cpu],
        "main": addon,
        "files": ["native", "provenance.json", "LICENSE", "NOTICE"],
        "aitNativeAddon": {
            "schema": "ait.node.napi-platform-addon/v2",
            "component": "ait-node",
            "target": target,
            "libc": libc,
            "addon": addon,
            "binding_repository": "ait-core",
            "binding_snapshot": "SNP-111111111111"
        }
    });
    if let Some(libc) = libc {
        package_json["libc"] = json!([libc]);
    }
    let provenance = json!({
        "schema": "ait.node.napi-platform-addon-provenance/v2",
        "family_version": "1.0.0-rc.2",
        "package": package,
        "target": target,
        "os": os,
        "cpu": cpu,
        "libc": libc,
        "component": "ait-node",
        "package_source_repository": "ait-node",
        "binding_repository": "ait-core",
        "binding_snapshot": "SNP-111111111111",
        "license": "Apache-2.0",
        "license_file": {
            "path": "LICENSE",
            "sha256": digest(&license_bytes),
            "size_bytes": license_bytes.len()
        },
        "notice_file": {
            "path": "NOTICE",
            "sha256": digest(&notice_bytes),
            "size_bytes": notice_bytes.len()
        },
        "source_artifact": {
            "sha256": digest(&addon_bytes),
            "size_bytes": addon_bytes.len()
        },
        "installed_path": addon
    });
    let entries = BTreeMap::from([
        ("package/LICENSE".to_string(), (license_bytes, 0o644)),
        ("package/NOTICE".to_string(), (notice_bytes, 0o644)),
        (format!("package/{addon}"), (addon_bytes.clone(), 0o755)),
        (
            "package/package.json".to_string(),
            (serde_json::to_vec_pretty(&package_json).unwrap(), 0o644),
        ),
        (
            "package/provenance.json".to_string(),
            (serde_json::to_vec_pretty(&provenance).unwrap(), 0o644),
        ),
    ]);
    (
        format!("wa120-ait-native-{os}-{cpu}-1.0.0-rc.2.tgz"),
        fixture_tar_gz(&entries),
        addon_bytes,
    )
}

#[derive(Clone)]
struct ReceiptArtifactFixture {
    target: Option<String>,
    declared_path: String,
    bytes: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
fn write_custom_component_receipts(
    root: &Path,
    component: &str,
    repo_name: &str,
    snapshot_id: &str,
    ecosystem: &str,
    version: &str,
    kind: &str,
    fixtures: &[ReceiptArtifactFixture],
) {
    let adapter_manifest_sha256 = "d".repeat(64);
    let license_bytes = format!("{repo_name}:license\n").into_bytes();
    let notice_bytes = format!("{repo_name}:notice\n").into_bytes();
    let declared_artifacts = fixtures
        .iter()
        .map(|fixture| {
            json!({
                "path": fixture.declared_path,
                "kind": kind,
                "target": fixture.target,
            })
        })
        .collect::<Vec<_>>();
    for fixture in fixtures {
        let selection = fixture.target.as_deref().unwrap_or("portable");
        let receipt_dir = root.join(component).join(selection);
        fs::create_dir_all(&receipt_dir).unwrap();
        let release_id = format!("REL-GEN-{component}-{selection}");
        let artifact_path = format!(
            "dist/{release_id}/components/{component}/{}",
            fixture.declared_path
        );
        let absolute_artifact_path = receipt_dir.join(&artifact_path);
        fs::create_dir_all(absolute_artifact_path.parent().unwrap()).unwrap();
        fs::write(&absolute_artifact_path, &fixture.bytes).unwrap();
        let license_path = format!("dist/{release_id}/license-material/license/LICENSE");
        let notice_path = format!("dist/{release_id}/license-material/notice/NOTICE");
        let absolute_license_path = receipt_dir.join(&license_path);
        let absolute_notice_path = receipt_dir.join(&notice_path);
        fs::create_dir_all(absolute_license_path.parent().unwrap()).unwrap();
        fs::create_dir_all(absolute_notice_path.parent().unwrap()).unwrap();
        fs::write(&absolute_license_path, &license_bytes).unwrap();
        fs::write(&absolute_notice_path, &notice_bytes).unwrap();
        let receipt = json!({
            "contract": "ait.release.adapter.receipt/v1",
            "release_id": release_id,
            "repo_name": repo_name,
            "version": version,
            "snapshot_id": snapshot_id,
            "profile": "generic-command",
            "target": fixture.target,
            "status": "built",
            "check_summary": {"decision": "pass"},
            "metadata": {
                "release_adapter": {
                    "contract": "ait.release.adapter/v1",
                    "manifest_path": "ait-release.json",
                    "manifest_sha256": adapter_manifest_sha256,
                    "component_count": 1,
                    "declared_artifact_count": fixtures.len(),
                    "license_material_count": 2,
                    "definition": {
                        "schema": "ait.release.adapter/v1",
                        "package": {
                            "name": component,
                            "version": version,
                            "license_files": [
                                {"path": "LICENSE", "role": "license"},
                                {"path": "NOTICE", "role": "notice"}
                            ]
                        },
                        "components": [{
                            "id": component,
                            "ecosystem": ecosystem,
                            "working_directory": ".",
                            "dependency_files": ["lockfile"],
                            "commands": {
                                "test": [["fixture-test"]],
                                "build": [["fixture-build"]]
                            },
                            "artifacts": declared_artifacts
                        }]
                    }
                },
                "build": {
                    "builder": "ait_generic_command_adapter_v1",
                    "adapter_contract": "ait.release.adapter/v1",
                    "adapter_manifest_sha256": adapter_manifest_sha256,
                    "component_count": 1,
                    "declared_artifact_count": 1,
                    "license_material_count": 2,
                    "components": [{
                        "component": component,
                        "ecosystem": ecosystem,
                        "status": "pass"
                    }]
                }
            },
            "artifacts": [
                {
                    "role": "component-artifact",
                    "component": component,
                    "ecosystem": ecosystem,
                    "kind": kind,
                    "target": fixture.target,
                    "declared_path": fixture.declared_path,
                    "path": artifact_path,
                    "size_bytes": fixture.bytes.len(),
                    "sha256": digest(&fixture.bytes)
                },
                {
                    "role": "license-material",
                    "kind": "license-material",
                    "material_role": "license",
                    "declared_path": "LICENSE",
                    "source_path": "LICENSE",
                    "path": license_path,
                    "size_bytes": license_bytes.len(),
                    "sha256": digest(&license_bytes)
                },
                {
                    "role": "license-material",
                    "kind": "license-material",
                    "material_role": "notice",
                    "declared_path": "NOTICE",
                    "source_path": "NOTICE",
                    "path": notice_path,
                    "size_bytes": notice_bytes.len(),
                    "sha256": digest(&notice_bytes)
                },
                {
                    "role": "release-manifest",
                    "kind": "manifest",
                    "path": format!("dist/{release_id}/ait-release.manifest.json"),
                    "size_bytes": 1,
                    "sha256": "e".repeat(64)
                },
                {
                    "role": "release-checksum",
                    "kind": "checksum",
                    "path": format!("dist/{release_id}/ait-release.sha256"),
                    "size_bytes": 1,
                    "sha256": "f".repeat(64)
                }
            ]
        });
        fs::write(
            receipt_dir.join("ait-release.receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}

fn write_component_receipts(
    root: &Path,
    component: &str,
    repo_name: &str,
    snapshot_id: &str,
    ecosystem: &str,
    version: &str,
    kind: &str,
) {
    write_component_receipts_for_targets(
        root,
        component,
        repo_name,
        snapshot_id,
        ecosystem,
        version,
        kind,
        TARGETS,
    );
}

#[allow(clippy::too_many_arguments)]
fn write_component_receipts_for_targets(
    root: &Path,
    component: &str,
    repo_name: &str,
    snapshot_id: &str,
    ecosystem: &str,
    version: &str,
    kind: &str,
    targets: &[&str],
) {
    let adapter_manifest_sha256 = "a".repeat(64);
    let license_bytes = format!("{repo_name}:license\n").into_bytes();
    let notice_bytes = format!("{repo_name}:notice\n").into_bytes();
    let declared_artifacts = targets
        .iter()
        .map(|target| {
            json!({
                "path": format!("{target}.bin"),
                "kind": kind,
                "target": target,
            })
        })
        .collect::<Vec<_>>();
    for target in targets {
        let receipt_dir = root.join(component).join(target);
        fs::create_dir_all(&receipt_dir).unwrap();
        let artifact_bytes = format!("{component}:{version}:{target}\n").into_bytes();
        let release_id = format!("REL-GEN-{component}-{target}");
        let declared_path = format!("{target}.bin");
        let artifact_path = format!("dist/{release_id}/components/{component}/{declared_path}");
        let absolute_artifact_path = receipt_dir.join(&artifact_path);
        fs::create_dir_all(absolute_artifact_path.parent().unwrap()).unwrap();
        fs::write(&absolute_artifact_path, &artifact_bytes).unwrap();
        let license_path = format!("dist/{release_id}/license-material/license/LICENSE");
        let notice_path = format!("dist/{release_id}/license-material/notice/NOTICE");
        let absolute_license_path = receipt_dir.join(&license_path);
        let absolute_notice_path = receipt_dir.join(&notice_path);
        fs::create_dir_all(absolute_license_path.parent().unwrap()).unwrap();
        fs::create_dir_all(absolute_notice_path.parent().unwrap()).unwrap();
        fs::write(&absolute_license_path, &license_bytes).unwrap();
        fs::write(&absolute_notice_path, &notice_bytes).unwrap();
        let receipt = json!({
            "contract": "ait.release.adapter.receipt/v1",
            "release_id": release_id,
            "repo_name": repo_name,
            "version": version,
            "snapshot_id": snapshot_id,
            "profile": "generic-command",
            "target": target,
            "status": "built",
            "check_summary": {"decision": "pass"},
            "metadata": {
                "release_adapter": {
                    "contract": "ait.release.adapter/v1",
                    "manifest_path": "ait-release.json",
                    "manifest_sha256": adapter_manifest_sha256,
                    "component_count": 1,
                    "declared_artifact_count": targets.len(),
                    "license_material_count": 2,
                    "definition": {
                        "schema": "ait.release.adapter/v1",
                        "package": {
                            "name": component,
                            "version": version,
                            "license_files": [
                                {"path": "LICENSE", "role": "license"},
                                {"path": "NOTICE", "role": "notice"}
                            ]
                        },
                        "components": [{
                            "id": component,
                            "ecosystem": ecosystem,
                            "working_directory": ".",
                            "dependency_files": ["lockfile"],
                            "commands": {
                                "test": [["fixture-test"]],
                                "build": [["fixture-build"]]
                            },
                            "artifacts": declared_artifacts
                        }]
                    },
                },
                "build": {
                    "builder": "ait_generic_command_adapter_v1",
                    "adapter_contract": "ait.release.adapter/v1",
                    "adapter_manifest_sha256": adapter_manifest_sha256,
                    "component_count": 1,
                    "declared_artifact_count": 1,
                    "license_material_count": 2,
                    "components": [{
                        "component": component,
                        "ecosystem": ecosystem,
                        "status": "pass"
                    }]
                }
            },
            "artifacts": [
                {
                    "role": "component-artifact",
                    "component": component,
                    "ecosystem": ecosystem,
                    "kind": kind,
                    "target": target,
                    "declared_path": declared_path,
                    "path": artifact_path,
                    "size_bytes": artifact_bytes.len(),
                    "sha256": digest(&artifact_bytes)
                },
                {
                    "role": "license-material",
                    "kind": "license-material",
                    "material_role": "license",
                    "declared_path": "LICENSE",
                    "source_path": "LICENSE",
                    "path": license_path,
                    "size_bytes": license_bytes.len(),
                    "sha256": digest(&license_bytes)
                },
                {
                    "role": "license-material",
                    "kind": "license-material",
                    "material_role": "notice",
                    "declared_path": "NOTICE",
                    "source_path": "NOTICE",
                    "path": notice_path,
                    "size_bytes": notice_bytes.len(),
                    "sha256": digest(&notice_bytes)
                },
                {
                    "role": "release-manifest",
                    "kind": "manifest",
                    "path": format!("dist/{release_id}/ait-release.manifest.json"),
                    "size_bytes": 1,
                    "sha256": "b".repeat(64)
                },
                {
                    "role": "release-checksum",
                    "kind": "checksum",
                    "path": format!("dist/{release_id}/ait-release.sha256"),
                    "size_bytes": 1,
                    "sha256": "c".repeat(64)
                }
            ]
        });
        fs::write(
            receipt_dir.join("ait-release.receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn public_family_release_freezes_six_targets_and_emits_rc_handoff_without_release_store() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);

    let manifest = json!({
        "schema": "ait.release.family/v3",
        "family": {
            "name": "ait-native",
            "version": "1.0.0-rc.1",
            "channel": "rc",
            "tag": "v1.0.0-rc.1"
        },
        "targets": TARGETS,
        "public_source": public_source(&["ait-core", "ait-python"]),
        "components": [
            {
                "id": "ait",
                "source_repository": "ait-core",
                "source_snapshot": "SNP-111111111111",
                "ecosystem": "native",
                "license": "Apache-2.0",
                "version_scheme": "family",
                "version": "1.0.0-rc.1",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            },
            {
                "id": "ait-python",
                "source_repository": "ait-python",
                "source_snapshot": "SNP-222222222222",
                "ecosystem": "python",
                "license": "Apache-2.0",
                "version_scheme": "pep440",
                "version": "1.0.0rc1",
                "artifacts": [{"kind": "python-wheel", "targets": TARGETS}]
            }
        ],
        "distributions": [
            {
                "channel": "pypi",
                "role": "product",
                "identity": "ait-native",
                "components": ["ait", "ait-python"],
                "targets": TARGETS
            },
            github_distribution(&["ait", "ait-python"], TARGETS)
        ],
        "compatibility": {
            "native_protocol": "ait-native/v1",
            "python_abi": "abi3"
        }
    });
    fs::write(
        root.join("ait-release-family.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "bind family RC manifest",
            "--json",
        ],
    );

    let candidate = run_json(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--json",
        ],
    );
    assert_eq!(candidate["contract"], "ait.release.family.candidate/v1");
    assert_eq!(candidate["profile"], "family");
    assert_eq!(
        candidate["authority"]["persistence"],
        "portable_dist_dossier"
    );
    assert_eq!(candidate["authority"]["binary_db_layout_changed"], false);
    let release_id = candidate["release_id"].as_str().unwrap().to_string();

    let repeated = run_json(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--json",
        ],
    );
    assert_eq!(repeated, candidate);

    let receipts = root.join("component-receipts");
    write_component_receipts(
        &receipts,
        "ait",
        "ait-core",
        "SNP-111111111111",
        "native",
        "1.0.0-rc.1",
        "native-executable",
    );
    write_component_receipts(
        &receipts,
        "ait-python",
        "ait-python",
        "SNP-222222222222",
        "python",
        "1.0.0rc1",
        "python-wheel",
    );
    let receipt_arg = receipts.to_str().unwrap();
    let conflicting_receipt_path = receipts
        .join("ait")
        .join("x86_64-unknown-linux-gnu")
        .join("ait-release.receipt.json");
    let mut conflicting_receipt: Value =
        serde_json::from_slice(&fs::read(&conflicting_receipt_path).unwrap()).unwrap();
    let conflicting_index = conflicting_receipt["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .position(|artifact| {
            artifact["role"] == "license-material" && artifact["material_role"] == "license"
        })
        .unwrap();
    let conflicting_bytes = b"ait-core:conflicting-license\n";
    let conflicting_relative = conflicting_receipt["artifacts"][conflicting_index]["path"]
        .as_str()
        .unwrap();
    fs::write(
        conflicting_receipt_path
            .parent()
            .unwrap()
            .join(conflicting_relative),
        conflicting_bytes,
    )
    .unwrap();
    conflicting_receipt["artifacts"][conflicting_index]["size_bytes"] =
        json!(conflicting_bytes.len());
    conflicting_receipt["artifacts"][conflicting_index]["sha256"] =
        json!(digest(conflicting_bytes));
    fs::write(
        &conflicting_receipt_path,
        serde_json::to_vec_pretty(&conflicting_receipt).unwrap(),
    )
    .unwrap();
    let conflict = run(
        root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr)
        .contains("license material conflicts across target receipts"));
    write_component_receipts(
        &receipts,
        "ait",
        "ait-core",
        "SNP-111111111111",
        "native",
        "1.0.0-rc.1",
        "native-executable",
    );
    let checked = run_json(
        root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert_eq!(checked["status"], "checked");
    assert_eq!(checked["check_summary"]["decision"], "pass");
    assert_eq!(checked["component_receipts"].as_array().unwrap().len(), 12);
    assert_eq!(checked["artifacts"].as_array().unwrap().len(), 12);
    assert_eq!(checked["license_material"].as_array().unwrap().len(), 4);
    assert_eq!(checked["check_summary"]["total"], 6);

    let built = run_json(
        root,
        &[
            "release",
            "build",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert_eq!(built["status"], "built");
    assert_eq!(built["promotion"]["registry_write"], false);
    assert_eq!(
        built["promotion"]["source_publication"]["binary_publication_allowed"],
        false
    );
    assert_eq!(
        built["promotion"]["source_publication"]["requirements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        built["promotion"]["source_publication"]["requirements"][0]["github_repository"],
        "weita2026/ait-native"
    );
    assert_eq!(
        built["promotion"]["source_publication"]["requirements"][0]["subtrees"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(built["artifacts"].as_array().unwrap().len(), 18);
    assert_eq!(
        run_json(root, &["release", "show", &release_id, "--json"]),
        built
    );

    let promoted = run_json(
        root,
        &[
            "release",
            "promote",
            &release_id,
            "--channel",
            "rc",
            "--json",
        ],
    );
    assert_eq!(promoted["status"], "ready_for_protected_ci");
    assert_eq!(promoted["routes"]["github"]["prerelease"], true);
    assert_eq!(promoted["routes"]["github"]["draft"], false);
    assert_eq!(promoted["routes"]["npm"]["dist_tag"], "rc");
    assert_eq!(promoted["routes"]["pypi"]["prerelease"], true);
    assert_eq!(promoted["routes"]["oci"]["moving_tag"], "rc");
    assert_eq!(promoted["routes"]["homebrew"]["channel"], "rc");
    assert_eq!(promoted["routes"]["apt"]["suite"], "testing");
    assert_eq!(promoted["routes"]["winget"]["route"], "validation");
    assert_eq!(
        promoted["routes"]["winget"]["community_manifest_submission"],
        false
    );
    assert_eq!(
        promoted["routes"]["distributions"][0]["identity"],
        "ait-native"
    );
    assert_eq!(promoted["mutation"]["registry_write"], false);
    assert_eq!(promoted["mutation"]["credentials_loaded"], false);
    assert_eq!(
        promoted["authorization"]["public_source_readback_required"],
        true
    );
    assert_eq!(
        promoted["authorization"]["snapshot_to_git_tree_equality_required"],
        true
    );
    assert_eq!(
        promoted["authorization"]["binary_publication_before_source_allowed"],
        false
    );
    assert_eq!(
        promoted["source_publication"]["publication_order"],
        "all_source_before_any_binary_endpoint"
    );
    assert_eq!(
        promoted["source_publication"]["requirements"][0]["status"],
        "required_unverified"
    );
    assert!(promoted["source_publication"]["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["public_source_url"]
            .as_str()
            .unwrap()
            .ends_with("/tree/v1.0.0-rc.1")));
    assert_eq!(
        run_json(root, &["release", "show", &release_id, "--json"]),
        promoted
    );

    let publish = run(root, &["release", "publish", &release_id, "--json"]);
    assert!(!publish.status.success());
    assert!(String::from_utf8_lossy(&publish.stderr).contains("use `ait release promote"));

    let first_frozen_license = built["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["role"] == "license-material")
        .unwrap()["path"]
        .as_str()
        .unwrap();
    let frozen_license_path = root.join(first_frozen_license);
    let frozen_license_bytes = fs::read(&frozen_license_path).unwrap();
    fs::write(&frozen_license_path, b"tampered license\n").unwrap();
    let tampered_license = run(root, &["release", "show", &release_id, "--json"]);
    assert!(!tampered_license.status.success());
    assert!(
        String::from_utf8_lossy(&tampered_license.stderr).contains("does not match its SHA-256")
    );
    fs::write(&frozen_license_path, frozen_license_bytes).unwrap();

    let first_frozen_artifact = built["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["role"] == "component-artifact")
        .unwrap()["path"]
        .as_str()
        .unwrap();
    fs::write(root.join(first_frozen_artifact), b"tampered\n").unwrap();
    let tampered = run(root, &["release", "show", &release_id, "--json"]);
    assert!(!tampered.status.success());
    assert!(String::from_utf8_lossy(&tampered.stderr).contains("does not match its SHA-256"));
}

#[test]
fn family_package_assembles_native_channels_without_endpoint_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);

    let linux_targets = ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"];
    let manifest = json!({
        "schema": "ait.release.family/v3",
        "family": {
            "name": "ait-native",
            "version": "1.0.0-rc.1",
            "channel": "rc",
            "tag": "v1.0.0-rc.1"
        },
        "targets": TARGETS,
        "public_source": public_source(&["ait-core", "ait-server", "ait-runner"]),
        "components": [
            {
                "id": "ait",
                "source_repository": "ait-core",
                "source_snapshot": "SNP-111111111111",
                "ecosystem": "native",
                "license": "Apache-2.0",
                "version_scheme": "family",
                "version": "1.0.0-rc.1",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            },
            {
                "id": "ait-server",
                "source_repository": "ait-server",
                "source_snapshot": "SNP-222222222222",
                "ecosystem": "native",
                "license": "AGPL-3.0-only",
                "version_scheme": "family",
                "version": "1.0.0-rc.1",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            },
            {
                "id": "ait-runner",
                "source_repository": "ait-runner",
                "source_snapshot": "SNP-333333333333",
                "ecosystem": "native",
                "license": "Apache-2.0",
                "version_scheme": "family",
                "version": "1.0.0-rc.1",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            }
        ],
        "distributions": [
            {
                "channel": "homebrew",
                "role": "product",
                "identity": "ait-native",
                "components": ["ait", "ait-server", "ait-runner"],
                "targets": [
                    "aarch64-apple-darwin",
                    "x86_64-apple-darwin",
                    "aarch64-unknown-linux-gnu",
                    "x86_64-unknown-linux-gnu"
                ]
            },
            {
                "channel": "apt",
                "role": "product",
                "identity": "ait-native",
                "components": ["ait", "ait-server", "ait-runner"],
                "targets": linux_targets
            },
            {
                "channel": "apt",
                "role": "standalone",
                "identity": "ait-runner",
                "components": ["ait-runner"],
                "targets": linux_targets
            },
            {
                "channel": "winget",
                "role": "product",
                "identity": "Weita.AitNative",
                "components": ["ait", "ait-server", "ait-runner"],
                "targets": [
                    "aarch64-pc-windows-msvc",
                    "x86_64-pc-windows-msvc"
                ]
            },
            github_distribution(&["ait", "ait-server", "ait-runner"], TARGETS)
        ],
        "compatibility": {"native_protocol": "ait-native/v1"}
    });
    fs::write(
        root.join("ait-release-family.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "bind native channel fixture",
            "--json",
        ],
    );
    let candidate = run_json(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--json",
        ],
    );
    let release_id = candidate["release_id"].as_str().unwrap().to_string();

    let receipts = root.join("component-receipts");
    write_component_receipts(
        &receipts,
        "ait",
        "ait-core",
        "SNP-111111111111",
        "native",
        "1.0.0-rc.1",
        "native-executable",
    );
    write_component_receipts(
        &receipts,
        "ait-server",
        "ait-server",
        "SNP-222222222222",
        "native",
        "1.0.0-rc.1",
        "native-executable",
    );
    write_component_receipts(
        &receipts,
        "ait-runner",
        "ait-runner",
        "SNP-333333333333",
        "native",
        "1.0.0-rc.1",
        "native-executable",
    );
    let receipt_arg = receipts.to_str().unwrap();
    run_json(
        root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    run_json(
        root,
        &[
            "release",
            "build",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );

    let homebrew = run_json(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "homebrew",
            "--json",
        ],
    );
    let apt = run_json(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "apt",
            "--json",
        ],
    );
    let winget = run_json(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "winget",
            "--json",
        ],
    );
    assert_eq!(homebrew["artifact_count"], 5);
    assert_eq!(apt["artifact_count"], 4);
    assert_eq!(winget["artifact_count"], 5);
    for receipt in [&homebrew, &apt, &winget] {
        assert_eq!(receipt["status"], "assembled");
        assert_eq!(receipt["check_summary"]["decision"], "pass");
        assert_eq!(receipt["mutation"]["component_rebuild"], false);
        assert_eq!(receipt["mutation"]["credentials_loaded"], false);
        assert_eq!(receipt["mutation"]["signing"], false);
        assert_eq!(receipt["mutation"]["registry_write"], false);
        assert_eq!(receipt["mutation"]["public_publish"], false);
        assert_eq!(receipt["mutation"]["service_start"], false);
        assert_eq!(receipt["mutation"]["service_enable"], false);
        assert_eq!(receipt["mutation"]["service_registration"], false);
        assert_eq!(
            receipt["mutation"]["server_authority_initialization"],
            false
        );
    }
    assert_eq!(homebrew["route"]["stable_formula_mutation"], false);
    assert_eq!(apt["route"]["suite"], "testing");
    assert_eq!(winget["route"]["route"], "validation");
    assert_eq!(winget["route"]["community_manifest_submission"], false);
    assert!(winget["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|artifact| artifact["kind"] == "winget-portable-zip")
        .all(|artifact| artifact["metadata"].get("scope").is_none()
            && artifact["metadata"]["portable_commands"]
                == json!(["ait", "ait-server", "ait-runner"])
            && artifact["metadata"]["portable_invocation_parameters"]
                == json!({"ait": "--help", "ait-server": "--help", "ait-runner": "--help"})
            && artifact["metadata"]["runner_activation"] == "inactive"
            && artifact["metadata"]["runner_controller"] == false));
    assert_eq!(
        run_json(
            root,
            &[
                "release",
                "package",
                &release_id,
                "--channel",
                "homebrew",
                "--json",
            ],
        ),
        homebrew
    );

    let package_root = root.join("dist").join(&release_id).join("packages");
    let formula_path = package_root
        .join("homebrew")
        .join("Formula")
        .join("ait-native-rc.rb");
    let formula = fs::read_to_string(&formula_path).unwrap();
    assert!(formula.contains("class AitNativeRc < Formula"));
    assert!(!formula.contains("\n  version \""));
    assert!(formula.contains("bin.install \"bin/ait\""));
    assert!(formula.contains("bin.install \"bin/ait-server\""));
    assert!(formula.contains("bin.install \"bin/ait-runner\""));
    assert!(formula.contains("service do"));
    assert!(formula.contains(
        "run [\n      opt_bin/\"ait-server\",\n      \"--data\",\n      var/\"ait-native/server-data\",\n      \"--init-if-missing\",\n      \"--defer-ci-admission\",\n    ]"
    ));
    assert!(!formula.contains("opt_bin/\"ait-server\",\n      \"run\""));
    assert!(formula.contains("keep_alive true"));
    assert!(formula.contains("brew services start ait-native-rc"));
    assert!(formula.contains("Service data: #{var}/ait-native/server-data"));
    assert!(
        formula.contains("ait-runner is installed but no runner daemon is configured or started")
    );
    assert!(formula.contains("#{bin}/ait-runner serve --help"));
    assert!(!formula.contains("ait ci-host"));
    let formula_evidence = homebrew["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "homebrew-formula")
        .unwrap();
    assert_eq!(formula_evidence["metadata"]["server_service_stanza"], true);
    assert_eq!(
        formula_evidence["metadata"]["server_activation"],
        "explicit_brew_services_start"
    );
    assert_eq!(formula_evidence["metadata"]["runner_included"], true);
    assert_eq!(
        formula_evidence["metadata"]["runner_activation"],
        "inactive"
    );
    assert_eq!(formula_evidence["metadata"]["runner_service_stanza"], false);

    let homebrew_archive = fs::read(
        package_root
            .join("homebrew")
            .join("archives")
            .join("ait-native-1.0.0-rc.1-x86_64-unknown-linux-gnu.tar.gz"),
    )
    .unwrap();
    let homebrew_members = tar_gz_members(&homebrew_archive);
    for path in [
        "bin/ait",
        "bin/ait-server",
        "bin/ait-runner",
        "share/licenses/ait-core/LICENSE",
        "share/licenses/ait-core/NOTICE",
        "share/licenses/ait-server/LICENSE",
        "share/licenses/ait-server/NOTICE",
        "share/licenses/ait-runner/LICENSE",
        "share/licenses/ait-runner/NOTICE",
        "share/ait-native/ait-family-provenance.json",
    ] {
        assert!(homebrew_members.contains_key(path), "missing {path}");
    }
    let homebrew_provenance: Value =
        serde_json::from_slice(&homebrew_members["share/ait-native/ait-family-provenance.json"])
            .unwrap();
    assert_eq!(
        homebrew_provenance["component_content"][0]["installed_path"],
        "bin/ait"
    );
    assert_eq!(
        homebrew_provenance["component_content"][1]["installed_path"],
        "bin/ait-server"
    );
    assert_eq!(
        homebrew_provenance["component_content"][1]["public_source_url"],
        "https://github.com/weita2026/ait-native/tree/v1.0.0-rc.1/ait-server"
    );
    assert_eq!(
        homebrew_provenance["component_content"][2]["installed_path"],
        "bin/ait-runner"
    );
    assert!(homebrew_provenance["license_material"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["public_source_url"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/weita2026/ait-native/tree/v1.0.0-rc.1/ait-")));

    let debian_bytes = fs::read(
        package_root
            .join("apt")
            .join("packages")
            .join("ait-native_1.0.0~rc.1_amd64.deb"),
    )
    .unwrap();
    let debian_members = ar_members(&debian_bytes);
    assert_eq!(debian_members["debian-binary"], b"2.0\n");
    let control_members = tar_gz_members(&debian_members["control.tar.gz"]);
    let control = String::from_utf8(control_members["control"].clone()).unwrap();
    assert!(control.contains("Package: ait-native\n"));
    assert!(control.contains("Version: 1.0.0~rc.1\n"));
    assert!(control.contains("Architecture: amd64\n"));
    assert_eq!(control_members.len(), 1);
    assert_eq!(
        tar_gz_directory_entries(&debian_members["control.tar.gz"]),
        BTreeMap::from([("control".to_string(), false)])
    );
    let data_members = tar_gz_members(&debian_members["data.tar.gz"]);
    assert_regular_file_parents_are_directories(&debian_members["data.tar.gz"]);
    let data_directories = tar_gz_directory_entries(&debian_members["data.tar.gz"]);
    for directory in [
        "usr",
        "usr/bin",
        "usr/lib",
        "usr/lib/systemd",
        "usr/lib/systemd/system",
        "usr/share",
        "usr/share/doc",
        "usr/share/doc/ait-native",
        "usr/share/doc/ait-native/licenses",
    ] {
        assert_eq!(
            data_directories.get(directory),
            Some(&true),
            "missing directory entry {directory:?}"
        );
    }
    for path in [
        "usr/bin/ait",
        "usr/bin/ait-server",
        "usr/bin/ait-runner",
        "usr/share/doc/ait-native/licenses/ait-core/LICENSE",
        "usr/share/doc/ait-native/licenses/ait-server/NOTICE",
        "usr/share/doc/ait-native/licenses/ait-runner/LICENSE",
        "usr/share/doc/ait-native/ait-family-provenance.json",
        "usr/share/doc/ait-native/copyright",
        "usr/lib/systemd/system/ait-server.service",
    ] {
        assert!(data_members.contains_key(path), "missing {path}");
    }
    let systemd_unit =
        String::from_utf8(data_members["usr/lib/systemd/system/ait-server.service"].clone())
            .unwrap();
    assert!(systemd_unit.contains("DynamicUser=yes\n"));
    assert!(systemd_unit.contains("StateDirectory=ait-native\n"));
    assert!(systemd_unit.contains(
        "ExecStart=/usr/bin/ait-server --data /var/lib/ait-native/server-data --init-if-missing --defer-ci-admission\n"
    ));
    assert!(systemd_unit.contains("ProtectSystem=strict\n"));
    assert!(systemd_unit.contains("WantedBy=multi-user.target\n"));
    let copyright =
        String::from_utf8(data_members["usr/share/doc/ait-native/copyright"].clone()).unwrap();
    assert!(copyright.contains(
        "Files: usr/bin/ait\nCopyright: 2026 Weita and contributors\nLicense: Apache-2.0"
    ));
    assert!(copyright.contains(
        "Files: usr/bin/ait-server\nCopyright: 2026 Weita and contributors\nLicense: AGPL-3.0-only"
    ));
    assert!(copyright.contains(
        "Files: usr/bin/ait-runner\nCopyright: 2026 Weita and contributors\nLicense: Apache-2.0"
    ));
    assert!(copyright.contains("Files: usr/share/doc/ait-native/licenses/ait-core/*"));
    assert!(copyright.contains("Files: usr/share/doc/ait-native/licenses/ait-server/*"));
    assert!(copyright.contains("Files: usr/share/doc/ait-native/licenses/ait-runner/*"));
    assert!(copyright.contains("usr/lib/systemd/system/ait-server.service"));
    assert!(copyright.contains("/usr/share/common-licenses/Apache-2.0"));
    assert!(copyright.contains("/usr/share/common-licenses/AGPL-3"));
    assert!(!copyright.contains("Files: *"));
    assert!(!copyright.contains("AGPL-3.0-only AND Apache-2.0"));
    let runner_debian_bytes = fs::read(
        package_root
            .join("apt")
            .join("packages")
            .join("ait-runner_1.0.0~rc.1_amd64.deb"),
    )
    .unwrap();
    let runner_debian_members = ar_members(&runner_debian_bytes);
    let runner_control_members = tar_gz_members(&runner_debian_members["control.tar.gz"]);
    let runner_control = String::from_utf8(runner_control_members["control"].clone()).unwrap();
    assert!(runner_control.contains("Depends: ait-native (= 1.0.0~rc.1), libc6 (>= 2.28)"));
    let runner_data_members = tar_gz_members(&runner_debian_members["data.tar.gz"]);
    assert_regular_file_parents_are_directories(&runner_debian_members["data.tar.gz"]);
    assert!(!runner_data_members.contains_key("usr/bin/ait-runner"));
    assert!(!runner_data_members.contains_key("usr/lib/systemd/system/ait-server.service"));
    assert!(runner_data_members.contains_key("usr/share/doc/ait-runner/ait-family-provenance.json"));
    let apt_product_evidence = apt["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| {
            row["kind"] == "debian-package"
                && row["distribution_identity"] == "ait-native"
                && row["target"] == "x86_64-unknown-linux-gnu"
        })
        .unwrap();
    assert_eq!(apt_product_evidence["metadata"]["systemd_unit"], true);
    assert_eq!(
        apt_product_evidence["metadata"]["systemd_unit_path"],
        "usr/lib/systemd/system/ait-server.service"
    );
    assert_eq!(apt_product_evidence["metadata"]["runner_included"], true);
    assert_eq!(
        apt_product_evidence["metadata"]["runner_activation"],
        "inactive"
    );
    assert_eq!(
        apt_product_evidence["metadata"]["runner_systemd_unit"],
        false
    );
    assert_eq!(
        apt_product_evidence["metadata"]["maintainer_script_count"],
        0
    );
    let apt_runner_alias_evidence = apt["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| {
            row["kind"] == "debian-package"
                && row["distribution_identity"] == "ait-runner"
                && row["target"] == "x86_64-unknown-linux-gnu"
        })
        .unwrap();
    assert_eq!(
        apt_runner_alias_evidence["metadata"]["transitional_dependency_alias"],
        true
    );
    assert_eq!(
        apt_runner_alias_evidence["metadata"]["runner_payload_owner"],
        "ait-native"
    );

    let installer_manifest = fs::read_to_string(
        package_root
            .join("winget")
            .join("manifests")
            .join("Weita.AitNative.installer.yaml"),
    )
    .unwrap();
    assert!(installer_manifest.contains("InstallerType: zip"));
    assert!(installer_manifest.contains("NestedInstallerType: portable"));
    assert!(installer_manifest.contains("PortableCommandAlias: ait\n"));
    assert!(installer_manifest.contains("PortableCommandAlias: ait-server\n"));
    assert!(installer_manifest.contains("PortableCommandAlias: ait-runner\n"));
    assert!(!installer_manifest.contains("\n    Scope:"));
    assert!(!installer_manifest.contains("RelativeFilePath: ait-server-control.ps1"));
    assert!(!installer_manifest.contains("PortableCommandAlias: ait-server-control.ps1"));
    assert_eq!(
        installer_manifest.matches("InstallationMetadata:").count(),
        2
    );
    assert_eq!(installer_manifest.matches("FileType: launch").count(), 6);
    assert_eq!(
        installer_manifest
            .matches("InvocationParameter: --help")
            .count(),
        6
    );
    assert_eq!(installer_manifest.matches("RelativeFilePath:").count(), 12);
    assert!(installer_manifest.contains("ManifestVersion: 1.12.0"));
    let locale_manifest = fs::read_to_string(
        package_root
            .join("winget")
            .join("manifests")
            .join("Weita.AitNative.locale.en-US.yaml"),
    )
    .unwrap();
    assert!(locale_manifest.contains(
        "License: \"AGPL-3.0-only AND Apache-2.0\"\nLicenseUrl: \"https://github.com/weita2026/ait-native/blob/v1.0.0-rc.1/docs/distribution.md#license-and-source-publication-gate\""
    ));
    let winget_zip = fs::read(
        package_root
            .join("winget")
            .join("installers")
            .join("ait-native-1.0.0-rc.1-x86_64-pc-windows-msvc.zip"),
    )
    .unwrap();
    let winget_members = zip_members(&winget_zip);
    for path in [
        "ait.exe",
        "ait-server.exe",
        "ait-runner.exe",
        "licenses/ait-core/LICENSE",
        "licenses/ait-core/NOTICE",
        "licenses/ait-server/LICENSE",
        "licenses/ait-server/NOTICE",
        "licenses/ait-runner/LICENSE",
        "licenses/ait-runner/NOTICE",
        "ait-family-provenance.json",
        "ait-server-control.ps1",
    ] {
        assert!(winget_members.contains_key(path), "missing {path}");
    }
    assert!(!winget_members.contains_key("bin/ait.exe"));
    let controller = String::from_utf8(winget_members["ait-server-control.ps1"].clone()).unwrap();
    assert!(controller.contains("ValidateSet('init', 'probe', 'start', 'status', 'stop')"));
    assert!(controller.contains("Join-Path $env:LOCALAPPDATA 'AIT\\server-data'"));
    assert!(controller.contains("PID $ManagedProcessId belongs to another executable"));
    assert!(controller.contains(
        "@('--data', $DataRoot, '--listen', $Listen, '--init-if-missing', '--defer-ci-admission')"
    ));
    assert!(!controller.contains("'run'"));
    assert!(controller.contains("Stop-Process -Id $Managed.Id"));
    assert!(!controller.contains("sc.exe"));
    assert!(!controller.contains("New-Service"));
    let winget_provenance: Value =
        serde_json::from_slice(&winget_members["ait-family-provenance.json"]).unwrap();
    assert_eq!(
        winget_provenance["component_content"][0]["installed_path"],
        "ait.exe"
    );
    assert_eq!(
        winget_provenance["component_content"][1]["installed_path"],
        "ait-server.exe"
    );
    assert_eq!(
        winget_provenance["component_content"][2]["installed_path"],
        "ait-runner.exe"
    );

    fs::write(&formula_path, b"tampered formula\n").unwrap();
    let tampered = run(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "homebrew",
            "--json",
        ],
    );
    assert!(!tampered.status.success());
    assert!(
        String::from_utf8_lossy(&tampered.stderr).contains("differs from deterministic assembly")
    );
    assert_eq!(fs::read(&formula_path).unwrap(), b"tampered formula\n");
}

#[test]
fn family_package_assembles_registry_channels_without_endpoint_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);

    let manifest = json!({
        "schema": "ait.release.family/v3",
        "family": {
            "name": "ait-native",
            "version": "1.0.0-rc.2",
            "channel": "rc",
            "tag": "v1.0.0-rc.2"
        },
        "targets": TARGETS,
        "public_source": public_source(&["ait-core", "ait-server", "ait-python", "ait-node"]),
        "components": [
            {
                "id": "ait",
                "source_repository": "ait-core",
                "source_snapshot": "SNP-111111111111",
                "ecosystem": "native",
                "license": "Apache-2.0",
                "version_scheme": "family",
                "version": "1.0.0-rc.2",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            },
            {
                "id": "ait-server",
                "source_repository": "ait-server",
                "source_snapshot": "SNP-222222222222",
                "ecosystem": "native",
                "license": "AGPL-3.0-only",
                "version_scheme": "family",
                "version": "1.0.0-rc.2",
                "artifacts": [{"kind": "native-executable", "targets": TARGETS}]
            },
            {
                "id": "ait-python",
                "source_repository": "ait-python",
                "source_snapshot": "SNP-333333333333",
                "ecosystem": "python",
                "license": "Apache-2.0",
                "version_scheme": "pep440",
                "version": "1.0.0rc2",
                "artifacts": [{"kind": "python-wheel", "targets": TARGETS}]
            },
            {
                "id": "ait-node",
                "source_repository": "ait-node",
                "source_snapshot": "SNP-444444444444",
                "ecosystem": "node",
                "license": "Apache-2.0",
                "version_scheme": "family",
                "version": "1.0.0-rc.2",
                "artifacts": [
                    {"kind": "npm-napi-envelope", "targets": []},
                    {"kind": "npm-napi-addon", "targets": TARGETS}
                ]
            }
        ],
        "distributions": [
            {
                "channel": "pypi",
                "role": "product",
                "identity": "ait-native",
                "components": ["ait", "ait-server", "ait-python"],
                "targets": TARGETS
            },
            {
                "channel": "npm",
                "role": "product",
                "identity": "@wa120/ait-native",
                "components": ["ait-node"],
                "targets": TARGETS
            },
            github_distribution(&["ait", "ait-server", "ait-python", "ait-node"], TARGETS)
        ],
        "compatibility": {
            "native_protocol": "ait-native/v1",
            "python_abi": "abi3",
            "npm_surface": "direct-napi"
        }
    });
    fs::write(
        root.join("ait-release-family.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "bind registry channel fixture",
            "--json",
        ],
    );
    let candidate = run_json(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.2",
            "--channel",
            "rc",
            "--json",
        ],
    );
    let release_id = candidate["release_id"].as_str().unwrap().to_string();

    let receipts = root.join("component-receipts");
    write_component_receipts(
        &receipts,
        "ait",
        "ait-core",
        "SNP-111111111111",
        "native",
        "1.0.0-rc.2",
        "native-executable",
    );
    write_component_receipts(
        &receipts,
        "ait-server",
        "ait-server",
        "SNP-222222222222",
        "native",
        "1.0.0-rc.2",
        "native-executable",
    );
    let mut wheel_fixtures = Vec::new();
    let mut source_bindings = BTreeMap::new();
    for target in TARGETS {
        let (filename, bytes, extension_path, extension_bytes) = fixture_wheel(target);
        wheel_fixtures.push(ReceiptArtifactFixture {
            target: Some((*target).to_string()),
            declared_path: filename,
            bytes,
        });
        source_bindings.insert((*target).to_string(), (extension_path, extension_bytes));
    }
    write_custom_component_receipts(
        &receipts,
        "ait-python",
        "ait-python",
        "SNP-333333333333",
        "python",
        "1.0.0rc2",
        "python-wheel",
        &wheel_fixtures,
    );
    let envelope_bytes = fixture_npm_envelope();
    write_custom_component_receipts(
        &receipts,
        "ait-node",
        "ait-node",
        "SNP-444444444444",
        "node",
        "1.0.0-rc.2",
        "npm-napi-envelope",
        &[ReceiptArtifactFixture {
            target: None,
            declared_path: "wa120-ait-native-1.0.0-rc.2.tgz".to_string(),
            bytes: envelope_bytes.clone(),
        }],
    );
    let mut addon_fixtures = Vec::new();
    let mut source_addons = BTreeMap::new();
    for target in TARGETS {
        let (filename, bytes, addon_bytes) = fixture_npm_addon(target);
        addon_fixtures.push(ReceiptArtifactFixture {
            target: Some((*target).to_string()),
            declared_path: filename,
            bytes,
        });
        source_addons.insert((*target).to_string(), addon_bytes);
    }
    write_custom_component_receipts(
        &receipts,
        "ait-node",
        "ait-node",
        "SNP-444444444444",
        "node",
        "1.0.0-rc.2",
        "npm-napi-addon",
        &addon_fixtures,
    );
    let receipt_arg = receipts.to_str().unwrap();
    let checked = run_json(
        root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert_eq!(checked["artifacts"].as_array().unwrap().len(), 25);
    assert_eq!(checked["license_material"].as_array().unwrap().len(), 8);
    let built = run_json(
        root,
        &[
            "release",
            "build",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );

    let pypi = run_json(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "pypi",
            "--json",
        ],
    );
    let npm = run_json(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "npm",
            "--json",
        ],
    );
    assert_eq!(pypi["artifact_count"], 6);
    assert_eq!(npm["artifact_count"], 7);
    assert_eq!(pypi["route"]["repository"], "pypi");
    assert_eq!(pypi["route"]["prerelease"], true);
    assert_eq!(npm["route"]["dist_tag"], "rc");
    let npm_artifacts = npm["artifacts"].as_array().unwrap();
    let npm_linux = npm_artifacts
        .iter()
        .find(|artifact| {
            artifact["kind"] == "npm-napi-addon" && artifact["target"] == "x86_64-unknown-linux-gnu"
        })
        .unwrap();
    assert_eq!(npm_linux["metadata"]["libc"], "glibc");
    let npm_darwin = npm_artifacts
        .iter()
        .find(|artifact| {
            artifact["kind"] == "npm-napi-addon" && artifact["target"] == "aarch64-apple-darwin"
        })
        .unwrap();
    assert_eq!(npm_darwin["metadata"]["libc"], Value::Null);
    for receipt in [&pypi, &npm] {
        assert_eq!(receipt["status"], "assembled");
        assert_eq!(receipt["check_summary"]["decision"], "pass");
        assert_eq!(receipt["mutation"]["component_rebuild"], false);
        assert_eq!(receipt["mutation"]["credentials_loaded"], false);
        assert_eq!(receipt["mutation"]["signing"], false);
        assert_eq!(receipt["mutation"]["registry_write"], false);
        assert_eq!(receipt["mutation"]["public_publish"], false);
        assert_eq!(receipt["mutation"]["service_start"], false);
    }
    assert_eq!(
        run_json(
            root,
            &[
                "release",
                "package",
                &release_id,
                "--channel",
                "pypi",
                "--json",
            ],
        ),
        pypi
    );
    assert_eq!(
        run_json(
            root,
            &[
                "release",
                "package",
                &release_id,
                "--channel",
                "npm",
                "--json",
            ],
        ),
        npm
    );

    let package_root = root.join("dist").join(&release_id).join("packages");
    let wheel_path = package_root
        .join("pypi")
        .join("wheels")
        .join("ait_native-1.0.0rc2-cp311-abi3-manylinux_2_28_x86_64.whl");
    let wheel_members = zip_members(&fs::read(&wheel_path).unwrap());
    let output_dist_info = "ait_native-1.0.0rc2.dist-info";
    assert!(!wheel_members
        .keys()
        .any(|path| path.starts_with("ait_python-1.0.0rc2.dist-info/")));
    let (binding_path, binding_bytes) = &source_bindings["x86_64-unknown-linux-gnu"];
    assert_eq!(&wheel_members[binding_path], binding_bytes);
    assert_eq!(
        wheel_members["ait_native-1.0.0rc2.data/scripts/ait"],
        b"ait:1.0.0-rc.2:x86_64-unknown-linux-gnu\n"
    );
    assert_eq!(
        wheel_members["ait_native-1.0.0rc2.data/scripts/ait-server"],
        b"ait-server:1.0.0-rc.2:x86_64-unknown-linux-gnu\n"
    );
    for path in [
        "ait_native-1.0.0rc2.dist-info/licenses/ait-core/LICENSE",
        "ait_native-1.0.0rc2.dist-info/licenses/ait-core/NOTICE",
        "ait_native-1.0.0rc2.dist-info/licenses/ait-server/LICENSE",
        "ait_native-1.0.0rc2.dist-info/licenses/ait-server/NOTICE",
        "ait_native-1.0.0rc2.dist-info/licenses/ait-python/LICENSE",
        "ait_native-1.0.0rc2.dist-info/licenses/ait-python/NOTICE",
        "ait_native-1.0.0rc2.dist-info/ait-family-provenance.json",
    ] {
        assert!(wheel_members.contains_key(path), "missing {path}");
    }
    let metadata =
        String::from_utf8(wheel_members[&format!("{output_dist_info}/METADATA")].clone()).unwrap();
    assert!(metadata.contains("Name: ait-native\n"));
    assert!(metadata.contains("Version: 1.0.0rc2\n"));
    assert!(metadata.contains(
        "Summary: Agent-first, language-neutral workflow for verified repository changes\n"
    ));
    assert!(metadata.contains("Description-Content-Type: text/markdown\n"));
    assert!(metadata.contains("License-Expression: AGPL-3.0-only AND Apache-2.0\n"));
    assert!(metadata.contains("Project-URL: Homepage, https://ait-native.dev/\n"));
    assert!(
        metadata.contains("Project-URL: Quickstart, https://ait-native.dev/local-quickstart/\n")
    );
    assert!(metadata.contains(
        "Project-URL: Source, https://github.com/weita2026/ait-native/tree/v1.0.0-rc.2\n"
    ));
    assert!(metadata.contains(
        "Project-URL: Documentation, https://github.com/weita2026/ait-native/blob/v1.0.0-rc.2/docs/distribution.md\n"
    ));
    assert!(metadata.contains(
        "Project-URL: Migration, https://github.com/weita2026/ait-native/blob/v1.0.0-rc.2/docs/distribution.md#public-0x-to-10-transition\n"
    ));
    for storefront_marker in [
        "AIT turns an ordinary coding request into an isolated, sprint-bound repository",
        "individual developers and maintainers",
        "python -m pip install ait-native==1.0.0rc2",
        "ait init",
        "## What initialization provides",
        "Official website: <https://ait-native.dev/>",
        "## Upgrading from 0.x",
        "There is no `ait install` command in 1.x.",
        "ait workflow ready <change-id> --apply",
        "ait workflow finish <change-id> --apply",
    ] {
        assert!(
            metadata.contains(storefront_marker),
            "missing {storefront_marker}"
        );
    }
    assert!(!metadata.contains("@AIT_"));
    let provenance: Value = serde_json::from_slice(
        &wheel_members[&format!("{output_dist_info}/ait-family-provenance.json")],
    )
    .unwrap();
    assert_eq!(provenance["server_activation"], "inactive");
    assert_eq!(provenance["component_rebuild"], false);
    assert_eq!(provenance["registry_write"], false);
    assert_eq!(
        provenance["component_content"][0]["source_snapshot"],
        "SNP-333333333333"
    );
    assert_eq!(
        provenance["component_content"][1]["source_snapshot"],
        "SNP-111111111111"
    );
    assert_eq!(
        provenance["component_content"][2]["source_snapshot"],
        "SNP-222222222222"
    );
    let record_path = format!("{output_dist_info}/RECORD");
    let record_text = std::str::from_utf8(&wheel_members[&record_path]).unwrap();
    let record_rows = record_text
        .lines()
        .map(|line| {
            let fields = line.split(',').collect::<Vec<_>>();
            assert_eq!(fields.len(), 3);
            (
                fields[0].to_string(),
                (fields[1].to_string(), fields[2].to_string()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(record_rows.len(), wheel_members.len());
    for (path, bytes) in &wheel_members {
        let (hash, size) = &record_rows[path];
        if path == &record_path {
            assert!(hash.is_empty());
            assert!(size.is_empty());
        } else {
            assert_eq!(
                hash,
                &format!("sha256={}", URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)))
            );
            assert_eq!(size, &bytes.len().to_string());
        }
    }

    let npm_root = package_root.join("npm").join("packages");
    assert_eq!(
        fs::read(npm_root.join("wa120-ait-native-1.0.0-rc.2.tgz")).unwrap(),
        envelope_bytes
    );
    let envelope_members = tar_gz_members(&envelope_bytes);
    let envelope_package: Value =
        serde_json::from_slice(&envelope_members["package/package.json"]).unwrap();
    assert_eq!(
        envelope_package["description"],
        "Agent-first, language-neutral workflow for verified repository changes"
    );
    assert_eq!(envelope_package["homepage"], "https://ait-native.dev/");
    let envelope_readme = std::str::from_utf8(&envelope_members["package/README.md"]).unwrap();
    for storefront_marker in [
        "AIT turns an ordinary coding request into an isolated, sprint-bound repository",
        "individual developers and maintainers",
        "npm install --global @wa120/ait-native@1.0.0-rc.2",
        "ait init",
        "## What initialization provides",
        "https://ait-native.dev/",
        "## Upgrading from 0.x",
        "There is no `ait install` command in 1.0.",
        "ait workflow ready <change-id> --apply",
        "ait workflow finish <change-id> --apply",
    ] {
        assert!(
            envelope_readme.contains(storefront_marker),
            "missing {storefront_marker}"
        );
    }
    let addon_path = npm_root.join("wa120-ait-native-linux-x64-1.0.0-rc.2.tgz");
    let addon_package_bytes = fs::read(&addon_path).unwrap();
    let addon_members = tar_gz_members(&addon_package_bytes);
    assert_eq!(addon_members.len(), 5);
    assert_eq!(
        addon_members["package/native/ait_napi.node"],
        source_addons["x86_64-unknown-linux-gnu"]
    );
    assert_eq!(addon_members["package/LICENSE"], b"ait-node:license\n");
    assert_eq!(addon_members["package/NOTICE"], b"ait-node:notice\n");
    let addon_package: Value =
        serde_json::from_slice(&addon_members["package/package.json"]).unwrap();
    assert_eq!(addon_package["name"], "@wa120/ait-native-linux-x64");
    assert_eq!(addon_package["version"], "1.0.0-rc.2");
    assert_eq!(addon_package["os"], json!(["linux"]));
    assert_eq!(addon_package["cpu"], json!(["x64"]));
    assert_eq!(addon_package["libc"], json!(["glibc"]));
    assert_eq!(addon_package["main"], "native/ait_napi.node");
    assert_eq!(
        addon_package["aitNativeAddon"],
        json!({
            "schema": "ait.node.napi-platform-addon/v2",
            "component": "ait-node",
            "target": "x86_64-unknown-linux-gnu",
            "libc": "glibc",
            "addon": "native/ait_napi.node",
            "binding_repository": "ait-core",
            "binding_snapshot": "SNP-111111111111"
        })
    );
    for forbidden in ["bin", "exports", "scripts", "dependencies"] {
        assert!(addon_package.get(forbidden).is_none());
    }
    let addon_provenance: Value =
        serde_json::from_slice(&addon_members["package/provenance.json"]).unwrap();
    assert_eq!(addon_provenance["binding_snapshot"], "SNP-111111111111");
    assert_eq!(addon_provenance["libc"], "glibc");
    assert_eq!(addon_provenance["package_source_repository"], "ait-node");
    assert_eq!(addon_provenance["installed_path"], "native/ait_napi.node");

    let darwin_addon_path = npm_root.join("wa120-ait-native-darwin-arm64-1.0.0-rc.2.tgz");
    let darwin_members = tar_gz_members(&fs::read(darwin_addon_path).unwrap());
    let darwin_package: Value =
        serde_json::from_slice(&darwin_members["package/package.json"]).unwrap();
    assert!(darwin_package.get("libc").is_none());
    assert_eq!(darwin_package["aitNativeAddon"]["libc"], Value::Null);
    let darwin_provenance: Value =
        serde_json::from_slice(&darwin_members["package/provenance.json"]).unwrap();
    assert_eq!(darwin_provenance["libc"], Value::Null);

    let frozen_addon = built["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| {
            artifact["role"] == "component-artifact"
                && artifact["component"] == "ait-node"
                && artifact["kind"] == "npm-napi-addon"
                && artifact["target"] == "x86_64-unknown-linux-gnu"
        })
        .unwrap()["path"]
        .as_str()
        .unwrap();
    let frozen_addon_path = root.join(frozen_addon);
    let frozen_addon_bytes = fs::read(&frozen_addon_path).unwrap();
    fs::write(&frozen_addon_path, b"tampered frozen addon\n").unwrap();
    let frozen_tamper = run(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "npm",
            "--json",
        ],
    );
    assert!(!frozen_tamper.status.success());
    assert!(String::from_utf8_lossy(&frozen_tamper.stderr).contains("does not match its SHA-256"));
    fs::write(&frozen_addon_path, frozen_addon_bytes).unwrap();

    fs::write(&addon_path, b"tampered addon package\n").unwrap();
    let tampered = run(
        root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "npm",
            "--json",
        ],
    );
    assert!(!tampered.status.success());
    assert!(
        String::from_utf8_lossy(&tampered.stderr).contains("differs from deterministic assembly")
    );
    assert_eq!(fs::read(&addon_path).unwrap(), b"tampered addon package\n");
}

#[test]
fn release_help_exposes_product_lifecycle_and_hides_internal_adapter() {
    let temp = TempDir::new().unwrap();
    let output = run(temp.path(), &["release", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("candidate"));
    assert!(help.contains("check"));
    assert!(help.contains("build"));
    assert!(help.contains("package"));
    assert!(help.contains("show"));
    assert!(help.contains("promote"));
    assert!(!help.contains("adapter"));
}

#[test]
fn family_release_cli_rejects_wrong_channel_and_missing_receipts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);
    let manifest = json!({
        "schema": "ait.release.family/v3",
        "family": {
            "name": "ait-native",
            "version": "1.0.0-rc.1",
            "channel": "rc",
            "tag": "v1.0.0-rc.1"
        },
        "targets": ["x86_64-unknown-linux-gnu"],
        "public_source": public_source(&["ait-core"]),
        "components": [{
            "id": "ait",
            "source_repository": "ait-core",
            "source_snapshot": "SNP-111111111111",
            "ecosystem": "native",
            "license": "Apache-2.0",
            "version_scheme": "family",
            "version": "1.0.0-rc.1",
            "artifacts": [{
                "kind": "native-executable",
                "targets": ["x86_64-unknown-linux-gnu"]
            }]
        }],
        "distributions": [github_distribution(
            &["ait"],
            &["x86_64-unknown-linux-gnu"]
        )],
        "compatibility": {}
    });
    fs::write(
        root.join("ait-release-family.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "family",
            "--json",
            "--full",
        ],
    );

    let wrong = run(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "stable",
            "--json",
        ],
    );
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("does not match"));

    let candidate = run_json(
        root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--json",
        ],
    );
    let release_id = candidate["release_id"].as_str().unwrap();
    let missing = run(root, &["release", "check", release_id, "--json"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("requires --receipts"));

    for command in ["check", "build"] {
        let rejected = run(
            root,
            &[
                "release",
                command,
                "REL-LEGACY",
                "--public-source-root",
                root.to_str().unwrap(),
                "--json",
            ],
        );
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr)
            .contains("--public-source-root applies only to a family release candidate"));
    }

    let rejected_show = run(
        root,
        &[
            "release",
            "show",
            "REL-LEGACY",
            "--public-source-root",
            root.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected_show.status.success());
    assert!(String::from_utf8_lossy(&rejected_show.stderr)
        .contains("--public-source-root applies only to a family release dossier"));
}

#[test]
fn public_git_family_reconstructs_candidate_and_rejects_receipt_authority_drift() {
    let temp = TempDir::new().unwrap();
    let public_root = temp.path().join("public-source");
    let core_root = public_root.join("ait-core");
    fs::create_dir_all(&core_root).unwrap();
    run_json(&core_root, &["init", "--json"]);
    let admission_root = temp.path().join("family-admission");
    fs::create_dir(&admission_root).unwrap();
    initialize_internal_repo(&admission_root, "ait-core", "release-bootstrap");

    let target = "x86_64-unknown-linux-gnu";
    let source_snapshot = "SNP-111111111111";
    let coordinator_snapshot = "SNP-AAAAAAAAAAAA";
    let coordinator_manifest_hash = "c".repeat(64);
    let source_manifest_hash = "d".repeat(64);
    let mapping_content_hash = "e".repeat(64);
    let exported_content_hash = "f".repeat(64);
    let git_commit = "1".repeat(40);
    let manifest = json!({
        "schema": "ait.release.family/v3",
        "family": {
            "name": "ait-native",
            "version": "1.0.0-rc.1",
            "channel": "rc",
            "tag": "v1.0.0-rc.1"
        },
        "targets": [target],
        "public_source": public_source(&["ait-core"]),
        "components": [{
            "id": "ait",
            "source_repository": "ait-core",
            "source_snapshot": source_snapshot,
            "ecosystem": "native",
            "license": "Apache-2.0",
            "version_scheme": "family",
            "version": "1.0.0-rc.1",
            "artifacts": [{"kind": "native-executable", "targets": [target]}]
        }],
        "distributions": [github_distribution(&["ait"], &[target])],
        "compatibility": {}
    });
    let family_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(public_root.join("ait-release-family.json"), &family_bytes).unwrap();
    let mapping = json!({
        "schema": "ait.release.monorepo-source/v1",
        "public_source_identity": "weita2026/ait-native",
        "coordinator_snapshot": coordinator_snapshot,
        "coordinator_manifest_hash": coordinator_manifest_hash,
        "coordinator_created_at": "1700000000",
        "family_version": "1.0.0-rc.1",
        "family_tag": "v1.0.0-rc.1",
        "family_manifest_sha256": digest(&family_bytes),
        "product_document_sha256": "2".repeat(64),
        "content_digest_contract": "size-sha256-path/v1; excludes ait-monorepo-source.json",
        "content_sha256": mapping_content_hash,
        "subtrees": [{
            "source_repository": "ait-core",
            "source_snapshot": source_snapshot,
            "source_manifest_hash": source_manifest_hash,
            "source_snapshot_created_at": "1699999999",
            "path": "ait-core",
            "license": "Apache-2.0",
            "components": ["ait"],
            "transforms": [],
            "source_cache_evidence_sha256": "3".repeat(64),
            "source_content_sha256": "4".repeat(64),
            "exported_content_sha256": exported_content_hash
        }],
        "excluded_operational_roots": [".ait", ".git"],
        "git_commit_created": false,
        "public_publish": false
    });
    let mapping_path = public_root.join("ait-monorepo-source.json");
    fs::write(&mapping_path, serde_json::to_vec_pretty(&mapping).unwrap()).unwrap();
    let mapping_sha256 = digest(&fs::read(&mapping_path).unwrap());

    let wrong_profile = run(
        &admission_root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--profile",
            "generic-command",
            "--public-source-root",
            public_root.to_str().unwrap(),
        ],
    );
    assert!(!wrong_profile.status.success());
    assert!(String::from_utf8_lossy(&wrong_profile.stderr).contains("requires --profile family"));

    let candidate = run_json(
        &admission_root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--profile",
            "family",
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(candidate["snapshot_id"], coordinator_snapshot);
    assert_eq!(candidate["manifest_hash"], coordinator_manifest_hash);
    assert_eq!(candidate["created_at"], "1700000000");
    assert_eq!(candidate["authority"]["source"], "selected_snapshot");
    let release_id = candidate["release_id"].as_str().unwrap().to_string();

    let adjacent_candidate = run_json(
        &core_root,
        &[
            "release",
            "candidate",
            "create",
            "--version",
            "1.0.0-rc.1",
            "--channel",
            "rc",
            "--profile",
            "family",
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(adjacent_candidate, candidate);

    let receipts = admission_root.join("component-receipts");
    write_component_receipts_for_targets(
        &receipts,
        "ait",
        "ait-core",
        source_snapshot,
        "native",
        "1.0.0-rc.1",
        "native-executable",
        &[target],
    );
    let receipt_path = receipts
        .join("ait")
        .join(target)
        .join("ait-release.receipt.json");
    let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    receipt["contract"] = json!("ait.release.public-git.receipt/v1");
    receipt["manifest_hash"] = json!(source_manifest_hash);
    receipt["created_at"] = json!("1699999999");
    receipt["updated_at"] = json!("1699999999");
    receipt["metadata"]["source_snapshot_created_at"] = json!("1699999999");
    receipt["metadata"]["build"]["builder"] = json!("ait_public_git_adapter_v1");
    receipt["metadata"]["build"]["built_at"] = json!("1699999999");
    receipt["metadata"]["build"]["source_date_epoch"] = json!("1699999999");
    receipt["authority"] = json!({
        "source": "public_git_commit",
        "public_source_identity": "weita2026/ait-native",
        "git_commit": git_commit,
        "coordinator_snapshot": coordinator_snapshot,
        "source_snapshot": source_snapshot,
        "source_manifest_hash": source_manifest_hash,
        "source_mapping_path": "ait-monorepo-source.json",
        "source_mapping_sha256": mapping_sha256,
        "source_content_sha256": mapping_content_hash,
        "subtree_path": "ait-core",
        "subtree_exported_content_sha256": exported_content_hash,
        "persistence": "ci_artifact_bundle",
        "local_release_authority": "not_activated",
        "remote_publish_supported": false
    });
    receipt["public_publish"] = json!(false);
    receipt["publishable"] = json!(false);
    fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();

    let receipt_arg = receipts.to_str().unwrap();
    let missing_explicit_authority = run(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert!(!missing_explicit_authority.status.success());

    let checked = run_json(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(checked["check_summary"]["decision"], "pass");
    assert_eq!(checked["check_summary"]["total"], 7);
    assert_eq!(checked["component_receipts"][0]["git_commit"], git_commit);

    let adjacent_checked = run_json(
        &core_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--json",
        ],
    );
    assert_eq!(adjacent_checked, checked);

    let mut tampered = receipt.clone();
    tampered["authority"]["git_commit"] = json!("2".repeat(39));
    fs::write(&receipt_path, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
    let rejected = run(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("differs from ait-monorepo-source.json")
    );

    let mut wrong_epoch = receipt.clone();
    wrong_epoch["metadata"]["build"]["source_date_epoch"] = json!("1700000000");
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&wrong_epoch).unwrap(),
    )
    .unwrap();
    let rejected = run(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("differs from ait-monorepo-source.json")
    );

    let mut mixed = receipt.clone();
    mixed["contract"] = json!("ait.release.adapter.receipt/v1");
    mixed["metadata"]["build"]["builder"] = json!("ait_generic_command_adapter_v1");
    fs::write(&receipt_path, serde_json::to_vec_pretty(&mixed).unwrap()).unwrap();
    let rejected = run(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .contains("requires public-Git component receipts"));

    let mut mapping_drift = mapping.clone();
    mapping_drift["content_sha256"] = json!("9".repeat(64));
    fs::write(
        &mapping_path,
        serde_json::to_vec_pretty(&mapping_drift).unwrap(),
    )
    .unwrap();
    fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    let rejected = run(
        &admission_root,
        &[
            "release",
            "check",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("differs from ait-monorepo-source.json")
    );

    fs::write(&mapping_path, serde_json::to_vec_pretty(&mapping).unwrap()).unwrap();
    fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    let built = run_json(
        &admission_root,
        &[
            "release",
            "build",
            &release_id,
            "--receipts",
            receipt_arg,
            "--public-source-root",
            public_root.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(built["status"], "built");
    assert_eq!(
        built["component_receipts"][0]["contract"],
        "ait.release.public-git.receipt/v1"
    );

    let restored_root = temp.path().join("restored-family-admission");
    fs::create_dir(&restored_root).unwrap();
    initialize_internal_repo(&restored_root, "ait-core", "release-bootstrap");
    copy_tree(
        &admission_root.join("dist").join(&release_id),
        &restored_root.join("dist").join(&release_id),
    );

    let missing_show_authority = run(&restored_root, &["release", "show", &release_id, "--json"]);
    assert!(!missing_show_authority.status.success());
    assert!(String::from_utf8_lossy(&missing_show_authority.stderr).contains("Unknown line: main"));

    let missing_promote_authority = run(
        &restored_root,
        &[
            "release",
            "promote",
            &release_id,
            "--channel",
            "rc",
            "--json",
        ],
    );
    assert!(!missing_promote_authority.status.success());
    assert!(
        String::from_utf8_lossy(&missing_promote_authority.stderr).contains("Unknown line: main")
    );

    let missing_package_authority = run(
        &restored_root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "homebrew",
            "--json",
        ],
    );
    assert!(!missing_package_authority.status.success());
    assert!(
        String::from_utf8_lossy(&missing_package_authority.stderr).contains("Unknown line: main")
    );
    assert!(!restored_root
        .join("dist")
        .join(&release_id)
        .join("ait-release.promotion.json")
        .exists());
    assert!(!restored_root
        .join("dist")
        .join(&release_id)
        .join("packages")
        .exists());

    let public_root_arg = public_root.to_str().unwrap();
    let shown = run_json(
        &restored_root,
        &[
            "release",
            "show",
            &release_id,
            "--public-source-root",
            public_root_arg,
            "--json",
        ],
    );
    assert_eq!(shown, built);

    let package_reaches_declared_channel_validation = run(
        &restored_root,
        &[
            "release",
            "package",
            &release_id,
            "--channel",
            "homebrew",
            "--public-source-root",
            public_root_arg,
            "--json",
        ],
    );
    assert!(!package_reaches_declared_channel_validation.status.success());
    let package_error =
        String::from_utf8_lossy(&package_reaches_declared_channel_validation.stderr);
    assert!(
        package_error.contains("Frozen family does not declare a homebrew distribution"),
        "unexpected package error: {package_error}"
    );
    assert!(!package_error.contains("Unknown line: main"));

    let promoted = run_json(
        &restored_root,
        &[
            "release",
            "promote",
            &release_id,
            "--channel",
            "rc",
            "--public-source-root",
            public_root_arg,
            "--json",
        ],
    );
    assert_eq!(promoted["status"], "ready_for_protected_ci");
    assert_eq!(promoted["authorization"]["granted"], false);
    assert_eq!(promoted["mutation"]["credentials_loaded"], false);
    assert_eq!(promoted["mutation"]["registry_write"], false);
    assert_eq!(
        run_json(
            &restored_root,
            &[
                "release",
                "show",
                &release_id,
                "--public-source-root",
                public_root_arg,
                "--json",
            ],
        ),
        promoted
    );

    let mut post_build_mapping_drift = mapping.clone();
    post_build_mapping_drift["coordinator_manifest_hash"] = json!("9".repeat(64));
    fs::write(
        &mapping_path,
        serde_json::to_vec_pretty(&post_build_mapping_drift).unwrap(),
    )
    .unwrap();
    let drifted_show = run(
        &restored_root,
        &[
            "release",
            "show",
            &release_id,
            "--public-source-root",
            public_root_arg,
            "--json",
        ],
    );
    assert!(!drifted_show.status.success());
    assert!(String::from_utf8_lossy(&drifted_show.stderr)
        .contains("does not match its immutable Snapshot manifest"));
    fs::write(&mapping_path, serde_json::to_vec_pretty(&mapping).unwrap()).unwrap();
}
