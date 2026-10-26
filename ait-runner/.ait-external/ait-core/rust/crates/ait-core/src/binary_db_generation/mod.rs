//! Current Binary DB generation capture, validation, and activation.
//!
//! Every operation is repository-scoped and fail-closed. A generation is
//! activated only after its manifest, checksums, schema, and content closure
//! have been validated.

mod generation_activation;
mod generation_capture;
mod generation_content_closure;
mod generation_content_indexes;
mod generation_manifest;
mod u64_second_upgrade;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

pub use generation_activation::{
    activate_binary_db_generation, admit_activated_binary_db_generation,
    admit_activated_binary_db_generation_for_runtime, binary_db_activation_lock_root,
    snapshot_binary_db_authority_fingerprint, ActivatedBinaryDbGeneration,
    BinaryDbGenerationActivationOptions, BinaryDbGenerationActivationReport,
};
pub use generation_capture::{
    capture_binary_db_generation, CaptureBinaryDbGenerationOptions, CaptureBinaryDbGenerationReport,
};
pub use generation_manifest::GenerationFileManifest;
pub use u64_second_upgrade::{
    stage_binary_db_u64_second_upgrade, StageBinaryDbU64SecondUpgradeOptions,
    StageBinaryDbU64SecondUpgradeReport, U32_TIME_V0_SOURCE_SELECTOR,
    U64_SECOND_V0_TARGET_SELECTOR,
};

pub type GenerationResult<T> = Result<T, String>;

fn deterministic_parallel_map<T, R, F>(
    items: &[T],
    jobs: usize,
    operation: F,
) -> GenerationResult<Vec<R>>
where
    T: Sync,
    R: Send + 'static,
    F: Fn(usize, &T) -> GenerationResult<R> + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = jobs.min(items.len()).max(1);
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, GenerationResult<R>)>();
    thread::scope(|scope| -> GenerationResult<()> {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next = &next;
            let operation = &operation;
            handles.push(scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(index) else {
                    break;
                };
                if sender.send((index, operation(index, item))).is_err() {
                    break;
                }
            }));
        }
        drop(sender);
        for handle in handles {
            if handle.join().is_err() {
                return Err("generation worker panicked".to_string());
            }
        }
        Ok(())
    })?;

    let mut ordered = BTreeMap::new();
    for (index, result) in receiver {
        if ordered.insert(index, result).is_some() {
            return Err(format!(
                "generation worker returned duplicate index {index}"
            ));
        }
    }
    if ordered.len() != items.len() {
        return Err(format!(
            "generation workers returned {} results for {} inputs",
            ordered.len(),
            items.len()
        ));
    }
    ordered.into_values().collect()
}
