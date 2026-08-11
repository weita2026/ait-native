use std::path::{Component, Path};

const GLOBAL_REBUILD_TEMPORARIES: [&str; 2] = [
    ".repository.bin.rewrite",
    ".repository_namespace.idx.rebuild",
];
const REPOSITORY_REBUILD_TEMPORARIES: [&str; 3] = [
    ".worker_job.bin.rewrite",
    ".worker_ready.idx.rebuild",
    ".worker_state.idx.rebuild",
];

pub(crate) fn is_disposable_runtime_rebuild(relative: &Path) -> bool {
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>();
    let Some(components) = components else {
        return false;
    };

    match components.as_slice() {
        ["global", file] => GLOBAL_REBUILD_TEMPORARIES.contains(file),
        ["repositories", repository_index, file] => {
            is_canonical_repository_index(repository_index)
                && REPOSITORY_REBUILD_TEMPORARIES.contains(file)
        }
        _ => false,
    }
}

pub(crate) fn is_disposable_runtime_file(relative: &Path) -> bool {
    relative
        .to_str()
        .is_some_and(|value| value.ends_with(".lock"))
        || is_disposable_runtime_rebuild(relative)
}

fn is_canonical_repository_index(value: &str) -> bool {
    value
        .parse::<u32>()
        .is_ok_and(|index| index.to_string() == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_rebuild_paths_are_exact_and_repository_scoped() {
        for path in [
            "global/.repository.bin.rewrite",
            "global/.repository_namespace.idx.rebuild",
            "repositories/0/.worker_job.bin.rewrite",
            "repositories/2/.worker_ready.idx.rebuild",
            "repositories/4294967295/.worker_state.idx.rebuild",
        ] {
            assert!(is_disposable_runtime_rebuild(Path::new(path)), "{path}");
        }

        for path in [
            ".worker_ready.idx.rebuild",
            "repositories/02/.worker_ready.idx.rebuild",
            "repositories/4294967296/.worker_state.idx.rebuild",
            "repositories/2/nested/.worker_ready.idx.rebuild",
            "repositories/2/worker_ready.idx",
            "other/2/.worker_ready.idx.rebuild",
            "global/nested/.repository_namespace.idx.rebuild",
            "global/.unrecognized.idx.rebuild",
        ] {
            assert!(!is_disposable_runtime_rebuild(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn runtime_lock_paths_are_disposable_without_loosening_rebuild_paths() {
        for path in [
            ".lock",
            ".locks/binary-db/server-content.write.lock",
            "repositories/2/worker-queue.lock",
        ] {
            assert!(is_disposable_runtime_file(Path::new(path)), "{path}");
        }

        for path in [
            "repositories/2/worker-queue.lock.extra",
            "repositories/2/.worker_ready.idx.rebuild.extra",
            "global/.unrecognized.idx.rebuild",
        ] {
            assert!(!is_disposable_runtime_file(Path::new(path)), "{path}");
        }
    }
}
