use crate::model::DisplayedGeometry;
use crate::viewport::DisplayGeometryRevision;
use blake3::Hasher;
use geom_diagnostics::{DiagnosticReport, DiagnosticTiming};
use geom_geometry::{
    BoolmeshBackend, GeometryEvaluator, diagnostic_from_geometry_error, validate_evaluated_geometry,
};
use geom_scene::{
    NodeId, ParamId, SceneDocument, parse_scene_report,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The debounce window used to coalesce editor save bursts.
pub const DEFAULT_DEBOUNCE_WINDOW: Duration = Duration::from_millis(75);

/// A deterministic fingerprint of scene source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceFingerprint([u8; 32]);

impl SourceFingerprint {
    pub fn from_text(source_text: &str) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(source_text.as_bytes());
        Self(*hasher.finalize().as_bytes())
    }
}

/// Monotonic app-session source revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SourceRevision(u64);

impl SourceRevision {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn advance(&mut self) -> Self {
        self.0 += 1;
        *self
    }
}

/// Monotonic build-request generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BuildGeneration(u64);

impl BuildGeneration {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn advance(&mut self) -> Self {
        self.0 += 1;
        *self
    }
}

/// Transient workspace session identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WorkspaceSessionId(u64);

impl WorkspaceSessionId {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn advance(&mut self) -> Self {
        self.0 += 1;
        *self
    }
}

/// Reactive origin metadata for a source/build request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOrigin {
    StartupReopen,
    ExternalFile,
    Gui,
    Programmatic,
    ManualReload,
}

/// A deterministic coalescer for raw watcher events.
#[derive(Debug, Clone)]
pub struct SourceEventCoalescer {
    debounce_window: Duration,
    pending_since: Option<Instant>,
    pending_count: usize,
}

impl SourceEventCoalescer {
    pub fn new(debounce_window: Duration) -> Self {
        Self {
            debounce_window,
            pending_since: None,
            pending_count: 0,
        }
    }

    pub fn observe(&mut self, observed_at: Instant) {
        self.pending_since = Some(observed_at);
        self.pending_count += 1;
    }

    pub fn ready(&self, now: Instant) -> bool {
        self.pending_since.is_some_and(|pending_since| {
            now.saturating_duration_since(pending_since) >= self.debounce_window
        })
    }

    pub fn drain_ready(&mut self, now: Instant) -> Option<usize> {
        if !self.ready(now) {
            return None;
        }

        let pending_count = self.pending_count;
        self.pending_since = None;
        self.pending_count = 0;
        Some(pending_count)
    }

    pub fn is_pending(&self) -> bool {
        self.pending_since.is_some()
    }

    pub fn pending_count(&self) -> usize {
        self.pending_count
    }
}

/// A logical accepted source snapshot.
#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    pub session_id: WorkspaceSessionId,
    pub source_revision: SourceRevision,
    pub fingerprint: SourceFingerprint,
    pub source_text: String,
    pub origin: EditOrigin,
    pub detected_at: Instant,
}

/// A worker build request derived from one accepted source snapshot.
#[derive(Debug, Clone)]
pub struct BuildRequestSnapshot {
    pub session_id: WorkspaceSessionId,
    pub generation: BuildGeneration,
    pub source_revision: SourceRevision,
    pub source_fingerprint: SourceFingerprint,
    pub source_text: String,
    pub origin: EditOrigin,
    pub requested_at: Instant,
}

/// Timings attached to a generation-tagged build attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReactiveBuildTimings {
    pub parse_millis: f64,
    pub evaluation_millis: f64,
    pub mesh_upload_millis: f64,
    pub total_millis: f64,
}

impl ReactiveBuildTimings {
    pub const fn zero() -> Self {
        Self {
            parse_millis: 0.0,
            evaluation_millis: 0.0,
            mesh_upload_millis: 0.0,
            total_millis: 0.0,
        }
    }
}

/// The stage that failed for a build generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStage {
    Workspace,
    Scene,
    Geometry,
    Conflict,
}

/// Concise structured failure state for M05.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactiveDiagnostic {
    pub stage: DiagnosticStage,
    pub source_revision: SourceRevision,
    pub generation: BuildGeneration,
    pub report: DiagnosticReport,
}

/// A worker-side success payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactiveBuildSuccess {
    pub scene: SceneDocument,
    pub geometry: Option<DisplayedGeometry>,
    pub changed_node_ids: Vec<NodeId>,
    pub changed_parameter_ids: Vec<ParamId>,
    pub semantic_scene_changed: bool,
    pub timings: ReactiveBuildTimings,
}

/// A generation-tagged worker outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildOutcomeKind {
    Success(Box<ReactiveBuildSuccess>),
    Failure {
        stage: DiagnosticStage,
        report: DiagnosticReport,
        timings: ReactiveBuildTimings,
    },
}

/// A generation-tagged worker outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildOutcome {
    pub session_id: WorkspaceSessionId,
    pub generation: BuildGeneration,
    pub source_revision: SourceRevision,
    pub source_fingerprint: SourceFingerprint,
    pub origin: EditOrigin,
    pub requested_at: Instant,
    pub kind: BuildOutcomeKind,
}

/// Acceptance decision for an arriving worker result.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildAcceptance {
    Accepted(Box<BuildOutcome>),
    IgnoredStale,
}

/// Pending own-write echo suppression metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnWriteEcho {
    pub fingerprint: SourceFingerprint,
    pub source_revision: SourceRevision,
}

/// UI-facing reactive snapshot state.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactiveStatusSnapshot {
    pub watching: bool,
    pub source_revision: SourceRevision,
    pub newest_generation: BuildGeneration,
    pub last_accepted_generation: Option<BuildGeneration>,
    pub last_successful_generation: Option<BuildGeneration>,
    pub current_error_generation: Option<BuildGeneration>,
    pub last_successful_geometry_revision: DisplayGeometryRevision,
    pub build_in_progress: bool,
    pub pending_file_events: usize,
    pub timings: Option<ReactiveBuildTimings>,
}

/// Presentation-neutral reactive session controller.
#[derive(Debug, Clone)]
pub struct ReactiveController {
    session_id: WorkspaceSessionId,
    current_source_revision: SourceRevision,
    current_source_fingerprint: Option<SourceFingerprint>,
    newest_requested_generation: BuildGeneration,
    last_accepted_generation: Option<BuildGeneration>,
    last_successful_generation: Option<BuildGeneration>,
    current_error_generation: Option<BuildGeneration>,
    last_successful_geometry_revision: DisplayGeometryRevision,
    last_timings: Option<ReactiveBuildTimings>,
    build_in_progress: bool,
    watching: bool,
    coalescer: SourceEventCoalescer,
    pending_own_write: Option<OwnWriteEcho>,
}

impl ReactiveController {
    pub fn new() -> Self {
        Self {
            session_id: WorkspaceSessionId::ZERO,
            current_source_revision: SourceRevision::ZERO,
            current_source_fingerprint: None,
            newest_requested_generation: BuildGeneration::ZERO,
            last_accepted_generation: None,
            last_successful_generation: None,
            current_error_generation: None,
            last_successful_geometry_revision: DisplayGeometryRevision::ZERO,
            last_timings: None,
            build_in_progress: false,
            watching: false,
            coalescer: SourceEventCoalescer::new(DEFAULT_DEBOUNCE_WINDOW),
            pending_own_write: None,
        }
    }

    pub fn status_snapshot(&self) -> ReactiveStatusSnapshot {
        ReactiveStatusSnapshot {
            watching: self.watching,
            source_revision: self.current_source_revision,
            newest_generation: self.newest_requested_generation,
            last_accepted_generation: self.last_accepted_generation,
            last_successful_generation: self.last_successful_generation,
            current_error_generation: self.current_error_generation,
            last_successful_geometry_revision: self.last_successful_geometry_revision,
            build_in_progress: self.build_in_progress,
            pending_file_events: self.coalescer.pending_count(),
            timings: self.last_timings,
        }
    }

    pub fn current_source_revision(&self) -> SourceRevision {
        self.current_source_revision
    }

    pub fn current_source_fingerprint(&self) -> Option<SourceFingerprint> {
        self.current_source_fingerprint
    }

    pub fn begin_session(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> BuildRequestSnapshot {
        self.session_id.advance();
        self.current_source_revision = SourceRevision::ZERO;
        self.newest_requested_generation = BuildGeneration::ZERO;
        self.last_accepted_generation = None;
        self.last_successful_generation = None;
        self.current_error_generation = None;
        self.last_successful_geometry_revision = DisplayGeometryRevision::ZERO;
        self.last_timings = None;
        self.build_in_progress = false;
        self.pending_own_write = None;
        self.coalescer = SourceEventCoalescer::new(DEFAULT_DEBOUNCE_WINDOW);

        self.accept_source_snapshot(source_text, origin, now)
    }

    pub fn set_watching(&mut self, watching: bool) {
        self.watching = watching;
    }

    pub fn observe_file_event(&mut self, observed_at: Instant) {
        self.coalescer.observe(observed_at);
    }

    pub fn drain_ready_file_event(&mut self, now: Instant) -> Option<usize> {
        self.coalescer.drain_ready(now)
    }

    pub fn note_manual_rebuild(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> BuildRequestSnapshot {
        self.schedule_generation(source_text, origin, now)
    }

    pub fn accept_external_source_reload(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> Option<BuildRequestSnapshot> {
        let fingerprint = SourceFingerprint::from_text(source_text);
        if self.pending_own_write
            == Some(OwnWriteEcho {
                fingerprint,
                source_revision: self.current_source_revision,
            })
        {
            self.pending_own_write = None;
            return None;
        }

        if self.current_source_fingerprint == Some(fingerprint) {
            return None;
        }

        Some(self.accept_source_snapshot(source_text, origin, now))
    }

    pub fn clear_own_write_if_matches_current_source(&mut self, source_text: &str) -> bool {
        let fingerprint = SourceFingerprint::from_text(source_text);
        if self.pending_own_write
            == Some(OwnWriteEcho {
                fingerprint,
                source_revision: self.current_source_revision,
            })
        {
            self.pending_own_write = None;
            return true;
        }
        false
    }

    pub fn accept_internal_source_write(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> BuildRequestSnapshot {
        let snapshot = self.accept_source_snapshot(source_text, origin, now);
        self.pending_own_write = Some(OwnWriteEcho {
            fingerprint: snapshot.source_fingerprint,
            source_revision: snapshot.source_revision,
        });
        snapshot
    }

    pub fn accept_build_outcome(&mut self, outcome: BuildOutcome) -> BuildAcceptance {
        if outcome.session_id != self.session_id
            || outcome.generation != self.newest_requested_generation
        {
            return BuildAcceptance::IgnoredStale;
        }

        self.build_in_progress = false;
        self.last_accepted_generation = Some(outcome.generation);
        match &outcome.kind {
            BuildOutcomeKind::Success(success) => {
                self.current_error_generation = None;
                self.last_successful_generation = Some(outcome.generation);
                if let Some(geometry) = &success.geometry {
                    self.last_successful_geometry_revision =
                        DisplayGeometryRevision::new(geometry.geometry_revision);
                }
                self.last_timings = Some(success.timings);
            }
            BuildOutcomeKind::Failure { timings, .. } => {
                self.current_error_generation = Some(outcome.generation);
                self.last_timings = Some(*timings);
            }
        }

        BuildAcceptance::Accepted(Box::new(outcome))
    }

    pub fn note_mesh_upload_complete(&mut self, mesh_upload_millis: f64, total_millis: f64) {
        if let Some(mut timings) = self.last_timings {
            timings.mesh_upload_millis = mesh_upload_millis;
            timings.total_millis = total_millis;
            self.last_timings = Some(timings);
        }
    }

    fn accept_source_snapshot(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> BuildRequestSnapshot {
        let fingerprint = SourceFingerprint::from_text(source_text);
        self.current_source_fingerprint = Some(fingerprint);
        let source_revision = self.current_source_revision.advance();
        self.schedule_generation_with_snapshot(SourceSnapshot {
            session_id: self.session_id,
            source_revision,
            fingerprint,
            source_text: source_text.to_owned(),
            origin,
            detected_at: now,
        })
    }

    fn schedule_generation(
        &mut self,
        source_text: &str,
        origin: EditOrigin,
        now: Instant,
    ) -> BuildRequestSnapshot {
        self.schedule_generation_with_snapshot(SourceSnapshot {
            session_id: self.session_id,
            source_revision: self.current_source_revision,
            fingerprint: self
                .current_source_fingerprint
                .unwrap_or_else(|| SourceFingerprint::from_text(source_text)),
            source_text: source_text.to_owned(),
            origin,
            detected_at: now,
        })
    }

    fn schedule_generation_with_snapshot(
        &mut self,
        snapshot: SourceSnapshot,
    ) -> BuildRequestSnapshot {
        let generation = self.newest_requested_generation.advance();
        self.build_in_progress = true;
        BuildRequestSnapshot {
            session_id: snapshot.session_id,
            generation,
            source_revision: snapshot.source_revision,
            source_fingerprint: snapshot.fingerprint,
            source_text: snapshot.source_text,
            origin: snapshot.origin,
            requested_at: snapshot.detected_at,
        }
    }
}

impl Default for ReactiveController {
    fn default() -> Self {
        Self::new()
    }
}

/// Worker-specific command messages.
#[derive(Debug, Clone)]
pub enum WorkerCommand {
    Build(BuildRequestSnapshot),
    Shutdown,
}

/// Builds scene snapshots while preserving evaluator cache across the same session.
#[derive(Debug)]
pub struct BuildWorker {
    evaluator: GeometryEvaluator<BoolmeshBackend>,
    last_successful_scene: Option<SceneDocument>,
}

impl BuildWorker {
    pub fn new() -> Self {
        Self {
            evaluator: GeometryEvaluator::new(BoolmeshBackend::new()),
            last_successful_scene: None,
        }
    }

    pub fn process(&mut self, request: BuildRequestSnapshot) -> BuildOutcome {
        let total_started = request.requested_at;
        let parse_started = Instant::now();
        let scene = match parse_scene_report(&request.source_text) {
            Ok(scene) => scene,
            Err(mut report) => {
                let now = Instant::now();
                attach_timings(
                    &mut report,
                    ReactiveBuildTimings {
                        parse_millis: now
                            .saturating_duration_since(parse_started)
                            .as_secs_f64()
                            * 1_000.0,
                        evaluation_millis: 0.0,
                        mesh_upload_millis: 0.0,
                        total_millis: now
                            .saturating_duration_since(total_started)
                            .as_secs_f64()
                            * 1_000.0,
                    },
                );
                return BuildOutcome {
                    session_id: request.session_id,
                    generation: request.generation,
                    source_revision: request.source_revision,
                    source_fingerprint: request.source_fingerprint,
                    origin: request.origin,
                    requested_at: request.requested_at,
                    kind: BuildOutcomeKind::Failure {
                        stage: DiagnosticStage::Scene,
                        report,
                        timings: ReactiveBuildTimings {
                            parse_millis: now
                                .saturating_duration_since(parse_started)
                                .as_secs_f64()
                                * 1_000.0,
                            evaluation_millis: 0.0,
                            mesh_upload_millis: 0.0,
                            total_millis: now
                                .saturating_duration_since(total_started)
                                .as_secs_f64()
                                * 1_000.0,
                        },
                    },
                };
            }
        };
        let parse_millis = Instant::now()
            .saturating_duration_since(parse_started)
            .as_secs_f64()
            * 1_000.0;

        let semantic_scene_changed = self
            .last_successful_scene
            .as_ref()
            .map(|last| last != &scene)
            .unwrap_or(true);
        let (changed_node_ids, changed_parameter_ids) =
            changed_ids(self.last_successful_scene.as_ref(), &scene);

        if !semantic_scene_changed {
            let total_millis = Instant::now()
                .saturating_duration_since(total_started)
                .as_secs_f64()
                * 1_000.0;
            return BuildOutcome {
                session_id: request.session_id,
                generation: request.generation,
                source_revision: request.source_revision,
                source_fingerprint: request.source_fingerprint,
                origin: request.origin,
                requested_at: request.requested_at,
                kind: BuildOutcomeKind::Success(Box::new(ReactiveBuildSuccess {
                    scene,
                    geometry: None,
                    changed_node_ids,
                    changed_parameter_ids,
                    semantic_scene_changed: false,
                    timings: ReactiveBuildTimings {
                        parse_millis,
                        evaluation_millis: 0.0,
                        mesh_upload_millis: 0.0,
                        total_millis,
                    },
                })),
            };
        }

        let evaluation_started = Instant::now();
        match self.evaluator.evaluate_root(&scene) {
            Ok(geometry) => {
                let evaluation_millis = Instant::now()
                    .saturating_duration_since(evaluation_started)
                    .as_secs_f64()
                    * 1_000.0;
                let total_millis = Instant::now()
                    .saturating_duration_since(total_started)
                    .as_secs_f64()
                    * 1_000.0;
                let mut report = validate_evaluated_geometry(&geometry);
                if report.has_blocking() {
                    attach_timings(
                        &mut report,
                        ReactiveBuildTimings {
                            parse_millis,
                            evaluation_millis,
                            mesh_upload_millis: 0.0,
                            total_millis,
                        },
                    );
                    return BuildOutcome {
                        session_id: request.session_id,
                        generation: request.generation,
                        source_revision: request.source_revision,
                        source_fingerprint: request.source_fingerprint,
                        origin: request.origin,
                        requested_at: request.requested_at,
                        kind: BuildOutcomeKind::Failure {
                            stage: DiagnosticStage::Geometry,
                            report,
                            timings: ReactiveBuildTimings {
                                parse_millis,
                                evaluation_millis,
                                mesh_upload_millis: 0.0,
                                total_millis,
                            },
                        },
                    };
                }
                self.last_successful_scene = Some(scene.clone());
                BuildOutcome {
                    session_id: request.session_id,
                    generation: request.generation,
                    source_revision: request.source_revision,
                    source_fingerprint: request.source_fingerprint,
                    origin: request.origin,
                    requested_at: request.requested_at,
                    kind: BuildOutcomeKind::Success(Box::new(ReactiveBuildSuccess {
                        scene,
                        geometry: Some(DisplayedGeometry::from_evaluated(geometry)),
                        changed_node_ids,
                        changed_parameter_ids,
                        semantic_scene_changed: true,
                        timings: ReactiveBuildTimings {
                            parse_millis,
                            evaluation_millis,
                            mesh_upload_millis: 0.0,
                            total_millis,
                        },
                    })),
                }
            }
            Err(error) => {
                let now = Instant::now();
                let mut report =
                    DiagnosticReport::new(vec![diagnostic_from_geometry_error(&error)]);
                attach_timings(
                    &mut report,
                    ReactiveBuildTimings {
                        parse_millis,
                        evaluation_millis: now
                            .saturating_duration_since(evaluation_started)
                            .as_secs_f64()
                            * 1_000.0,
                        mesh_upload_millis: 0.0,
                        total_millis: now
                            .saturating_duration_since(total_started)
                            .as_secs_f64()
                            * 1_000.0,
                    },
                );
                BuildOutcome {
                    session_id: request.session_id,
                    generation: request.generation,
                    source_revision: request.source_revision,
                    source_fingerprint: request.source_fingerprint,
                    origin: request.origin,
                    requested_at: request.requested_at,
                    kind: BuildOutcomeKind::Failure {
                        stage: DiagnosticStage::Geometry,
                        report,
                        timings: ReactiveBuildTimings {
                            parse_millis,
                            evaluation_millis: now
                                .saturating_duration_since(evaluation_started)
                                .as_secs_f64()
                                * 1_000.0,
                            mesh_upload_millis: 0.0,
                            total_millis: now
                                .saturating_duration_since(total_started)
                                .as_secs_f64()
                                * 1_000.0,
                        },
                    },
                }
            }
        }
    }
}

impl Default for BuildWorker {
    fn default() -> Self {
        Self::new()
    }
}

fn attach_timings(report: &mut DiagnosticReport, timings: ReactiveBuildTimings) {
    for diagnostic in &mut report.diagnostics {
        diagnostic.telemetry = Some(DiagnosticTiming {
            parse_millis: Some(timings.parse_millis.round() as u64),
            validation_millis: None,
            evaluation_millis: Some(timings.evaluation_millis.round() as u64),
            total_millis: Some(timings.total_millis.round() as u64),
        });
    }
}

fn changed_ids(
    previous: Option<&SceneDocument>,
    current: &SceneDocument,
) -> (Vec<NodeId>, Vec<ParamId>) {
    let Some(previous) = previous else {
        return (
            current.nodes().keys().cloned().collect(),
            current.parameters().keys().cloned().collect(),
        );
    };

    let mut changed_nodes = BTreeSet::new();
    for (node_id, node) in current.nodes() {
        if previous.nodes().get(node_id) != Some(node) {
            changed_nodes.insert(node_id.clone());
        }
    }
    for node_id in previous.nodes().keys() {
        if !current.nodes().contains_key(node_id) {
            changed_nodes.insert(node_id.clone());
        }
    }

    let mut changed_parameters = BTreeSet::new();
    for (param_id, parameter) in current.parameters() {
        if previous.parameters().get(param_id) != Some(parameter) {
            changed_parameters.insert(param_id.clone());
        }
    }
    for param_id in previous.parameters().keys() {
        if !current.parameters().contains_key(param_id) {
            changed_parameters.insert(param_id.clone());
        }
    }

    (
        changed_nodes.into_iter().collect(),
        changed_parameters.into_iter().collect(),
    )
}

pub fn is_relevant_watch_event(scene_path: &Path, event_paths: &[PathBuf]) -> bool {
    event_paths
        .iter()
        .any(|event_path| event_path == scene_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SOURCE: &str = r#"
schema_version = 1
root = "root"

[params.width]
type = "scalar"
value = 3.0

[nodes.root]
kind = "box"
size = { x = 1.0, y = 2.0, z = 3.0 }
"#;

    const VALID_SOURCE_WITH_COMMENT: &str = r#"
# changed comment
schema_version = 1
root = "root"

[params.width]
type = "scalar"
value = 3.0

[nodes.root]
kind = "box"
size = { x = 1.0, y = 2.0, z = 3.0 }
"#;

    #[test]
    fn coalescer_drains_one_logical_reload_after_burst() {
        let started = Instant::now();
        let mut coalescer = SourceEventCoalescer::new(Duration::from_millis(75));
        coalescer.observe(started);
        coalescer.observe(started + Duration::from_millis(15));
        coalescer.observe(started + Duration::from_millis(35));
        assert!(coalescer.is_pending());
        assert_eq!(
            coalescer.drain_ready(started + Duration::from_millis(74)),
            None
        );
        assert_eq!(
            coalescer.drain_ready(started + Duration::from_millis(110)),
            Some(3)
        );
        assert!(!coalescer.is_pending());
    }

    #[test]
    fn stale_generation_is_ignored() {
        let now = Instant::now();
        let mut controller = ReactiveController::new();
        let _first = controller.begin_session(VALID_SOURCE, EditOrigin::StartupReopen, now);
        let newest = controller.note_manual_rebuild(
            VALID_SOURCE,
            EditOrigin::ManualReload,
            now + Duration::from_millis(1),
        );

        let stale = BuildOutcome {
            session_id: WorkspaceSessionId::new(1),
            generation: BuildGeneration::new(1),
            source_revision: SourceRevision::new(1),
            source_fingerprint: SourceFingerprint::from_text(VALID_SOURCE),
            origin: EditOrigin::StartupReopen,
            requested_at: now,
            kind: BuildOutcomeKind::Failure {
                stage: DiagnosticStage::Scene,
                message: "stale".to_owned(),
                timings: ReactiveBuildTimings::zero(),
            },
        };
        assert_eq!(
            controller.accept_build_outcome(stale),
            BuildAcceptance::IgnoredStale
        );

        let accepted = BuildOutcome {
            session_id: newest.session_id,
            generation: newest.generation,
            source_revision: newest.source_revision,
            source_fingerprint: newest.source_fingerprint,
            origin: newest.origin,
            requested_at: newest.requested_at,
            kind: BuildOutcomeKind::Failure {
                stage: DiagnosticStage::Scene,
                message: "current".to_owned(),
                timings: ReactiveBuildTimings::zero(),
            },
        };
        assert!(matches!(
            controller.accept_build_outcome(accepted),
            BuildAcceptance::Accepted(_)
        ));
    }

    #[test]
    fn own_write_echo_is_suppressed_but_external_change_is_not() {
        let now = Instant::now();
        let mut controller = ReactiveController::new();
        let _ = controller.begin_session(VALID_SOURCE, EditOrigin::StartupReopen, now);

        let own = controller.accept_internal_source_write(
            VALID_SOURCE_WITH_COMMENT,
            EditOrigin::Gui,
            now + Duration::from_millis(1),
        );
        assert_eq!(own.source_revision.get(), 2);
        assert!(
            controller
                .accept_external_source_reload(
                    VALID_SOURCE_WITH_COMMENT,
                    EditOrigin::ExternalFile,
                    now + Duration::from_millis(2)
                )
                .is_none()
        );
        assert!(
            controller
                .accept_external_source_reload(
                    VALID_SOURCE,
                    EditOrigin::ExternalFile,
                    now + Duration::from_millis(3)
                )
                .is_some()
        );
    }

    #[test]
    fn semantic_noop_source_change_preserves_geometry_revision() {
        let now = Instant::now();
        let mut controller = ReactiveController::new();
        let initial = controller.begin_session(VALID_SOURCE, EditOrigin::StartupReopen, now);
        let mut worker = BuildWorker::new();
        let first = worker.process(initial.clone());
        let BuildAcceptance::Accepted(first) = controller.accept_build_outcome(first) else {
            panic!("first build should accept");
        };
        let first_geometry_revision = match &first.kind {
            BuildOutcomeKind::Success(success) => {
                success
                    .geometry
                    .as_ref()
                    .expect("initial geometry")
                    .geometry_revision
            }
            BuildOutcomeKind::Failure { .. } => panic!("expected success"),
        };

        let second = controller
            .accept_external_source_reload(
                VALID_SOURCE_WITH_COMMENT,
                EditOrigin::ExternalFile,
                now + Duration::from_millis(1),
            )
            .expect("source revision advances");
        let second = worker.process(second);
        let BuildAcceptance::Accepted(second) = controller.accept_build_outcome(second) else {
            panic!("second build should accept");
        };
        match second.kind {
            BuildOutcomeKind::Success(success) => {
                assert!(!success.semantic_scene_changed);
                assert!(success.geometry.is_none());
            }
            BuildOutcomeKind::Failure { .. } => panic!("expected success"),
        }
        assert_eq!(
            controller
                .status_snapshot()
                .last_successful_geometry_revision
                .get(),
            first_geometry_revision
        );
    }

    #[test]
    fn changed_ids_reflect_semantic_differences() {
        let previous = parse_scene(VALID_SOURCE).expect("parse previous");
        let current = parse_scene(
            r#"
schema_version = 1
root = "root"

[params.width]
type = "scalar"
value = 4.0

[nodes.root]
kind = "box"
size = { x = 1.0, y = 2.0, z = 3.0 }
"#,
        )
        .expect("parse current");
        let (nodes, params) = changed_ids(Some(&previous), &current);
        assert!(nodes.is_empty());
        assert_eq!(params, vec![ParamId::new("width").expect("id")]);
    }

    #[test]
    fn rapid_consecutive_edits_converge_on_newest_generation() {
        let now = Instant::now();
        let mut controller = ReactiveController::new();
        let initial = controller.begin_session(VALID_SOURCE, EditOrigin::StartupReopen, now);
        let mut worker = BuildWorker::new();
        let _ = controller.accept_build_outcome(worker.process(initial));

        let a = controller
            .accept_external_source_reload(
                &VALID_SOURCE.replace("value = 3.0", "value = 3.1"),
                EditOrigin::ExternalFile,
                now + Duration::from_millis(1),
            )
            .expect("a");
        let b = controller
            .accept_external_source_reload(
                &VALID_SOURCE.replace("value = 3.0", "value = 3.2"),
                EditOrigin::ExternalFile,
                now + Duration::from_millis(2),
            )
            .expect("b");
        let c = controller
            .accept_external_source_reload(
                &VALID_SOURCE.replace("value = 3.0", "value = 3.3"),
                EditOrigin::ExternalFile,
                now + Duration::from_millis(3),
            )
            .expect("c");
        let d = controller
            .accept_external_source_reload(
                &VALID_SOURCE.replace("value = 3.0", "value = 3.4"),
                EditOrigin::ExternalFile,
                now + Duration::from_millis(4),
            )
            .expect("d");

        assert_eq!(
            controller.accept_build_outcome(worker.process(a)),
            BuildAcceptance::IgnoredStale
        );
        assert_eq!(
            controller.accept_build_outcome(worker.process(b)),
            BuildAcceptance::IgnoredStale
        );
        assert_eq!(
            controller.accept_build_outcome(worker.process(c)),
            BuildAcceptance::IgnoredStale
        );
        let BuildAcceptance::Accepted(outcome) = controller.accept_build_outcome(worker.process(d))
        else {
            panic!("newest generation should be accepted");
        };
        assert_eq!(
            outcome.generation.get(),
            controller.status_snapshot().newest_generation.get()
        );
        assert_eq!(
            controller
                .status_snapshot()
                .last_successful_generation
                .map(|generation| generation.get()),
            Some(outcome.generation.get())
        );
    }
}
