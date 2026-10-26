use std::path::Path;

use ait_core::file_io::{FileIoByteStore, FileIoStore, FilesystemFileIoStore};
use ait_core::json_support::{json, JsonValue};

const ATTACHMENT_PUBLISH_LABEL: &str = "ait-agent attachment";
pub const AGENT_FILE_STORE_CONTRACT: &str = "ait.agent.file_store.v1";

pub fn agent_file_store_read_bytes_json(
    request: &JsonValue,
) -> Result<(JsonValue, Vec<u8>), String> {
    agent_file_store_read_bytes_with_store(&FilesystemFileIoStore, request)
}

pub fn agent_file_store_read_bytes_with_store<S>(
    file_io: &S,
    request: &JsonValue,
) -> Result<(JsonValue, Vec<u8>), String>
where
    S: FileIoStore,
{
    let request = request
        .as_object()
        .ok_or_else(|| "ait-agent file store read request must be an object".to_string())?;
    let path = required_text(request.get("path"), "path")?;
    let payload = file_io.read_bytes(Path::new(&path)).map_err(|error| {
        format!(
            "failed to read ait-agent file '{}': {error}",
            Path::new(&path).display()
        )
    })?;
    let metadata = json!({
        "contract": AGENT_FILE_STORE_CONTRACT,
        "operation": "read",
        "path": path,
        "result": {
            "byte_count": payload.len(),
        },
        "python_file_read_allowed": false,
        "python_file_mutation_allowed": false,
    });
    Ok((metadata, payload))
}

pub fn agent_file_store_execute_json(
    request: &JsonValue,
    payload: Option<&[u8]>,
) -> Result<JsonValue, String> {
    agent_file_store_execute_with_store(&FilesystemFileIoStore, request, payload)
}

pub fn agent_file_store_execute_with_store<S>(
    file_io: &S,
    request: &JsonValue,
    payload: Option<&[u8]>,
) -> Result<JsonValue, String>
where
    S: FileIoByteStore,
{
    let request = request
        .as_object()
        .ok_or_else(|| "ait-agent file store request must be an object".to_string())?;
    let path = required_text(request.get("path"), "path")?;
    let operation = required_text(request.get("operation"), "operation")?;
    let result = match operation.as_str() {
        "inspect" => {
            if payload.is_some() {
                return Err(
                    "ait-agent file store inspect operation does not accept payload".to_string(),
                );
            }
            json!({"exists": file_io.path_exists(Path::new(&path))})
        }
        "publish" => {
            let payload = payload.ok_or_else(|| {
                "ait-agent file store publish operation requires payload".to_string()
            })?;
            file_io
                .write_bytes_atomically(Path::new(&path), payload, ATTACHMENT_PUBLISH_LABEL)
                .map_err(|error| {
                    format!(
                        "failed to publish ait-agent attachment '{}': {error}",
                        Path::new(&path).display()
                    )
                })?;
            json!({
                "published": true,
                "byte_count": payload.len(),
            })
        }
        other => {
            return Err(format!(
                "unsupported ait-agent file store operation '{other}'"
            ))
        }
    };
    Ok(json!({
        "contract": AGENT_FILE_STORE_CONTRACT,
        "operation": operation,
        "path": path,
        "result": result,
        "python_file_read_allowed": false,
        "python_file_mutation_allowed": false,
    }))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    match value {
        Some(JsonValue::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(JsonValue::String(_)) | None | Some(JsonValue::Null) => {
            Err(format!("ait-agent file store request requires {field}"))
        }
        Some(_) => Err(format!(
            "ait-agent file store request field '{field}' must be a string"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use ait_core::file_io::{FileIoByteStore, FileIoResult, FileIoStore, FilesystemFileIoStore};
    use ait_core::json_support::json;
    use tempfile::tempdir;

    use super::*;

    #[derive(Default)]
    struct RecordingFileIoStore {
        exists: bool,
        read_payload: Vec<u8>,
        read_paths: RefCell<Vec<PathBuf>>,
        atomic_writes: RefCell<Vec<(PathBuf, Vec<u8>, String)>>,
    }

    impl FileIoStore for RecordingFileIoStore {
        fn home_dir(&self) -> Option<PathBuf> {
            None
        }

        fn path_exists(&self, _path: &Path) -> bool {
            self.exists
        }

        fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
            self.read_paths.borrow_mut().push(path.to_path_buf());
            Ok(self.read_payload.clone())
        }

        fn read_to_string(&self, _path: &Path) -> FileIoResult<String> {
            Ok(String::new())
        }

        fn write_string(&self, _path: &Path, _text: &str) -> FileIoResult<()> {
            Ok(())
        }

        fn write_string_atomically(
            &self,
            _path: &Path,
            _text: &str,
            _publish_label: &str,
        ) -> FileIoResult<()> {
            Ok(())
        }
    }

    #[test]
    fn read_delegates_raw_bytes_to_file_io_port() {
        let store = RecordingFileIoStore {
            read_payload: vec![0, 255, 1],
            ..RecordingFileIoStore::default()
        };

        let (metadata, payload) = agent_file_store_read_bytes_with_store(
            &store,
            &json!({"path": "/cache/attachment.bin"}),
        )
        .expect("read response");

        assert_eq!(metadata["contract"], AGENT_FILE_STORE_CONTRACT);
        assert_eq!(metadata["operation"], "read");
        assert_eq!(metadata["path"], "/cache/attachment.bin");
        assert_eq!(metadata["result"], json!({"byte_count": 3}));
        assert_eq!(metadata["python_file_read_allowed"], false);
        assert_eq!(metadata["python_file_mutation_allowed"], false);
        assert_eq!(payload, [0, 255, 1]);
        assert_eq!(
            store.read_paths.borrow().as_slice(),
            &[PathBuf::from("/cache/attachment.bin")]
        );
    }

    #[test]
    fn read_preserves_an_empty_payload() {
        let store = RecordingFileIoStore::default();

        let (metadata, payload) =
            agent_file_store_read_bytes_with_store(&store, &json!({"path": "/cache/empty.bin"}))
                .expect("empty read response");

        assert_eq!(metadata["result"], json!({"byte_count": 0}));
        assert!(payload.is_empty());
    }

    impl FileIoByteStore for RecordingFileIoStore {
        fn write_bytes_atomically(
            &self,
            path: &Path,
            bytes: &[u8],
            publish_label: &str,
        ) -> FileIoResult<()> {
            self.atomic_writes.borrow_mut().push((
                path.to_path_buf(),
                bytes.to_vec(),
                publish_label.to_string(),
            ));
            Ok(())
        }
    }

    #[test]
    fn inspect_delegates_path_state_to_file_io_port() {
        let store = RecordingFileIoStore {
            exists: true,
            ..RecordingFileIoStore::default()
        };

        let response = agent_file_store_execute_with_store(
            &store,
            &json!({"path": "/cache/attachment.bin", "operation": "inspect"}),
            None,
        )
        .expect("inspect response");

        assert_eq!(response["contract"], AGENT_FILE_STORE_CONTRACT);
        assert_eq!(response["operation"], "inspect");
        assert_eq!(response["path"], "/cache/attachment.bin");
        assert_eq!(response["result"], json!({"exists": true}));
        assert_eq!(response["python_file_read_allowed"], false);
        assert_eq!(response["python_file_mutation_allowed"], false);
        assert!(store.atomic_writes.borrow().is_empty());
    }

    #[test]
    fn publish_delegates_raw_bytes_to_atomic_file_io_port() {
        let store = RecordingFileIoStore::default();

        let response = agent_file_store_execute_with_store(
            &store,
            &json!({"path": "/cache/attachment.bin", "operation": "publish"}),
            Some(&[0, 255, 1]),
        )
        .expect("publish response");

        assert_eq!(
            response["result"],
            json!({"published": true, "byte_count": 3})
        );
        assert_eq!(
            store.atomic_writes.borrow().as_slice(),
            &[(
                PathBuf::from("/cache/attachment.bin"),
                vec![0, 255, 1],
                ATTACHMENT_PUBLISH_LABEL.to_string(),
            )]
        );
    }

    #[test]
    fn file_store_rejects_invalid_requests_and_payload_contracts() {
        let store = RecordingFileIoStore::default();
        for (request, payload, expected) in [
            (json!([]), None, "request must be an object"),
            (
                json!({"operation": "inspect"}),
                None,
                "request requires path",
            ),
            (
                json!({"path": "/cache/file", "operation": "publish"}),
                None,
                "publish operation requires payload",
            ),
            (
                json!({"path": "/cache/file", "operation": "inspect"}),
                Some(&b"unexpected"[..]),
                "inspect operation does not accept payload",
            ),
            (
                json!({"path": "/cache/file", "operation": "remove"}),
                None,
                "unsupported ait-agent file store operation 'remove'",
            ),
        ] {
            let error = agent_file_store_execute_with_store(&store, &request, payload)
                .expect_err("invalid request");
            assert!(
                error.contains(expected),
                "{error:?} did not contain {expected:?}"
            );
        }
    }

    #[test]
    fn filesystem_file_store_inspects_publishes_and_replaces_binary_payloads() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("nested/attachment.bin");
        let request = json!({"path": path, "operation": "inspect"});

        let missing = agent_file_store_execute_json(&request, None).expect("missing inspect");
        assert_eq!(missing["result"]["exists"], false);

        let publish_request = json!({"path": path, "operation": "publish"});
        let published = agent_file_store_execute_json(&publish_request, Some(&[0, 255, 1]))
            .expect("binary publish");
        assert_eq!(published["result"]["byte_count"], 3);
        assert_eq!(fs::read(&path).expect("published bytes"), [0, 255, 1]);

        let existing = agent_file_store_execute_json(&request, None).expect("existing inspect");
        assert_eq!(existing["result"]["exists"], true);

        agent_file_store_execute_json(&publish_request, Some(b""))
            .expect("empty replacement publish");
        assert_eq!(
            FilesystemFileIoStore
                .read_bytes(&path)
                .expect("empty replacement bytes"),
            b""
        );
    }

    #[test]
    fn filesystem_file_store_reads_nested_binary_and_empty_payloads() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("nested/attachment.bin");
        fs::create_dir_all(path.parent().expect("nested parent")).expect("create parent");
        fs::write(&path, [0, 255, 1]).expect("write binary fixture");

        let (metadata, payload) =
            agent_file_store_read_bytes_json(&json!({"path": path})).expect("binary read");
        assert_eq!(metadata["result"]["byte_count"], 3);
        assert_eq!(payload, [0, 255, 1]);

        fs::write(&path, []).expect("write empty fixture");
        let (metadata, payload) =
            agent_file_store_read_bytes_json(&json!({"path": path})).expect("empty read");
        assert_eq!(metadata["result"]["byte_count"], 0);
        assert!(payload.is_empty());
    }

    #[test]
    fn filesystem_file_store_read_propagates_missing_and_directory_errors() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing.bin");
        let missing_error = agent_file_store_read_bytes_json(&json!({"path": missing}))
            .expect_err("missing read should fail");
        assert!(missing_error.contains("failed to read ait-agent file"));

        let directory_error = agent_file_store_read_bytes_json(&json!({"path": temp.path()}))
            .expect_err("directory read should fail");
        assert!(directory_error.contains("failed to read ait-agent file"));
    }
}
