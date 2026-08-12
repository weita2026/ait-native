def ensure($condition; $message):
  if $condition then . else error($message) end;

($family[0]) as $family_manifest |
($platforms[0]) as $platform_contract |
($authorities[0]) as $authority_contract |
($authority_contract.repositories | INDEX(.repo_name)) as $authority_by_repo |
($platform_contract.targets | INDEX(.target)) as $platform_by_target |
($family_manifest.components | group_by(.source_repository)) as $component_groups |
[
  $family_manifest.components[] as $component
  | $component.artifacts[] as $artifact
  | if ($artifact.targets | length) == 0 then
      {
        repo_name: $component.source_repository,
        source_snapshot: $component.source_snapshot,
        version: $component.version,
        component: $component.id,
        ecosystem: $component.ecosystem,
        kind: $artifact.kind,
        target: "portable"
      }
    else
      $artifact.targets[] as $target
      | {
          repo_name: $component.source_repository,
          source_snapshot: $component.source_snapshot,
          version: $component.version,
          component: $component.id,
          ecosystem: $component.ecosystem,
          kind: $artifact.kind,
          target: $target
        }
    end
] as $artifact_rows |
[
  $component_groups[] as $group
  | ($group[0].source_repository) as $repo_name
  | ($authority_by_repo[$repo_name]) as $authority
  | {
      repo_name: $repo_name,
      repository_index: $authority.repository_index,
      namespace: $authority.namespace,
      source_snapshot: $group[0].source_snapshot,
      version: $group[0].version,
      license: $group[0].license,
      line: $authority_contract.source_line,
      bootstrap_line: $authority_contract.bootstrap_line,
      component_ids: ([$group[].id] | sort),
      source_cache_artifact: (
        $authority_contract.source_cache_artifact_prefix + "-" + $repo_name
      )
    }
] | sort_by(.repository_index) as $sources |
[
  ($artifact_rows | group_by([.repo_name, .target]))[] as $group
  | ($group[0]) as $artifact
  | (
      if $artifact.target == "portable"
      then $authority_contract.portable_runner
      else $platform_by_target[$artifact.target]
      end
    ) as $platform
  | ($sources[] | select(.repo_name == $artifact.repo_name)) as $source
  | $source + {
      target: $artifact.target,
      runner: $platform.runner,
      os: $platform.os,
      runner_os: (
        if $platform.os == "macos" then "macOS"
        elif $platform.os == "linux" then "Linux"
        elif $platform.os == "windows" then "Windows"
        else error("release runner operating system is unsupported")
        end
      ),
      architecture: $platform.architecture,
      minimum_platform_kind: $platform.minimum_platform_kind,
      minimum_platform: $platform.minimum_platform,
      executable_suffix: $platform.executable_suffix,
      expected_component_artifact_count: ($group | length),
      expected_components: ([$group[].component] | sort),
      receipt_artifact: (
        $authority_contract.component_receipt_artifact_prefix
        + "-" + $artifact.repo_name + "-" + $artifact.target
      )
    }
] | sort_by([.repository_index, .target]) as $builds |
null
| ensure(
    $family_manifest.schema == "ait.release.family/v3";
    "ait-release-family.json must use ait.release.family/v3"
  )
| ensure(
    $family_manifest.public_source.model == "release-monorepo"
      and $family_manifest.public_source.identity == "weita2026/ait-native"
      and $family_manifest.public_source.product_document == "docs/distribution.md"
      and $family_manifest.public_source.family_manifest == "ait-release-family.json"
      and $family_manifest.public_source.mapping_manifest == "ait-monorepo-source.json"
      and $family_manifest.public_source.build_entrypoints == {
        unix: "build-release.sh",
        windows: "build-release.ps1",
        implementation: "build-release.mjs"
      }
      and ($family_manifest.public_source.subtrees | length) == 5
      and ([ $family_manifest.public_source.subtrees[].source_repository ] | sort)
        == ([ $authority_contract.repositories[].repo_name ] | sort)
      and all(
        $family_manifest.public_source.subtrees[];
        .path == .source_repository
          and if .source_repository == "ait-runner" then
            .transforms == ["runner-core-path/v1"]
          elif .source_repository == "ait-python" then
            .transforms == ["python-core-path/v1"]
          else
            .transforms == []
          end
      )
      and ($family_manifest.public_source.transforms | length) == 2
      and all(
        $family_manifest.public_source.transforms[];
        . == {
          id: "runner-core-path/v1",
          source_repository: "ait-runner",
          path: "Cargo.toml",
          from: ".ait-external/ait-core/rust/crates/ait-core",
          to: "../ait-core/rust/crates/ait-core"
        }
        or . == {
          id: "python-core-path/v1",
          source_repository: "ait-python",
          path: "pyproject.toml",
          from: ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
          to: "../ait-core/rust/crates/ait-py/Cargo.toml"
        }
      )
      and ([ $family_manifest.public_source.transforms[].id ] | unique | length) == 2;
    "family public source monorepo contract is invalid"
  )
| ensure(
    ([ $family_manifest.distributions[] | select(.channel == "github") ] | length) == 1
      and ($family_manifest.distributions[] | select(.channel == "github") |
        .role == "product"
          and .identity == $family_manifest.public_source.identity
          and ([.components[]] | sort) == ([ $family_manifest.components[].id ] | sort)
          and ([.targets[]] | sort) == ($family_manifest.targets | sort));
    "family must declare one complete product GitHub monorepo distribution"
  )
| ensure(
    $platform_contract.contract == "ait-native-bootstrap-matrix/v1"
      and $platform_contract.schema_version == 1;
    "native bootstrap platform contract is unsupported"
  )
| ensure(
    $authority_contract.contract == "ait.release.repository-authorities/v1"
      and $authority_contract.schema_version == 1;
    "release Repository authority contract is unsupported"
  )
| ensure(
    $authority_contract.public_publish == false;
    "release receipt matrix must remain internal-only"
  )
| ensure(
    $family_manifest.family.version == $authority_contract.family_version
      and $platform_contract.version == $authority_contract.family_version;
    "family, authority, and native platform versions differ"
  )
  | ensure(
    ($authority_contract.source_line | type == "string"
      and test("^[A-Za-z0-9._/-]+$"))
      and ($authority_contract.bootstrap_line | type == "string"
        and test("^[A-Za-z0-9._/-]+$"))
      and $authority_contract.source_line == "main"
      and $authority_contract.bootstrap_line != $authority_contract.source_line;
    "source and disposable bootstrap Lines are invalid"
  )
| ensure(
    ($authority_contract.source_cache_artifact_prefix
      | type == "string" and test("^[A-Za-z0-9._-]+$"))
      and ($authority_contract.component_receipt_artifact_prefix
        | type == "string" and test("^[A-Za-z0-9._-]+$"))
      and ($authority_contract.source_cache_retention_days
        | type == "number" and floor == . and . >= 1 and . <= 30)
      and ($authority_contract.receipt_retention_days
        | type == "number" and floor == . and . >= 1 and . <= 90);
    "release artifact names or retention bounds are invalid"
  )
| ensure(
    ($authority_contract.repositories | length) == 5
      and ([ $authority_contract.repositories[].repo_name ] | unique | length) == 5
      and ([ $authority_contract.repositories[].repository_index ] | unique | length) == 5
      and all(
        $authority_contract.repositories[];
        (.repo_name | type == "string" and test("^[a-z0-9-]+$"))
          and (.repository_index | type == "number" and floor == . and . >= 0)
          and (.namespace | type == "string" and test("^[A-Za-z0-9_-]{1,2}$"))
      );
    "release Repository authorities must contain five unique bounded mappings"
  )
| ensure(
    ([ $authority_contract.repositories[].repo_name ] | sort)
      == ([ $component_groups[][0].source_repository ] | sort);
    "family source repositories and numeric Repository authorities differ"
  )
| ensure(
    all(
      $component_groups[];
      ([ .[].source_snapshot ] | unique | length) == 1
        and ([ .[].version ] | unique | length) == 1
        and ([ .[].license ] | unique | length) == 1
    );
    "components from one source repository must share Snapshot, version, and license"
  )
| ensure(
    all(
      $family_manifest.components[];
      (.source_snapshot | test("^SNP-[0-9A-F]{12}$"))
        and (.artifacts | length) > 0
        and all(.artifacts[]; (.targets | type) == "array")
    );
    "family components contain an invalid Snapshot or artifact declaration"
  )
| ensure(
    ($platform_contract.targets | length) == 6
      and ([ $platform_contract.targets[].target ] | unique | length) == 6
      and ([ $platform_contract.targets[].target ] | sort)
        == ($family_manifest.targets | sort);
    "family and native platform target sets differ"
  )
  | ensure(
    $authority_contract.portable_runner.target == "portable"
      and ($authority_contract.portable_runner.runner | type) == "string"
      and ($authority_contract.portable_runner.runner | length) > 0
      and ($authority_contract.portable_runner.os == "macos"
        or $authority_contract.portable_runner.os == "linux"
        or $authority_contract.portable_runner.os == "windows")
      and ($authority_contract.portable_runner.architecture == "arm64"
        or $authority_contract.portable_runner.architecture == "x86_64")
      and $authority_contract.portable_runner.minimum_platform_kind
        == "portable_envelope"
      and ($authority_contract.portable_runner.minimum_platform
        | type == "string" and length > 0)
      and ($authority_contract.portable_runner.executable_suffix == ""
        or $authority_contract.portable_runner.executable_suffix == ".exe");
    "portable artifact runner mapping is invalid"
  )
| ensure(
    all(
      $artifact_rows[];
      .target == "portable" or ($platform_by_target[.target] != null)
    );
    "family artifact selects an undeclared target"
  )
| ensure(
    ([ $artifact_rows[] | [.repo_name, .component, .kind, .target] ] | unique | length)
      == ($artifact_rows | length);
    "family component artifact keys are duplicated"
  )
| ensure(
    ($sources | length) == 5
      and ($builds | length) == 31
      and ($artifact_rows | length) == 37
      and ([ $builds[].expected_component_artifact_count ] | add) == 37;
    "release matrix must resolve to five sources, 31 receipts, and 37 component artifacts"
  )
| ensure(
    ([ $builds[] | select(.target == "portable") ] | length) == 1
      and ($builds[] | select(.target == "portable") | .repo_name) == "ait-node"
      and ([ $builds[] | select(.repo_name == "ait-node") ] | length) == 7;
    "ait-node must select one portable envelope and six target addon receipts"
  )
| {
    contract: "ait.release.receipt-matrix/v1",
    family: $family_manifest.family,
    bootstrap_line: $authority_contract.bootstrap_line,
    source_line: $authority_contract.source_line,
    public_publish: false,
    expected_source_count: ($sources | length),
    expected_receipt_count: ($builds | length),
    expected_component_artifact_count: ($artifact_rows | length),
    source_cache_retention_days: $authority_contract.source_cache_retention_days,
    receipt_retention_days: $authority_contract.receipt_retention_days,
    bootstrap: {
      include: [
        $platform_contract.targets[]
        | . + {artifact_name: ("ait-release-bootstrap-" + .target)}
      ]
    },
    sources: {include: $sources},
    builds: {include: $builds}
  }
