use crate::developer_console_save::{FolderSaveJournal, FolderSaveMutation};
use crate::{persist_live_console_save_group, persist_live_console_save_group_timed};
use anyhow::{Context, Result};
use clonk_resources::MutableGroup;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
use std::time::Duration;

// A prepared save owns landscape and group snapshots, so keep the pending
// memory budget small while still allowing deliberate repeated-slot saves.
const SAVE_WORKER_QUEUE_CAPACITY: usize = 4;

pub(crate) struct PreparedNativeSave {
    pub(crate) source_group: PreparedNativeSaveSource,
    pub(crate) destination: PathBuf,
    pub(crate) preserve_folder_group: bool,
    pub(crate) folder_journal: FolderSaveJournal,
    pub(crate) maker: Vec<u8>,
    pub(crate) parameters: Option<Vec<u8>>,
    pub(crate) live_capture: Option<PreparedLiveNativeCapture>,
    pub(crate) timings: NativeSaveTimings,
}

pub(crate) enum PreparedNativeSaveSource {
    /// Packed groups retain an immutable decompressed backing allocation, so
    /// their writable rewrite can be built safely after the app advances.
    Opened(clonk_resources::Group),
    /// Directory groups read file bytes lazily. Freeze those on the caller so
    /// an external edit cannot change an already accepted save generation.
    Materialized(MutableGroup),
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NativeSaveTimings {
    pub(crate) source_group_copy: std::time::Duration,
    pub(crate) live_state_capture: std::time::Duration,
    pub(crate) embedded_player_capture: std::time::Duration,
    pub(crate) live_state_encode: std::time::Duration,
    pub(crate) group_mutation: std::time::Duration,
    pub(crate) pack_compress: std::time::Duration,
    pub(crate) physical_publish: std::time::Duration,
}

pub(crate) struct PreparedLiveNativeCapture {
    pub(crate) capture: clonk_engine::LiveC4SaveCapture,
    pub(crate) policy: OwnedLiveC4SavePolicy,
    pub(crate) landscape_is_static: bool,
    pub(crate) save_player_infos: Option<Vec<u8>>,
    pub(crate) player_groups: Vec<crate::runtime_join_save::SerializedRuntimeJoinPlayerGroup>,
    pub(crate) description: Option<(Vec<u8>, Vec<u8>)>,
    pub(crate) title_components: Vec<(&'static str, Vec<u8>)>,
    pub(crate) component_mutations: Vec<FolderSaveMutation>,
}

pub(crate) enum OwnedLiveC4SavePolicy {
    Scenario { force_exact_landscape: bool },
    Savegame { target_group_name: String },
}

impl OwnedLiveC4SavePolicy {
    pub(crate) fn from_policy(policy: clonk_engine::LiveC4SavePolicy<'_>) -> Self {
        match policy {
            clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape,
            } => Self::Scenario {
                force_exact_landscape,
            },
            clonk_engine::LiveC4SavePolicy::Savegame { target_group_name } => Self::Savegame {
                target_group_name: target_group_name.to_owned(),
            },
            clonk_engine::LiveC4SavePolicy::Record
            | clonk_engine::LiveC4SavePolicy::RuntimeNetwork => {
                unreachable!("native slot preparation uses scenario/savegame policy")
            }
        }
    }

    fn as_policy(&self) -> clonk_engine::LiveC4SavePolicy<'_> {
        match self {
            Self::Scenario {
                force_exact_landscape,
            } => clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape: *force_exact_landscape,
            },
            Self::Savegame { target_group_name } => {
                clonk_engine::LiveC4SavePolicy::Savegame { target_group_name }
            }
        }
    }
}

pub(crate) struct PersistedNativeSave {
    pub(crate) packed_group: Option<Vec<u8>>,
    pub(crate) thumbnail_retention_error: Option<String>,
    pub(crate) timings: NativeSaveTimings,
}

impl PreparedNativeSave {
    fn persist(mut self, retain_packed_group: bool) -> Result<PersistedNativeSave> {
        let source_group_copy_started = std::time::Instant::now();
        let mut group = match self.source_group {
            PreparedNativeSaveSource::Opened(source_group) => {
                let group = MutableGroup::from_group(&source_group).with_context(|| {
                    format!("copy source scenario {}", source_group.root().display())
                })?;
                self.timings.source_group_copy = source_group_copy_started.elapsed();
                group
            }
            PreparedNativeSaveSource::Materialized(group) => group,
        };
        if !self.maker.is_empty() {
            group.set_maker_bytes(&self.maker);
        }
        if let Some(parameters) = self.parameters.take() {
            group
                .add_file("Parameters.txt", parameters)
                .context("write live save Parameters.txt")?;
        }
        if let Some(capture) = self.live_capture.take() {
            let (live_state_encode, group_mutation) = capture.apply(
                &mut group,
                &mut self.folder_journal,
                &self.destination,
                self.preserve_folder_group,
                &self.maker,
            )?;
            self.timings.live_state_encode = live_state_encode;
            self.timings.group_mutation = group_mutation;
        }
        let persistence = persist_live_console_save_group_timed(
            &group,
            &self.destination,
            self.preserve_folder_group,
            &self.folder_journal,
            &self.maker,
        )?;
        self.timings.pack_compress = persistence.pack_compress;
        self.timings.physical_publish = persistence.physical_publish;
        let (packed_group, thumbnail_retention_error) = if retain_packed_group {
            match fs::read(&self.destination).with_context(|| {
                format!(
                    "retain native save generation {}",
                    self.destination.display()
                )
            }) {
                Ok(packed) => (Some(packed), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            }
        } else {
            (None, None)
        };
        Ok(PersistedNativeSave {
            packed_group,
            thumbnail_retention_error,
            timings: self.timings,
        })
    }
}

impl PreparedLiveNativeCapture {
    fn apply(
        self,
        group: &mut MutableGroup,
        journal: &mut FolderSaveJournal,
        destination: &std::path::Path,
        preserve_folder_group: bool,
        maker: &[u8],
    ) -> Result<(std::time::Duration, std::time::Duration)> {
        let policy = self.policy.as_policy();
        let encode_started = std::time::Instant::now();
        let save = match self.capture.encode() {
            Ok(save) => save,
            Err(error) => {
                if let Some(partial) = error.pre_landscape_components() {
                    let apply_result =
                        crate::developer_console_save::apply_live_save_pre_landscape_to_group_recorded(
                            group, policy, partial, journal,
                        );
                    if !maker.is_empty() {
                        group.set_maker_bytes_recursively(maker);
                    }
                    let persist_result = persist_live_console_save_group(
                        group,
                        destination,
                        preserve_folder_group,
                        journal,
                        maker,
                    );
                    apply_result?;
                    persist_result?;
                }
                return Err(error).context("serialize live C4 scenario state");
            }
        };
        let live_state_encode = encode_started.elapsed();
        let mutation_started = std::time::Instant::now();

        crate::developer_console_save::apply_live_save_runtime_components_to_group_recorded(
            group,
            policy,
            &save,
            self.landscape_is_static,
            journal,
        )?;
        group.remove_entry("SavePlayerInfos.txt");
        journal.delete_entry("SavePlayerInfos.txt");
        if let Some(save_player_infos) = self.save_player_infos {
            crate::developer_console_save::add_live_save_player_infos_after_delete_to_group_recorded(
                group,
                &save_player_infos,
                journal,
            )?;
        }
        for player_group in self.player_groups {
            crate::developer_console_save::add_live_save_player_group_recorded(
                group,
                player_group,
                journal,
            )?;
        }
        if let Some((name, payload)) = self.description {
            journal.put_file(
                &name,
                &payload,
                crate::developer_console_save::FolderSaveAddFailure::Ignore,
            );
            if let Err(error) = group.add_file_bytes(name, payload) {
                tracing::warn!(%error, "failed to write live save description");
            }
        }
        for (name, payload) in self.title_components {
            journal.put_file(
                name,
                &payload,
                crate::developer_console_save::FolderSaveAddFailure::Ignore,
            );
            if let Err(error) = group.add_file(name, payload) {
                tracing::warn!(%error, component = name, "failed to write live save title");
            }
        }
        for mutation in self.component_mutations {
            match mutation {
                FolderSaveMutation::DeleteEntry { name } => {
                    let name = String::from_utf8_lossy(&name).into_owned();
                    journal.delete_entry(&name);
                    group.remove_entry(&name);
                }
                FolderSaveMutation::PutFile { name, payload, .. } => {
                    let name = String::from_utf8_lossy(&name).into_owned();
                    journal.put_file(
                        &name,
                        &payload,
                        crate::developer_console_save::FolderSaveAddFailure::Fatal,
                    );
                    group
                        .add_file_bytes(name.as_str(), payload)
                        .with_context(|| format!("write edited component {name}"))?;
                }
                FolderSaveMutation::DeletePattern { .. }
                | FolderSaveMutation::PutChild { .. }
                | FolderSaveMutation::MergeMaterialGroup { .. } => {
                    unreachable!("component hosts only delete or write files")
                }
            }
        }
        if !maker.is_empty() {
            group.set_maker_bytes_recursively(maker);
        }
        Ok((live_state_encode, mutation_started.elapsed()))
    }
}

pub(crate) struct NativeSlotSaveRequest {
    pub(crate) slot: u8,
    pub(crate) status_label: String,
    pub(crate) request_gpu_thumbnail: bool,
    pub(crate) prepared: PreparedNativeSave,
}

pub(crate) struct NativeSlotSaveCompletion {
    pub(crate) slot: u8,
    pub(crate) status_label: String,
    pub(crate) path: PathBuf,
    pub(crate) result: Result<PersistedNativeSave>,
}

pub(crate) struct PreparedPlayerFileSave {
    pub(crate) player_number: i32,
    pub(crate) info_id: i32,
    pub(crate) group: MutableGroup,
    pub(crate) path: PathBuf,
    pub(crate) preserve_folder_group: bool,
    pub(crate) official_derivation: bool,
    pub(crate) derivation: Option<(
        clonk_network::ResourceDerivation,
        clonk_network::ResourceFileOwnership,
    )>,
}

pub(crate) struct PlayerFileSaveCompletion {
    pub(crate) player_number: i32,
    pub(crate) info_id: i32,
    pub(crate) path: PathBuf,
    pub(crate) official_derivation: bool,
    pub(crate) derivation: Option<(
        clonk_network::ResourceDerivation,
        clonk_network::ResourceFileOwnership,
    )>,
    pub(crate) result: Result<()>,
    pub(crate) persistence: std::time::Duration,
}

pub(crate) struct RuntimeDynamicSaveRequest {
    pub(crate) generation: u64,
    pub(crate) synchronized_control_tick: clonk_network::Tick,
    pub(crate) dynamic_tick: i32,
    pub(crate) parameters: clonk_network::JoinGameParametersEnvelope,
    pub(crate) group_filename: String,
    pub(crate) maker: Vec<u8>,
    pub(crate) parameter_bytes: Vec<u8>,
    pub(crate) save: clonk_engine::LiveC4SaveCapture,
    pub(crate) restore_infos: clonk_network::PlayerInfoListSnapshot,
    pub(crate) player_groups: Vec<crate::runtime_join_save::SerializedRuntimeJoinPlayerGroup>,
}

pub(crate) struct RuntimeDynamicSaveCompletion {
    pub(crate) generation: u64,
    pub(crate) synchronized_control_tick: clonk_network::Tick,
    pub(crate) dynamic_tick: i32,
    pub(crate) parameters: clonk_network::JoinGameParametersEnvelope,
    pub(crate) result: Result<clonk_network::LiveNetworkDynamic>,
}

pub(crate) enum BackgroundSaveCompletion {
    NativeSlot(NativeSlotSaveCompletion),
    PlayerFile(PlayerFileSaveCompletion),
    RuntimeDynamic(RuntimeDynamicSaveCompletion),
}

impl NativeSlotSaveRequest {
    fn finish(self) -> BackgroundSaveCompletion {
        let path = self.prepared.destination.clone();
        let result = self.prepared.persist(self.request_gpu_thumbnail);
        BackgroundSaveCompletion::NativeSlot(NativeSlotSaveCompletion {
            slot: self.slot,
            status_label: self.status_label,
            path,
            result,
        })
    }
}

impl PreparedPlayerFileSave {
    fn finish(self) -> BackgroundSaveCompletion {
        let started = std::time::Instant::now();
        let result =
            crate::persist_console_save_group(&self.group, &self.path, self.preserve_folder_group)
                .with_context(|| format!("persist player profile {}", self.path.display()));
        BackgroundSaveCompletion::PlayerFile(PlayerFileSaveCompletion {
            player_number: self.player_number,
            info_id: self.info_id,
            path: self.path,
            official_derivation: self.official_derivation,
            derivation: self.derivation,
            result,
            persistence: started.elapsed(),
        })
    }
}

impl RuntimeDynamicSaveRequest {
    fn finish(self) -> BackgroundSaveCompletion {
        let result = self
            .save
            .encode()
            .context("encode synchronized runtime game")
            .and_then(|save| {
                crate::runtime_join_save::compose_runtime_join_dynamic(
                    self.group_filename,
                    self.maker,
                    self.parameter_bytes,
                    save,
                    &self.restore_infos,
                    self.player_groups,
                )
            });
        BackgroundSaveCompletion::RuntimeDynamic(RuntimeDynamicSaveCompletion {
            generation: self.generation,
            synchronized_control_tick: self.synchronized_control_tick,
            dynamic_tick: self.dynamic_tick,
            parameters: self.parameters,
            result,
        })
    }
}

pub(crate) type BackgroundSaveJob<T> = Box<dyn FnOnce() -> T + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackgroundSaveSubmitError {
    Full,
    Disconnected,
}

/// One bounded worker for CPU- and I/O-heavy save finalization.
///
/// This deliberately does not use Tokio's blocking pool: a save must never
/// compete with the network runtime's transport workers. Dropping the owner
/// closes the request channel and joins the thread, so an accepted save is
/// flushed before application teardown completes.
pub(crate) struct BackgroundSaveWorker<T: Send + 'static> {
    request_tx: Option<SyncSender<BackgroundSaveJob<T>>>,
    result_rx: Receiver<T>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> BackgroundSaveWorker<T> {
    pub(crate) fn new(name: &str, capacity: usize) -> io::Result<Self> {
        let (request_tx, request_rx) = mpsc::sync_channel::<BackgroundSaveJob<T>>(capacity);
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                while let Ok(job) = request_rx.recv() {
                    let result = job();
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            request_tx: Some(request_tx),
            result_rx,
            worker: Some(worker),
        })
    }

    pub(crate) fn try_submit(
        &self,
        job: BackgroundSaveJob<T>,
    ) -> Result<(), BackgroundSaveSubmitError> {
        let Some(request_tx) = self.request_tx.as_ref() else {
            return Err(BackgroundSaveSubmitError::Disconnected);
        };
        request_tx.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => BackgroundSaveSubmitError::Full,
            TrySendError::Disconnected(_) => BackgroundSaveSubmitError::Disconnected,
        })
    }

    pub(crate) fn try_recv(&self) -> Option<T> {
        self.result_rx.try_recv().ok()
    }

    pub(crate) fn finish(&mut self) -> Vec<T> {
        self.request_tx.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::error!("background save worker panicked while flushing");
            }
        }
        self.result_rx.try_iter().collect()
    }

    #[cfg(all(
        test,
        any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
    ))]
    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Option<T> {
        self.result_rx.recv_timeout(timeout).ok()
    }
}

pub(crate) fn new_app_save_worker() -> io::Result<BackgroundSaveWorker<BackgroundSaveCompletion>> {
    BackgroundSaveWorker::new("clonk-save-worker", SAVE_WORKER_QUEUE_CAPACITY)
}

pub(crate) fn native_slot_save_job(
    request: NativeSlotSaveRequest,
) -> BackgroundSaveJob<BackgroundSaveCompletion> {
    Box::new(move || request.finish())
}

pub(crate) fn player_file_save_job(
    prepared: PreparedPlayerFileSave,
) -> BackgroundSaveJob<BackgroundSaveCompletion> {
    Box::new(move || prepared.finish())
}

pub(crate) fn runtime_dynamic_save_job(
    request: RuntimeDynamicSaveRequest,
) -> BackgroundSaveJob<BackgroundSaveCompletion> {
    Box::new(move || request.finish())
}

impl<T: Send + 'static> Drop for BackgroundSaveWorker<T> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::{
        BackgroundSaveCompletion, BackgroundSaveSubmitError, BackgroundSaveWorker,
        PreparedPlayerFileSave,
    };
    use clonk_resources::MutableGroup;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn a_blocked_save_job_does_not_block_its_submitter() {
        let worker = BackgroundSaveWorker::new("test-save-worker", 1).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        worker
            .try_submit(Box::new(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                37
            }))
            .unwrap();

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(worker.try_recv().is_none());
        release_tx.send(()).unwrap();
        assert_eq!(worker.recv_timeout(Duration::from_secs(1)), Some(37));
    }

    #[test]
    fn save_worker_queue_is_bounded_while_a_job_is_blocked() {
        let worker = BackgroundSaveWorker::new("test-bounded-save-worker", 1).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        worker
            .try_submit(Box::new(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                1
            }))
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.try_submit(Box::new(|| 2)).unwrap();

        assert_eq!(
            worker.try_submit(Box::new(|| 3)),
            Err(BackgroundSaveSubmitError::Full)
        );
        release_tx.send(()).unwrap();
        assert_eq!(worker.recv_timeout(Duration::from_secs(1)), Some(1));
        assert_eq!(worker.recv_timeout(Duration::from_secs(1)), Some(2));
    }

    #[test]
    fn player_file_persistence_failure_is_reported_by_the_worker_completion() {
        let directory = tempdir().unwrap();
        let blocking_parent = directory.path().join("ordinary-file");
        fs::write(&blocking_parent, b"not a directory").unwrap();
        let mut group = MutableGroup::new("Player.c4p");
        group
            .add_file("Player.txt", b"[Player]\n".to_vec())
            .unwrap();

        let completion = PreparedPlayerFileSave {
            player_number: 1,
            info_id: 2,
            group,
            path: blocking_parent.join("Player.c4p"),
            preserve_folder_group: false,
            official_derivation: false,
            derivation: None,
        }
        .finish();

        assert!(matches!(
            completion,
            BackgroundSaveCompletion::PlayerFile(completion) if completion.result.is_err()
        ));
    }
}
