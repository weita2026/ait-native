.contract == "ait-native-bootstrap-matrix/v1" and
.schema_version == 1 and
.matrix_revision == "six-target-2026-07-19.1" and
.version == "1.0.0-rc.3" and
.rust_toolchain == "1.96.0" and
.cargo_profile == "release" and
.package == "ait-cli" and
.binary == "ait-cli" and
.public_identity == "ait" and
.artifact_prefix == "ait-core-native-cli" and
.public_publish == false and
(.targets | length == 6) and
(([.targets[].target] | length) == ([.targets[].target] | unique | length)) and
all(.targets[];
  (.target | type == "string" and length > 0) and
  (.runner | type == "string" and length > 0) and
  (.os == "macos" or .os == "linux" or .os == "windows") and
  (.architecture == "arm64" or .architecture == "x86_64") and
  (.minimum_platform_kind | type == "string" and length > 0) and
  (.minimum_platform | type == "string" and length > 0) and
  (.executable_suffix == "" or .executable_suffix == ".exe")
)
