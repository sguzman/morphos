//! Durable workspace primitives for Morphos.
//!
//! `geom_workspace` is the canonical owner of repository-level workspace state.
//! M01 intentionally keeps the data model narrow:
//!
//! - versioned workspace metadata
//! - a stable workspace ID
//! - reserved workspace paths
//! - opaque scene/source text owned at the workspace layer
//! - dirty/revision/change tracking
//! - explicit save and conservative interrupted-write recovery
//!
//! The crate does not define scene schema, geometry types, GUI concerns, or AI
//! provider integrations.
//!
//! Current on-disk layout:
//!
//! - `<root>/source/scene.toml`
//! - `<root>/exports/`
//! - `<root>/.morphos/workspace.toml`
//! - `<root>/.morphos/cache/`
//! - `<root>/.morphos/history/`
//! - `<root>/.morphos/ai/`
//!
//! `workspace.toml` stores only M01 workspace metadata: format version, stable
//! workspace ID, user-facing name, and optional description. The source file is
//! intentionally treated as opaque text here so M02 can own the actual scene
//! language design.
//!
//! Revision semantics are in-memory only. A clean workspace starts at revision
//! zero. Each observable mutation advances the revision by one change set, and
//! `save()` emits a clean-state transition when it flushes dirty state
//! successfully.
//!
//! Recovery policy favors the last known-good canonical data when state is
//! ambiguous: canonical files win over stale temp/backup files, a valid backup
//! is restored before promoting a temp file, and a valid temp file is promoted
//! only when no backup is available.

use geom_scene::{
    Axis, Node, NodeId, NodeKind, ParamId, PrimitiveScalarField, ScalarExpr, SceneDocument,
    SceneNodeDraft, SceneSource, TransformProperty,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use thiserror::Error;
use uuid::Uuid;

const WORKSPACE_STATE_DIR: &str = ".morphos";
const WORKSPACE_METADATA_FILE: &str = "workspace.toml";
const WORKSPACE_SOURCE_DIR: &str = "source";
const WORKSPACE_SOURCE_FILE: &str = "scene.toml";
const WORKSPACE_EXPORTS_DIR: &str = "exports";
const WORKSPACE_CACHE_DIR: &str = "cache";
const WORKSPACE_HISTORY_DIR: &str = "history";
const WORKSPACE_AI_DIR: &str = "ai";
const TEMP_SUFFIX: &str = ".tmp";
const BACKUP_SUFFIX: &str = ".bak";

/// The only workspace format version supported by this crate.
pub const WORKSPACE_FORMAT_VERSION: u32 = 1;

/// A monotonically increasing in-memory mutation revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Revision(u64);

impl Revision {
    /// The initial revision for a clean workspace.
    pub const ZERO: Self = Self(0);

    /// Returns the raw integer value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable workspace identity persisted across reopen operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(Uuid);

impl WorkspaceId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Durable workspace metadata stored in the canonical workspace manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    format_version: u32,
    workspace_id: WorkspaceId,
    name: String,
    description: Option<String>,
}

impl WorkspaceMetadata {
    fn new(name: String, description: Option<String>) -> Result<Self, WorkspaceError> {
        Ok(Self {
            format_version: WORKSPACE_FORMAT_VERSION,
            workspace_id: WorkspaceId::new(),
            name: normalize_name(name)?,
            description: normalize_description(description),
        })
    }

    /// Returns the durable workspace format version.
    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the stable workspace ID.
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    /// Returns the current workspace name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional workspace description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// Logical workspace directories owned or reserved by M01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceDirectory {
    Source,
    Exports,
    Cache,
    History,
    AiData,
}

/// Workspace resources surfaced by the revision/change model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceResource {
    Metadata,
    Source,
    DirtyState,
}

/// Project-owned workspace change types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceChange {
    MetadataChanged,
    SourceReloaded,
    SceneReplaced,
    DirtyStateChanged { is_dirty: bool },
}

impl WorkspaceChange {
    /// Returns the logical resource affected by this change.
    pub const fn resource(&self) -> WorkspaceResource {
        match self {
            Self::MetadataChanged => WorkspaceResource::Metadata,
            Self::SourceReloaded | Self::SceneReplaced => WorkspaceResource::Source,
            Self::DirtyStateChanged { .. } => WorkspaceResource::DirtyState,
        }
    }
}

/// A coherent change group emitted for a single workspace revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChangeSet {
    revision: Revision,
    changes: Vec<WorkspaceChange>,
}

impl WorkspaceChangeSet {
    /// Returns the revision at which this change set was recorded.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the ordered logical changes in the change set.
    pub fn changes(&self) -> &[WorkspaceChange] {
        &self.changes
    }
}

/// Lightweight summary suitable for future UI, CLI, or AI consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummary {
    root: PathBuf,
    workspace_id: WorkspaceId,
    format_version: u32,
    name: String,
    description: Option<String>,
    is_dirty: bool,
    revision: Revision,
}

impl WorkspaceSummary {
    /// Returns the workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the stable workspace ID.
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    /// Returns the supported format version stored by the workspace.
    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the workspace name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns whether the in-memory workspace differs from durable state.
    pub const fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Returns the current logical revision.
    pub const fn revision(&self) -> Revision {
        self.revision
    }
}

/// Project-owned stable path helpers for a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    root: PathBuf,
}

impl WorkspacePaths {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the workspace root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the private Morphos state directory.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(WORKSPACE_STATE_DIR)
    }

    /// Returns the canonical workspace metadata file path.
    pub fn metadata_file(&self) -> PathBuf {
        self.state_dir().join(WORKSPACE_METADATA_FILE)
    }

    /// Returns the source directory path.
    pub fn source_dir(&self) -> PathBuf {
        self.root.join(WORKSPACE_SOURCE_DIR)
    }

    /// Returns the reserved source document path.
    pub fn source_file(&self) -> PathBuf {
        self.source_dir().join(WORKSPACE_SOURCE_FILE)
    }

    /// Returns the exports directory path.
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join(WORKSPACE_EXPORTS_DIR)
    }

    /// Returns the cache directory path.
    pub fn cache_dir(&self) -> PathBuf {
        self.state_dir().join(WORKSPACE_CACHE_DIR)
    }

    /// Returns the history directory path.
    pub fn history_dir(&self) -> PathBuf {
        self.state_dir().join(WORKSPACE_HISTORY_DIR)
    }

    /// Returns the AI/session data directory path.
    pub fn ai_dir(&self) -> PathBuf {
        self.state_dir().join(WORKSPACE_AI_DIR)
    }

    /// Resolves a workspace-relative path inside a reserved workspace directory.
    pub fn resolve(
        &self,
        directory: WorkspaceDirectory,
        relative_path: impl AsRef<Path>,
    ) -> Result<PathBuf, WorkspaceError> {
        let normalized = normalize_relative_path(relative_path.as_ref())?;
        Ok(self.directory_path(directory).join(normalized))
    }

    fn directory_path(&self, directory: WorkspaceDirectory) -> PathBuf {
        match directory {
            WorkspaceDirectory::Source => self.source_dir(),
            WorkspaceDirectory::Exports => self.exports_dir(),
            WorkspaceDirectory::Cache => self.cache_dir(),
            WorkspaceDirectory::History => self.history_dir(),
            WorkspaceDirectory::AiData => self.ai_dir(),
        }
    }
}

/// Workspace creation options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceOptions {
    pub name: String,
    pub description: Option<String>,
}

impl CreateWorkspaceOptions {
    /// Creates options for a named workspace.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }
}

impl Default for CreateWorkspaceOptions {
    fn default() -> Self {
        Self::new("Morphos Workspace")
    }
}

/// Project-owned workspace error categories.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace destination is already populated: {path}")]
    DestinationNotEmpty { path: PathBuf },

    #[error("invalid workspace layout at {path}: {details}")]
    InvalidWorkspaceLayout { path: PathBuf, details: String },

    #[error(
        "unsupported workspace format version {found} at {path}; supported version is {supported}"
    )]
    UnsupportedFormatVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    #[error("workspace metadata is malformed at {path}: {source}")]
    MalformedMetadata {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("invalid workspace metadata field `{field}`: {message}")]
    InvalidMetadata {
        field: &'static str,
        message: String,
    },

    #[error("unsafe workspace-relative path {path}: {reason}")]
    UnsafePath { path: PathBuf, reason: String },

    #[error("revision {requested} is ahead of current revision {current}")]
    InvalidRevision {
        requested: Revision,
        current: Revision,
    },

    #[error("failed to persist workspace data at {path} during {operation}: {source}")]
    PersistenceFailed {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("failed to recover workspace file at {path}: {details}")]
    RecoveryFailed { path: PathBuf, details: String },

    #[error("filesystem operation `{operation}` failed at {path}: {source}")]
    Io {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: io::Error,
    },
}

/// Stable transaction identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransactionId(Uuid);

impl TransactionId {
    /// Creates a new transaction ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable operation identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(Uuid);

impl OperationId {
    /// Creates a new operation ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The actor responsible for a structured workspace transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionActor {
    User,
    Ai,
    CliAutomation,
    SystemMigration,
}

/// A typed semantic workspace operation.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceOp {
    AddNode {
        id: OperationId,
        node_id: NodeId,
        draft: SceneNodeDraft,
    },
    ReplaceNode {
        id: OperationId,
        node: Box<Node>,
    },
    DeleteNode {
        id: OperationId,
        node_id: NodeId,
    },
    RenameNode {
        id: OperationId,
        from: NodeId,
        to: NodeId,
    },
    DuplicateNode {
        id: OperationId,
        source_node: NodeId,
        duplicate: NodeId,
    },
    SetNodeLabel {
        id: OperationId,
        node_id: NodeId,
        label: Option<String>,
    },
    SetCompositionChildren {
        id: OperationId,
        node_id: NodeId,
        children: Vec<NodeId>,
    },
    SetParameterScalar {
        id: OperationId,
        parameter_id: ParamId,
        value: f64,
    },
    SetTransformComponent {
        id: OperationId,
        node_id: NodeId,
        property: TransformProperty,
        axis: Axis,
        value: f64,
    },
    SetPrimitiveScalar {
        id: OperationId,
        node_id: NodeId,
        field: PrimitiveScalarField,
        value: f64,
    },
    SetRootNode {
        id: OperationId,
        node_id: NodeId,
    },
}

impl WorkspaceOp {
    /// Returns the stable operation ID.
    pub fn id(&self) -> &OperationId {
        match self {
            Self::AddNode { id, .. }
            | Self::ReplaceNode { id, .. }
            | Self::DeleteNode { id, .. }
            | Self::RenameNode { id, .. }
            | Self::DuplicateNode { id, .. }
            | Self::SetNodeLabel { id, .. }
            | Self::SetCompositionChildren { id, .. }
            | Self::SetParameterScalar { id, .. }
            | Self::SetTransformComponent { id, .. }
            | Self::SetPrimitiveScalar { id, .. }
            | Self::SetRootNode { id, .. } => id,
        }
    }

    /// Returns the canonical mutation targets affected by this operation.
    pub fn affected_targets(&self) -> AffectedTargets {
        let mut targets = AffectedTargets::default();
        match self {
            Self::AddNode { node_id, .. }
            | Self::DeleteNode { node_id, .. }
            | Self::SetNodeLabel { node_id, .. }
            | Self::SetCompositionChildren { node_id, .. }
            | Self::SetTransformComponent { node_id, .. }
            | Self::SetPrimitiveScalar { node_id, .. }
            | Self::SetRootNode { node_id, .. } => {
                targets.node_ids.insert(node_id.clone());
            }
            Self::ReplaceNode { node, .. } => {
                targets.node_ids.insert(node.id().clone());
            }
            Self::RenameNode { from, to, .. } => {
                targets.node_ids.insert(from.clone());
                targets.node_ids.insert(to.clone());
            }
            Self::DuplicateNode {
                source_node,
                duplicate,
                ..
            } => {
                targets.node_ids.insert(source_node.clone());
                targets.node_ids.insert(duplicate.clone());
            }
            Self::SetParameterScalar { parameter_id, .. } => {
                targets.parameter_ids.insert(parameter_id.clone());
            }
        }
        targets
    }

    fn apply(&self, source: &mut SceneSource) -> Result<SceneDocument, geom_scene::SceneError> {
        match self {
            Self::AddNode { node_id, draft, .. } => source.add_node(node_id, draft.clone()),
            Self::ReplaceNode { node, .. } => source.set_node(node),
            Self::DeleteNode { node_id, .. } => source.delete_node(node_id),
            Self::RenameNode { from, to, .. } => source.rename_node(from, to),
            Self::DuplicateNode {
                source_node,
                duplicate,
                ..
            } => source.duplicate_node(source_node, duplicate),
            Self::SetNodeLabel { node_id, label, .. } => {
                source.set_node_label(node_id, label.as_deref())
            }
            Self::SetCompositionChildren {
                node_id, children, ..
            } => source.set_composition_children(node_id, children),
            Self::SetParameterScalar {
                parameter_id,
                value,
                ..
            } => source.set_parameter_scalar(parameter_id, *value),
            Self::SetTransformComponent {
                node_id,
                property,
                axis,
                value,
                ..
            } => source.set_transform_component(node_id, *property, *axis, *value),
            Self::SetPrimitiveScalar {
                node_id,
                field,
                value,
                ..
            } => source.set_primitive_scalar(node_id, *field, *value),
            Self::SetRootNode { node_id, .. } => source.set_root_node(node_id),
        }
    }

    fn clone_with_new_id(&self) -> Self {
        match self {
            Self::AddNode { node_id, draft, .. } => Self::AddNode {
                id: OperationId::new(),
                node_id: node_id.clone(),
                draft: draft.clone(),
            },
            Self::ReplaceNode { node, .. } => Self::ReplaceNode {
                id: OperationId::new(),
                node: node.clone(),
            },
            Self::DeleteNode { node_id, .. } => Self::DeleteNode {
                id: OperationId::new(),
                node_id: node_id.clone(),
            },
            Self::RenameNode { from, to, .. } => Self::RenameNode {
                id: OperationId::new(),
                from: from.clone(),
                to: to.clone(),
            },
            Self::DuplicateNode {
                source_node,
                duplicate,
                ..
            } => Self::DuplicateNode {
                id: OperationId::new(),
                source_node: source_node.clone(),
                duplicate: duplicate.clone(),
            },
            Self::SetNodeLabel { node_id, label, .. } => Self::SetNodeLabel {
                id: OperationId::new(),
                node_id: node_id.clone(),
                label: label.clone(),
            },
            Self::SetCompositionChildren {
                node_id, children, ..
            } => Self::SetCompositionChildren {
                id: OperationId::new(),
                node_id: node_id.clone(),
                children: children.clone(),
            },
            Self::SetParameterScalar {
                parameter_id,
                value,
                ..
            } => Self::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: parameter_id.clone(),
                value: *value,
            },
            Self::SetTransformComponent {
                node_id,
                property,
                axis,
                value,
                ..
            } => Self::SetTransformComponent {
                id: OperationId::new(),
                node_id: node_id.clone(),
                property: *property,
                axis: *axis,
                value: *value,
            },
            Self::SetPrimitiveScalar {
                node_id,
                field,
                value,
                ..
            } => Self::SetPrimitiveScalar {
                id: OperationId::new(),
                node_id: node_id.clone(),
                field: *field,
                value: *value,
            },
            Self::SetRootNode { node_id, .. } => Self::SetRootNode {
                id: OperationId::new(),
                node_id: node_id.clone(),
            },
        }
    }
}

/// One atomic structured workspace transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceTransaction {
    id: TransactionId,
    actor: TransactionActor,
    intent: Option<String>,
    operations: Vec<WorkspaceOp>,
}

impl WorkspaceTransaction {
    /// Creates a new transaction with the provided actor and operations.
    pub fn new(
        actor: TransactionActor,
        intent: Option<String>,
        operations: Vec<WorkspaceOp>,
    ) -> Result<Self, WorkspaceTransactionError> {
        if operations.is_empty() {
            return Err(WorkspaceTransactionError::EmptyTransaction);
        }
        Ok(Self {
            id: TransactionId::new(),
            actor,
            intent: normalize_transaction_intent(intent),
            operations,
        })
    }

    /// Returns the stable transaction ID.
    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    /// Returns the actor responsible for this transaction.
    pub const fn actor(&self) -> TransactionActor {
        self.actor
    }

    /// Returns the optional human-readable intent/summary.
    pub fn intent(&self) -> Option<&str> {
        self.intent.as_deref()
    }

    /// Returns the ordered operations in this transaction.
    pub fn operations(&self) -> &[WorkspaceOp] {
        &self.operations
    }

    /// Returns the union of affected node and parameter targets.
    pub fn affected_targets(&self) -> AffectedTargets {
        let mut targets = AffectedTargets::default();
        for operation in &self.operations {
            targets.extend(&operation.affected_targets());
        }
        targets
    }

    fn with_reissued_ids(&self, actor: TransactionActor, intent: Option<String>) -> Self {
        Self {
            id: TransactionId::new(),
            actor,
            intent: normalize_transaction_intent(intent),
            operations: self
                .operations
                .iter()
                .map(WorkspaceOp::clone_with_new_id)
                .collect(),
        }
    }
}

/// A concise target summary for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AffectedTargets {
    node_ids: BTreeSet<NodeId>,
    parameter_ids: BTreeSet<ParamId>,
}

impl AffectedTargets {
    /// Returns the affected node IDs.
    pub fn node_ids(&self) -> &BTreeSet<NodeId> {
        &self.node_ids
    }

    /// Returns the affected parameter IDs.
    pub fn parameter_ids(&self) -> &BTreeSet<ParamId> {
        &self.parameter_ids
    }

    fn extend(&mut self, other: &AffectedTargets) {
        self.node_ids.extend(other.node_ids.iter().cloned());
        self.parameter_ids
            .extend(other.parameter_ids.iter().cloned());
    }
}

/// A committed transaction record emitted by the structured mutation layer.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceTransactionCommit {
    transaction_id: TransactionId,
    actor: TransactionActor,
    intent: Option<String>,
    operation_ids: Vec<OperationId>,
    affected_targets: AffectedTargets,
    forward_transaction: WorkspaceTransaction,
    inverse_transaction: WorkspaceTransaction,
    revision_before: Revision,
    revision_after: Revision,
}

impl WorkspaceTransactionCommit {
    /// Returns the committed transaction ID.
    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    /// Returns the transaction actor.
    pub const fn actor(&self) -> TransactionActor {
        self.actor
    }

    /// Returns the optional human-readable transaction intent.
    pub fn intent(&self) -> Option<&str> {
        self.intent.as_deref()
    }

    /// Returns the stable operation IDs committed by this transaction.
    pub fn operation_ids(&self) -> &[OperationId] {
        &self.operation_ids
    }

    /// Returns the union of affected mutation targets.
    pub fn affected_targets(&self) -> &AffectedTargets {
        &self.affected_targets
    }

    /// Returns the committed forward transaction definition.
    pub fn forward_transaction(&self) -> &WorkspaceTransaction {
        &self.forward_transaction
    }

    /// Returns the generated inverse transaction definition.
    pub fn inverse_transaction(&self) -> &WorkspaceTransaction {
        &self.inverse_transaction
    }

    /// Returns the logical workspace revision before the commit.
    pub const fn revision_before(&self) -> Revision {
        self.revision_before
    }

    /// Returns the logical workspace revision after the commit.
    pub const fn revision_after(&self) -> Revision {
        self.revision_after
    }
}

/// Structured transaction-layer failures.
#[derive(Debug, Error)]
pub enum WorkspaceTransactionError {
    #[error("workspace transaction requires at least one operation")]
    EmptyTransaction,

    #[error("structured workspace transaction failed during validation: {source}")]
    SceneValidation {
        #[from]
        source: geom_scene::SceneError,
    },

    #[error("workspace transaction failed during persistence: {source}")]
    Workspace {
        #[from]
        source: WorkspaceError,
    },

    #[error("workspace transaction cannot be inverted safely: {message}")]
    NonInvertible { message: String },
}

/// In-memory undo/redo availability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UndoRedoAvailability {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedTransaction {
    forward: WorkspaceTransaction,
    inverse: WorkspaceTransaction,
}

/// In-memory transaction-level undo/redo owner for one workspace session.
#[derive(Debug, Clone, Default)]
pub struct UndoRedoManager {
    undo_stack: Vec<RecordedTransaction>,
    redo_stack: Vec<RecordedTransaction>,
}

impl UndoRedoManager {
    /// Returns whether undo or redo is currently available.
    pub fn availability(&self) -> UndoRedoAvailability {
        UndoRedoAvailability {
            can_undo: !self.undo_stack.is_empty(),
            can_redo: !self.redo_stack.is_empty(),
        }
    }

    /// Clears all in-memory undo/redo history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Records a newly committed forward transaction and clears redo state.
    pub fn record_commit(&mut self, commit: &WorkspaceTransactionCommit) {
        self.undo_stack.push(RecordedTransaction {
            forward: commit.forward_transaction().clone(),
            inverse: commit.inverse_transaction().clone(),
        });
        self.redo_stack.clear();
    }

    /// Applies one undo as one inverse transaction and moves the record to redo.
    pub fn undo(
        &mut self,
        workspace: &mut Workspace,
        actor: TransactionActor,
    ) -> Result<Option<WorkspaceTransactionCommit>, WorkspaceTransactionError> {
        let Some(record) = self.undo_stack.pop() else {
            return Ok(None);
        };

        let commit = workspace.apply_transaction(&record.inverse.with_reissued_ids(
            actor,
            Some(undo_intent(record.forward.intent(), record.forward.id())),
        ))?;
        self.redo_stack.push(record);
        Ok(Some(commit))
    }

    /// Reapplies one undone transaction and returns it to the undo stack.
    pub fn redo(
        &mut self,
        workspace: &mut Workspace,
        actor: TransactionActor,
    ) -> Result<Option<WorkspaceTransactionCommit>, WorkspaceTransactionError> {
        let Some(record) = self.redo_stack.pop() else {
            return Ok(None);
        };

        let commit = workspace.apply_transaction(&record.forward.with_reissued_ids(
            actor,
            Some(redo_intent(record.forward.intent(), record.forward.id())),
        ))?;
        self.undo_stack.push(record);
        Ok(Some(commit))
    }
}

/// An opened Morphos workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    paths: WorkspacePaths,
    metadata: WorkspaceMetadata,
    source_text: String,
    persisted_metadata: WorkspaceMetadata,
    persisted_source_text: String,
    is_dirty: bool,
    revision: Revision,
    change_log: Vec<WorkspaceChangeSet>,
}

impl Workspace {
    /// Creates a new workspace and returns it in the opened clean state.
    pub fn create(
        path: impl AsRef<Path>,
        options: CreateWorkspaceOptions,
    ) -> Result<Self, WorkspaceError> {
        create_workspace(path, options)
    }

    /// Opens an existing workspace from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        open_workspace(path)
    }

    /// Returns the workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns stable workspace path helpers.
    pub fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    /// Returns the current durable workspace metadata.
    pub fn metadata(&self) -> &WorkspaceMetadata {
        &self.metadata
    }

    /// Returns the opaque workspace source document text.
    pub fn source_text(&self) -> &str {
        &self.source_text
    }

    /// Returns whether in-memory state differs from the last successful save.
    pub const fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Returns the current logical revision.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns a lightweight workspace summary.
    pub fn summary(&self) -> WorkspaceSummary {
        WorkspaceSummary {
            root: self.root.clone(),
            workspace_id: self.metadata.workspace_id.clone(),
            format_version: self.metadata.format_version,
            name: self.metadata.name.clone(),
            description: self.metadata.description.clone(),
            is_dirty: self.is_dirty,
            revision: self.revision,
        }
    }

    /// Resolves a relative path inside a reserved workspace directory.
    pub fn resolve_path(
        &self,
        directory: WorkspaceDirectory,
        relative_path: impl AsRef<Path>,
    ) -> Result<PathBuf, WorkspaceError> {
        self.paths.resolve(directory, relative_path)
    }

    /// Updates the workspace name.
    ///
    /// Returns `true` when the value changed.
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<bool, WorkspaceError> {
        let normalized = normalize_name(name.into())?;
        if self.metadata.name == normalized {
            return Ok(false);
        }

        self.metadata.name = normalized;
        self.record_mutation(vec![WorkspaceChange::MetadataChanged]);
        Ok(true)
    }

    /// Updates the optional workspace description.
    ///
    /// Returns `true` when the value changed.
    pub fn set_description(&mut self, description: Option<String>) -> bool {
        let normalized = normalize_description(description);
        if self.metadata.description == normalized {
            return false;
        }

        self.metadata.description = normalized;
        self.record_mutation(vec![WorkspaceChange::MetadataChanged]);
        true
    }

    /// Replaces the opaque source document.
    ///
    /// Returns `true` when the value changed.
    pub fn replace_source(&mut self, source_text: impl Into<String>) -> bool {
        let source_text = source_text.into();
        if self.source_text == source_text {
            return false;
        }

        self.source_text = source_text;
        self.record_mutation(vec![WorkspaceChange::SceneReplaced]);
        true
    }

    /// Reloads the source document from disk.
    ///
    /// Returns `true` when the in-memory source changed.
    pub fn reload_source(&mut self) -> Result<bool, WorkspaceError> {
        let source_path = self.paths.source_file();
        let source_text = read_to_string(&source_path)?;
        if self.source_text == source_text {
            return Ok(false);
        }

        self.source_text = source_text;
        self.record_mutation(vec![WorkspaceChange::SourceReloaded]);
        Ok(true)
    }

    /// Persists the workspace metadata and source document explicitly.
    ///
    /// Save behavior is atomic per canonical file through temp-and-replace
    /// writes, but M01 does not attempt a multi-file transaction across both
    /// metadata and source files.
    pub fn save(&mut self) -> Result<(), WorkspaceError> {
        ensure_workspace_directories(&self.paths)?;

        let metadata_path = self.paths.metadata_file();
        let metadata_text = serialize_metadata(&self.metadata)?;
        write_atomic_file(&metadata_path, metadata_text.as_bytes())?;

        let source_path = self.paths.source_file();
        write_atomic_file(&source_path, self.source_text.as_bytes())?;

        self.persisted_metadata = self.metadata.clone();
        self.persisted_source_text = self.source_text.clone();

        if self.is_dirty {
            self.revision = Revision(self.revision.0 + 1);
            self.is_dirty = false;
            self.change_log.push(WorkspaceChangeSet {
                revision: self.revision,
                changes: vec![WorkspaceChange::DirtyStateChanged { is_dirty: false }],
            });
        }

        Ok(())
    }

    /// Validates and applies one structured transaction atomically.
    ///
    /// The transaction is validated against a temporary `SceneSource` first. No
    /// canonical workspace mutation occurs unless every operation succeeds.
    pub fn apply_transaction(
        &mut self,
        transaction: &WorkspaceTransaction,
    ) -> Result<WorkspaceTransactionCommit, WorkspaceTransactionError> {
        let mut source = SceneSource::parse(&self.source_text)?;
        let mut scene = source.validate()?;
        let mut inverse_operations = Vec::with_capacity(transaction.operations().len());
        for operation in transaction.operations() {
            inverse_operations.push(capture_inverse_operation(&scene, operation)?);
            scene = operation.apply(&mut source)?;
        }

        let updated_text = source.into_text();
        let revision_before = self.revision;
        let affected_targets = transaction.affected_targets();
        let operation_ids = transaction
            .operations()
            .iter()
            .map(|operation| operation.id().clone())
            .collect();
        let inverse_transaction = WorkspaceTransaction::new(
            transaction.actor(),
            Some(undo_intent(transaction.intent(), transaction.id())),
            inverse_operations.into_iter().rev().collect(),
        )
        .expect("inverse transaction contains operations");

        if self.replace_source(updated_text) {
            self.save()?;
        }

        Ok(WorkspaceTransactionCommit {
            transaction_id: transaction.id().clone(),
            actor: transaction.actor(),
            intent: transaction.intent.clone(),
            operation_ids,
            affected_targets,
            forward_transaction: transaction.clone(),
            inverse_transaction,
            revision_before,
            revision_after: self.revision,
        })
    }

    /// Returns ordered change sets recorded after the supplied revision.
    pub fn changes_since(
        &self,
        revision: Revision,
    ) -> Result<Vec<WorkspaceChangeSet>, WorkspaceError> {
        if revision > self.revision {
            return Err(WorkspaceError::InvalidRevision {
                requested: revision,
                current: self.revision,
            });
        }

        Ok(self
            .change_log
            .iter()
            .filter(|change_set| change_set.revision > revision)
            .cloned()
            .collect())
    }

    fn record_mutation(&mut self, mut changes: Vec<WorkspaceChange>) {
        let next_dirty = self.metadata != self.persisted_metadata
            || self.source_text != self.persisted_source_text;
        if next_dirty != self.is_dirty {
            changes.push(WorkspaceChange::DirtyStateChanged {
                is_dirty: next_dirty,
            });
        }

        self.revision = Revision(self.revision.0 + 1);
        self.is_dirty = next_dirty;
        self.change_log.push(WorkspaceChangeSet {
            revision: self.revision,
            changes,
        });
    }
}

/// Creates a new workspace and returns it in the opened clean state.
pub fn create_workspace(
    path: impl AsRef<Path>,
    options: CreateWorkspaceOptions,
) -> Result<Workspace, WorkspaceError> {
    let root = path.as_ref().to_path_buf();
    prepare_workspace_destination(&root)?;

    let paths = WorkspacePaths::new(root.clone());
    let metadata = WorkspaceMetadata::new(options.name, options.description)?;
    let source_text = String::new();

    ensure_workspace_directories(&paths)?;

    let metadata_text = serialize_metadata(&metadata)?;
    write_atomic_file(&paths.metadata_file(), metadata_text.as_bytes())?;
    write_atomic_file(&paths.source_file(), source_text.as_bytes())?;

    Ok(Workspace {
        root,
        paths,
        metadata: metadata.clone(),
        source_text: source_text.clone(),
        persisted_metadata: metadata,
        persisted_source_text: source_text,
        is_dirty: false,
        revision: Revision::ZERO,
        change_log: Vec::new(),
    })
}

/// Opens an existing workspace from disk.
pub fn open_workspace(path: impl AsRef<Path>) -> Result<Workspace, WorkspaceError> {
    let root = path.as_ref().to_path_buf();
    let paths = WorkspacePaths::new(root.clone());

    validate_workspace_root(&root)?;
    recover_workspace_files(&paths)?;
    validate_required_layout(&paths)?;

    let metadata_path = paths.metadata_file();
    let metadata_text = read_to_string(&metadata_path)?;
    let metadata: WorkspaceMetadata =
        toml::from_str(&metadata_text).map_err(|source| WorkspaceError::MalformedMetadata {
            path: metadata_path.clone(),
            source,
        })?;

    if metadata.format_version != WORKSPACE_FORMAT_VERSION {
        return Err(WorkspaceError::UnsupportedFormatVersion {
            path: metadata_path,
            found: metadata.format_version,
            supported: WORKSPACE_FORMAT_VERSION,
        });
    }

    let source_text = read_to_string(&paths.source_file())?;

    Ok(Workspace {
        root,
        paths,
        metadata: metadata.clone(),
        source_text: source_text.clone(),
        persisted_metadata: metadata,
        persisted_source_text: source_text,
        is_dirty: false,
        revision: Revision::ZERO,
        change_log: Vec::new(),
    })
}

fn normalize_name(name: String) -> Result<String, WorkspaceError> {
    let normalized = name.trim().to_owned();
    if normalized.is_empty() {
        return Err(WorkspaceError::InvalidMetadata {
            field: "name",
            message: "workspace name must not be empty".to_owned(),
        });
    }

    Ok(normalized)
}

fn normalize_description(description: Option<String>) -> Option<String> {
    description.and_then(|description| {
        let normalized = description.trim().to_owned();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn normalize_transaction_intent(intent: Option<String>) -> Option<String> {
    intent.and_then(|intent| {
        let normalized = intent.trim().to_owned();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn undo_intent(intent: Option<&str>, transaction_id: &TransactionId) -> String {
    match intent {
        Some(intent) => format!("Undo {intent} ({transaction_id})"),
        None => format!("Undo transaction {transaction_id}"),
    }
}

fn redo_intent(intent: Option<&str>, transaction_id: &TransactionId) -> String {
    match intent {
        Some(intent) => format!("Redo {intent} ({transaction_id})"),
        None => format!("Redo transaction {transaction_id}"),
    }
}

fn capture_inverse_operation(
    scene: &SceneDocument,
    operation: &WorkspaceOp,
) -> Result<WorkspaceOp, WorkspaceTransactionError> {
    match operation {
        WorkspaceOp::AddNode { node_id, .. } => Ok(WorkspaceOp::DeleteNode {
            id: OperationId::new(),
            node_id: node_id.clone(),
        }),
        WorkspaceOp::ReplaceNode { node, .. } => {
            if let Some(previous) = scene.nodes().get(node.id()) {
                Ok(WorkspaceOp::ReplaceNode {
                    id: OperationId::new(),
                    node: Box::new(previous.clone()),
                })
            } else {
                Ok(WorkspaceOp::DeleteNode {
                    id: OperationId::new(),
                    node_id: node.id().clone(),
                })
            }
        }
        WorkspaceOp::DeleteNode { node_id, .. } => {
            let previous = scene.nodes().get(node_id).ok_or_else(|| {
                WorkspaceTransactionError::NonInvertible {
                    message: format!("cannot delete missing node `{node_id}`"),
                }
            })?;
            Ok(WorkspaceOp::ReplaceNode {
                id: OperationId::new(),
                node: Box::new(previous.clone()),
            })
        }
        WorkspaceOp::RenameNode { from, to, .. } => Ok(WorkspaceOp::RenameNode {
            id: OperationId::new(),
            from: to.clone(),
            to: from.clone(),
        }),
        WorkspaceOp::DuplicateNode { duplicate, .. } => Ok(WorkspaceOp::DeleteNode {
            id: OperationId::new(),
            node_id: duplicate.clone(),
        }),
        WorkspaceOp::SetNodeLabel { node_id, .. } => {
            let previous = scene.nodes().get(node_id).ok_or_else(|| {
                WorkspaceTransactionError::NonInvertible {
                    message: format!("cannot update missing node `{node_id}`"),
                }
            })?;
            Ok(WorkspaceOp::SetNodeLabel {
                id: OperationId::new(),
                node_id: node_id.clone(),
                label: previous.label().map(ToOwned::to_owned),
            })
        }
        WorkspaceOp::SetCompositionChildren { node_id, .. } => {
            let previous = scene.nodes().get(node_id).ok_or_else(|| {
                WorkspaceTransactionError::NonInvertible {
                    message: format!("cannot update missing node `{node_id}`"),
                }
            })?;
            let (NodeKind::Union(composition)
            | NodeKind::Difference(composition)
            | NodeKind::Intersection(composition)) = previous.kind()
            else {
                return Err(WorkspaceTransactionError::NonInvertible {
                    message: format!("node `{node_id}` is not a composition node"),
                });
            };
            Ok(WorkspaceOp::SetCompositionChildren {
                id: OperationId::new(),
                node_id: node_id.clone(),
                children: composition
                    .children
                    .iter()
                    .map(|child| child.target().clone())
                    .collect(),
            })
        }
        WorkspaceOp::SetParameterScalar { parameter_id, .. } => {
            let previous = scene.parameters().get(parameter_id).ok_or_else(|| {
                WorkspaceTransactionError::NonInvertible {
                    message: format!("cannot update missing parameter `{parameter_id}`"),
                }
            })?;
            Ok(WorkspaceOp::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: parameter_id.clone(),
                value: previous.scalar_value(),
            })
        }
        WorkspaceOp::SetTransformComponent {
            node_id,
            property,
            axis,
            ..
        } => Ok(WorkspaceOp::SetTransformComponent {
            id: OperationId::new(),
            node_id: node_id.clone(),
            property: *property,
            axis: *axis,
            value: scalar_expr_literal(
                transform_component_expr(
                    scene.nodes().get(node_id).ok_or_else(|| {
                        WorkspaceTransactionError::NonInvertible {
                            message: format!("cannot update missing node `{node_id}`"),
                        }
                    })?,
                    *property,
                    *axis,
                ),
                node_id.as_str(),
            )?,
        }),
        WorkspaceOp::SetPrimitiveScalar { node_id, field, .. } => {
            Ok(WorkspaceOp::SetPrimitiveScalar {
                id: OperationId::new(),
                node_id: node_id.clone(),
                field: *field,
                value: scalar_expr_literal(
                    primitive_scalar_expr(
                        scene.nodes().get(node_id).ok_or_else(|| {
                            WorkspaceTransactionError::NonInvertible {
                                message: format!("cannot update missing node `{node_id}`"),
                            }
                        })?,
                        *field,
                    )?,
                    node_id.as_str(),
                )?,
            })
        }
        WorkspaceOp::SetRootNode { .. } => Ok(WorkspaceOp::SetRootNode {
            id: OperationId::new(),
            node_id: scene.root().clone(),
        }),
    }
}

fn scalar_expr_literal(
    expression: &ScalarExpr,
    context: &str,
) -> Result<f64, WorkspaceTransactionError> {
    match expression {
        ScalarExpr::Literal(value) => Ok(*value),
        ScalarExpr::Parameter(parameter) => Err(WorkspaceTransactionError::NonInvertible {
            message: format!(
                "cannot invert literal edit for `{context}` because it is parameter-driven by `{}`",
                parameter.target()
            ),
        }),
    }
}

fn transform_component_expr(node: &Node, property: TransformProperty, axis: Axis) -> &ScalarExpr {
    let vector = match property {
        TransformProperty::Translation => &node.transform().translation,
        TransformProperty::RotationDegrees => &node.transform().rotation_deg,
        TransformProperty::Scale => &node.transform().scale,
    };
    match axis {
        Axis::X => &vector.x,
        Axis::Y => &vector.y,
        Axis::Z => &vector.z,
    }
}

fn primitive_scalar_expr(
    node: &Node,
    field: PrimitiveScalarField,
) -> Result<&ScalarExpr, WorkspaceTransactionError> {
    match (node.kind(), field) {
        (NodeKind::Box(shape), PrimitiveScalarField::BoxX) => Ok(&shape.size.x),
        (NodeKind::Box(shape), PrimitiveScalarField::BoxY) => Ok(&shape.size.y),
        (NodeKind::Box(shape), PrimitiveScalarField::BoxZ) => Ok(&shape.size.z),
        (NodeKind::Sphere(shape), PrimitiveScalarField::SphereRadius) => Ok(&shape.radius),
        (NodeKind::Cylinder(shape), PrimitiveScalarField::CylinderRadius) => Ok(&shape.radius),
        (NodeKind::Cylinder(shape), PrimitiveScalarField::CylinderHeight) => Ok(&shape.height),
        (NodeKind::Capsule(shape), PrimitiveScalarField::CapsuleRadius) => Ok(&shape.radius),
        (NodeKind::Capsule(shape), PrimitiveScalarField::CapsuleHeight) => Ok(&shape.height),
        (NodeKind::Plane(shape), PrimitiveScalarField::PlaneWidth) => Ok(&shape.width),
        (NodeKind::Plane(shape), PrimitiveScalarField::PlaneDepth) => Ok(&shape.depth),
        (NodeKind::Profile(shape), PrimitiveScalarField::ProfileWidth) => Ok(&shape.width),
        (NodeKind::Profile(shape), PrimitiveScalarField::ProfileHeight) => Ok(&shape.height),
        _ => Err(WorkspaceTransactionError::NonInvertible {
            message: format!(
                "field `{field:?}` is not valid for node `{}` of kind `{}`",
                node.id(),
                node_kind_name(node.kind())
            ),
        }),
    }
}

fn node_kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Box(_) => "box",
        NodeKind::Sphere(_) => "sphere",
        NodeKind::Cylinder(_) => "cylinder",
        NodeKind::Capsule(_) => "capsule",
        NodeKind::Plane(_) => "plane",
        NodeKind::Profile(_) => "profile",
        NodeKind::Union(_) => "union",
        NodeKind::Difference(_) => "difference",
        NodeKind::Intersection(_) => "intersection",
    }
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if path.is_absolute() {
        return Err(WorkspaceError::UnsafePath {
            path: path.to_path_buf(),
            reason: "absolute paths are not allowed".to_owned(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(WorkspaceError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "parent traversal is not allowed".to_owned(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceError::UnsafePath {
                    path: path.to_path_buf(),
                    reason: "absolute or prefixed paths are not allowed".to_owned(),
                });
            }
        }
    }

    Ok(normalized)
}

fn prepare_workspace_destination(root: &Path) -> Result<(), WorkspaceError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(WorkspaceError::InvalidWorkspaceLayout {
                    path: root.to_path_buf(),
                    details: "workspace root exists but is not a directory".to_owned(),
                });
            }

            let mut entries = read_dir(root)?;
            if entries.next().is_some() {
                return Err(WorkspaceError::DestinationNotEmpty {
                    path: root.to_path_buf(),
                });
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|source| WorkspaceError::Io {
                path: root.to_path_buf(),
                operation: "create workspace root",
                source,
            })?;
        }
        Err(source) => {
            return Err(WorkspaceError::Io {
                path: root.to_path_buf(),
                operation: "inspect workspace destination",
                source,
            });
        }
    }

    Ok(())
}

fn validate_workspace_root(root: &Path) -> Result<(), WorkspaceError> {
    let metadata = fs::metadata(root).map_err(|source| WorkspaceError::Io {
        path: root.to_path_buf(),
        operation: "inspect workspace root",
        source,
    })?;

    if !metadata.is_dir() {
        return Err(WorkspaceError::InvalidWorkspaceLayout {
            path: root.to_path_buf(),
            details: "workspace root is not a directory".to_owned(),
        });
    }

    Ok(())
}

fn validate_required_layout(paths: &WorkspacePaths) -> Result<(), WorkspaceError> {
    for required_dir in [paths.state_dir(), paths.source_dir(), paths.exports_dir()] {
        let metadata = fs::metadata(&required_dir).map_err(|source| {
            WorkspaceError::InvalidWorkspaceLayout {
                path: required_dir.clone(),
                details: format!("required directory is missing or unreadable: {source}"),
            }
        })?;

        if !metadata.is_dir() {
            return Err(WorkspaceError::InvalidWorkspaceLayout {
                path: required_dir,
                details: "required workspace path is not a directory".to_owned(),
            });
        }
    }

    let metadata_file = paths.metadata_file();
    let metadata_kind =
        fs::metadata(&metadata_file).map_err(|source| WorkspaceError::InvalidWorkspaceLayout {
            path: metadata_file.clone(),
            details: format!("workspace metadata file is missing or unreadable: {source}"),
        })?;
    if !metadata_kind.is_file() {
        return Err(WorkspaceError::InvalidWorkspaceLayout {
            path: metadata_file,
            details: "workspace metadata path is not a file".to_owned(),
        });
    }

    let source_file = paths.source_file();
    let source_kind =
        fs::metadata(&source_file).map_err(|source| WorkspaceError::InvalidWorkspaceLayout {
            path: source_file.clone(),
            details: format!("workspace source file is missing or unreadable: {source}"),
        })?;
    if !source_kind.is_file() {
        return Err(WorkspaceError::InvalidWorkspaceLayout {
            path: source_file,
            details: "workspace source path is not a file".to_owned(),
        });
    }

    Ok(())
}

fn ensure_workspace_directories(paths: &WorkspacePaths) -> Result<(), WorkspaceError> {
    for directory in [
        paths.state_dir(),
        paths.source_dir(),
        paths.exports_dir(),
        paths.cache_dir(),
        paths.history_dir(),
        paths.ai_dir(),
    ] {
        fs::create_dir_all(&directory).map_err(|source| WorkspaceError::Io {
            path: directory,
            operation: "create workspace directory",
            source,
        })?;
    }

    Ok(())
}

fn serialize_metadata(metadata: &WorkspaceMetadata) -> Result<String, WorkspaceError> {
    toml::to_string_pretty(metadata).map_err(|source| WorkspaceError::InvalidMetadata {
        field: "metadata",
        message: source.to_string(),
    })
}

fn read_to_string(path: &Path) -> Result<String, WorkspaceError> {
    fs::read_to_string(path).map_err(|source| WorkspaceError::Io {
        path: path.to_path_buf(),
        operation: "read file",
        source,
    })
}

fn write_atomic_file(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorkspaceError::PersistenceFailed {
            path: path.to_path_buf(),
            operation: "resolve parent directory",
            source: io::Error::other("canonical file has no parent directory"),
        })?;

    fs::create_dir_all(parent).map_err(|source| WorkspaceError::PersistenceFailed {
        path: parent.to_path_buf(),
        operation: "create parent directories",
        source,
    })?;

    let temp_path = temp_path_for(path);
    let backup_path = backup_path_for(path);

    fs::write(&temp_path, bytes).map_err(|source| WorkspaceError::PersistenceFailed {
        path: temp_path.clone(),
        operation: "write temporary file",
        source,
    })?;

    if path.exists() {
        remove_if_exists(&backup_path, "remove stale backup")?;
        fs::rename(path, &backup_path).map_err(|source| WorkspaceError::PersistenceFailed {
            path: path.to_path_buf(),
            operation: "move canonical file to backup",
            source,
        })?;
    }

    maybe_fail_atomic_write(path, AtomicWriteFailStage::BeforePromote)?;

    match fs::rename(&temp_path, path) {
        Ok(()) => {
            remove_if_exists(&backup_path, "remove backup file")?;
            Ok(())
        }
        Err(source) => {
            if backup_path.exists() && !path.exists() {
                let _ = fs::rename(&backup_path, path);
            }

            let _ = fs::remove_file(&temp_path);
            Err(WorkspaceError::PersistenceFailed {
                path: path.to_path_buf(),
                operation: "promote temporary file",
                source,
            })
        }
    }
}

fn recover_workspace_files(paths: &WorkspacePaths) -> Result<(), WorkspaceError> {
    recover_file(&paths.metadata_file(), validate_metadata_recovery_candidate)?;
    recover_file(&paths.source_file(), |_| Ok(()))?;
    Ok(())
}

fn recover_file(
    canonical_path: &Path,
    validator: impl Fn(&Path) -> Result<(), WorkspaceError>,
) -> Result<(), WorkspaceError> {
    let temp_path = temp_path_for(canonical_path);
    let backup_path = backup_path_for(canonical_path);

    if canonical_path.exists() {
        remove_if_exists(&temp_path, "remove stale temporary file")?;
        remove_if_exists(&backup_path, "remove stale backup file")?;
        return Ok(());
    }

    if backup_path.exists() {
        validator(&backup_path)?;
        fs::rename(&backup_path, canonical_path).map_err(|source| {
            WorkspaceError::RecoveryFailed {
                path: canonical_path.to_path_buf(),
                details: format!("failed to restore backup file: {source}"),
            }
        })?;
        remove_if_exists(&temp_path, "remove unrecoverable temporary file")?;
        return Ok(());
    }

    if temp_path.exists() && validator(&temp_path).is_ok() {
        fs::rename(&temp_path, canonical_path).map_err(|source| {
            WorkspaceError::RecoveryFailed {
                path: canonical_path.to_path_buf(),
                details: format!("failed to promote temporary file: {source}"),
            }
        })?;
        remove_if_exists(&backup_path, "remove superseded backup file")?;
        return Ok(());
    }

    if temp_path.exists() {
        return Err(WorkspaceError::RecoveryFailed {
            path: canonical_path.to_path_buf(),
            details:
                "temporary file existed but could not be validated, and no backup was available"
                    .to_owned(),
        });
    }

    Ok(())
}

fn validate_metadata_recovery_candidate(path: &Path) -> Result<(), WorkspaceError> {
    let text = read_to_string(path)?;
    let _: WorkspaceMetadata =
        toml::from_str(&text).map_err(|source| WorkspaceError::MalformedMetadata {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    suffixed_path(path, TEMP_SUFFIX)
}

fn backup_path_for(path: &Path) -> PathBuf {
    suffixed_path(path, BACKUP_SUFFIX)
}

fn suffixed_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn remove_if_exists(path: &Path, operation: &'static str) -> Result<(), WorkspaceError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(WorkspaceError::Io {
            path: path.to_path_buf(),
            operation,
            source,
        }),
    }
}

fn read_dir(path: &Path) -> Result<fs::ReadDir, WorkspaceError> {
    fs::read_dir(path).map_err(|source| WorkspaceError::Io {
        path: path.to_path_buf(),
        operation: "read directory",
        source,
    })
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicWriteFailStage {
    BeforePromote,
}

#[cfg(not(test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicWriteFailStage {
    BeforePromote,
}

#[cfg(test)]
static ATOMIC_WRITE_FAILPOINT: OnceLock<Mutex<Option<(PathBuf, AtomicWriteFailStage)>>> =
    OnceLock::new();

#[cfg(test)]
fn maybe_fail_atomic_write(path: &Path, stage: AtomicWriteFailStage) -> Result<(), WorkspaceError> {
    let failpoint = ATOMIC_WRITE_FAILPOINT.get_or_init(|| Mutex::new(None));
    let guard = failpoint
        .lock()
        .expect("atomic write failpoint mutex poisoned");
    if let Some((expected_path, expected_stage)) = guard.as_ref()
        && expected_path == path
        && *expected_stage == stage
    {
        return Err(WorkspaceError::PersistenceFailed {
            path: path.to_path_buf(),
            operation: "promote temporary file",
            source: io::Error::other("injected atomic write failure"),
        });
    }

    Ok(())
}

#[cfg(not(test))]
fn maybe_fail_atomic_write(
    _path: &Path,
    _stage: AtomicWriteFailStage,
) -> Result<(), WorkspaceError> {
    Ok(())
}

#[cfg(test)]
fn install_atomic_write_failpoint(path: &Path, stage: AtomicWriteFailStage) {
    let failpoint = ATOMIC_WRITE_FAILPOINT.get_or_init(|| Mutex::new(None));
    *failpoint
        .lock()
        .expect("atomic write failpoint mutex poisoned") = Some((path.to_path_buf(), stage));
}

#[cfg(test)]
fn clear_atomic_write_failpoint() {
    let failpoint = ATOMIC_WRITE_FAILPOINT.get_or_init(|| Mutex::new(None));
    *failpoint
        .lock()
        .expect("atomic write failpoint mutex poisoned") = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TRANSACTION_SOURCE: &str = r#"
schema_version = 1
root = "root"

[params.width]
type = "scalar"
value = 3.0

[nodes.body]
kind = "box"
label = "Body"
size = { x = { param = "width" }, y = 2.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.cap]
kind = "sphere"
radius = 0.5
transform = { translate = { x = 0.0, y = 1.5, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.spare]
kind = "sphere"
radius = 0.25
transform = { translate = { x = 2.0, y = 0.5, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.root]
kind = "union"
children = ["body", "cap"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
"#;

    fn transaction_workspace() -> Workspace {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("transaction-workspace");
        std::mem::forget(temp_dir);
        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Transaction Workspace"),
        )
        .expect("create workspace");
        workspace.replace_source(TRANSACTION_SOURCE);
        workspace.save().expect("save source");
        Workspace::open(&workspace_root).expect("reopen workspace")
    }

    #[test]
    fn create_save_reopen_round_trip_preserves_identity_and_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("round-trip");

        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions {
                name: "Round Trip".to_owned(),
                description: Some("Initial description".to_owned()),
            },
        )
        .expect("create workspace");

        assert_eq!(workspace.revision(), Revision::ZERO);
        assert!(!workspace.is_dirty());
        assert_eq!(workspace.summary().name(), "Round Trip");
        assert_eq!(
            workspace.metadata().description(),
            Some("Initial description")
        );
        assert_eq!(
            workspace
                .changes_since(Revision::ZERO)
                .expect("changes")
                .len(),
            0
        );

        let original_id = workspace.metadata().workspace_id().clone();

        workspace.set_name("Round Trip Updated").expect("set name");
        assert!(workspace.set_description(Some("Updated description".to_owned())));
        assert!(workspace.replace_source("# opaque scene source\n"));
        assert!(workspace.is_dirty());

        workspace.save().expect("save workspace");
        assert!(!workspace.is_dirty());

        let reopened = open_workspace(&workspace_root).expect("reopen workspace");
        assert_eq!(reopened.metadata().workspace_id(), &original_id);
        assert_eq!(reopened.metadata().name(), "Round Trip Updated");
        assert_eq!(
            reopened.metadata().description(),
            Some("Updated description")
        );
        assert_eq!(reopened.source_text(), "# opaque scene source\n");
        assert_eq!(reopened.revision(), Revision::ZERO);
        assert!(!reopened.is_dirty());
    }

    #[test]
    fn opening_unsupported_version_returns_typed_error() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("versioned");

        let workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Versioned Workspace"),
        )
        .expect("create workspace");

        let metadata_path = workspace.paths().metadata_file();
        let unsupported = format!(
            "format_version = 99\nworkspace_id = \"{}\"\nname = \"Versioned Workspace\"\n",
            workspace.metadata().workspace_id()
        );
        fs::write(&metadata_path, unsupported).expect("write unsupported metadata");

        let error = open_workspace(&workspace_root).expect_err("unsupported version should fail");
        match error {
            WorkspaceError::UnsupportedFormatVersion {
                found, supported, ..
            } => {
                assert_eq!(found, 99);
                assert_eq!(supported, WORKSPACE_FORMAT_VERSION);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn creation_refuses_non_empty_destination_without_overwriting_user_data() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("occupied");
        fs::create_dir_all(&workspace_root).expect("create root");
        let unrelated_file = workspace_root.join("notes.txt");
        fs::write(&unrelated_file, "do not overwrite").expect("write unrelated data");

        let error = create_workspace(&workspace_root, CreateWorkspaceOptions::new("Occupied"))
            .expect_err("non-empty destination should fail");
        match error {
            WorkspaceError::DestinationNotEmpty { path } => assert_eq!(path, workspace_root),
            other => panic!("unexpected error: {other:?}"),
        }

        assert_eq!(
            fs::read_to_string(unrelated_file).expect("read unrelated data"),
            "do not overwrite"
        );
    }

    #[test]
    fn cache_deletion_does_not_invalidate_workspace_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("cache-safe");

        let workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Cache Safe Workspace"),
        )
        .expect("create workspace");

        fs::write(workspace.paths().cache_dir().join("preview.bin"), b"cache")
            .expect("write cache file");
        fs::remove_dir_all(workspace.paths().cache_dir()).expect("remove cache directory");

        let mut reopened = open_workspace(&workspace_root).expect("open without cache");
        assert_eq!(reopened.metadata().name(), "Cache Safe Workspace");
        assert!(reopened.replace_source("source after cache deletion"));
        reopened
            .save()
            .expect("save recreates reserved directories");
        assert!(reopened.paths().cache_dir().is_dir());
    }

    #[test]
    fn dirty_revision_and_changes_since_behave_deterministically() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("changes");

        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Change Tracker"),
        )
        .expect("create workspace");

        assert!(!workspace.set_description(None));
        assert_eq!(workspace.revision(), Revision::ZERO);

        assert!(workspace.replace_source("v1"));
        assert!(workspace.is_dirty());
        assert_eq!(workspace.revision().get(), 1);

        let changes = workspace
            .changes_since(Revision::ZERO)
            .expect("changes since zero");
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].changes(),
            &[
                WorkspaceChange::SceneReplaced,
                WorkspaceChange::DirtyStateChanged { is_dirty: true }
            ]
        );

        assert!(!workspace.replace_source("v1"));
        assert_eq!(workspace.revision().get(), 1);

        workspace.save().expect("save");
        assert!(!workspace.is_dirty());
        assert_eq!(workspace.revision().get(), 2);

        let save_changes = workspace
            .changes_since(Revision(1))
            .expect("changes after save");
        assert_eq!(save_changes.len(), 1);
        assert_eq!(
            save_changes[0].changes(),
            &[WorkspaceChange::DirtyStateChanged { is_dirty: false }]
        );

        assert!(
            workspace
                .changes_since(workspace.revision())
                .expect("current revision")
                .is_empty()
        );
        match workspace
            .changes_since(Revision(99))
            .expect_err("future revision should fail")
        {
            WorkspaceError::InvalidRevision { requested, current } => {
                assert_eq!(requested, Revision(99));
                assert_eq!(current, workspace.revision());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn path_resolution_rejects_traversal_and_absolute_paths() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("paths");

        let workspace = create_workspace(&workspace_root, CreateWorkspaceOptions::new("Path Safe"))
            .expect("create workspace");

        let resolved = workspace
            .resolve_path(WorkspaceDirectory::Exports, Path::new("renders/chair.glb"))
            .expect("resolve normal path");
        assert_eq!(
            resolved,
            workspace
                .paths()
                .exports_dir()
                .join("renders")
                .join("chair.glb")
        );

        match workspace
            .resolve_path(WorkspaceDirectory::Source, Path::new("../escape.toml"))
            .expect_err("traversal should fail")
        {
            WorkspaceError::UnsafePath { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }

        let absolute = if cfg!(windows) {
            PathBuf::from(r"C:\escape.toml")
        } else {
            PathBuf::from("/escape.toml")
        };

        match workspace
            .resolve_path(WorkspaceDirectory::Source, absolute)
            .expect_err("absolute path should fail")
        {
            WorkspaceError::UnsafePath { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn save_failure_keeps_last_known_good_metadata_and_dirty_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("save-failure");

        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Original Name"),
        )
        .expect("create workspace");
        let metadata_path = workspace.paths().metadata_file();

        workspace.set_name("Mutated Name").expect("set name");
        install_atomic_write_failpoint(&metadata_path, AtomicWriteFailStage::BeforePromote);
        let save_error = workspace.save().expect_err("save should fail");
        clear_atomic_write_failpoint();

        match save_error {
            WorkspaceError::PersistenceFailed { path, .. } => assert_eq!(path, metadata_path),
            other => panic!("unexpected error: {other:?}"),
        }

        assert!(workspace.is_dirty());

        let reopened = open_workspace(&workspace_root).expect("reopen after failed save");
        assert_eq!(reopened.metadata().name(), "Original Name");
    }

    #[test]
    fn opening_recovers_from_interrupted_metadata_backup_and_temp_source() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("recovery");

        let workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Recoverable Workspace"),
        )
        .expect("create workspace");

        let metadata_path = workspace.paths().metadata_file();
        let metadata_backup = backup_path_for(&metadata_path);
        fs::copy(&metadata_path, &metadata_backup).expect("copy metadata backup");
        fs::remove_file(&metadata_path).expect("remove canonical metadata");

        let source_path = workspace.paths().source_file();
        let source_temp = temp_path_for(&source_path);
        fs::write(&source_temp, "recovered source").expect("write temp source");
        fs::remove_file(&source_path).expect("remove canonical source");

        let reopened = open_workspace(&workspace_root).expect("open with recovery");
        assert_eq!(reopened.metadata().name(), "Recoverable Workspace");
        assert_eq!(reopened.source_text(), "recovered source");
        assert!(reopened.paths().metadata_file().is_file());
        assert!(reopened.paths().source_file().is_file());
        assert!(!backup_path_for(&metadata_path).exists());
        assert!(!temp_path_for(&source_path).exists());
    }

    #[test]
    fn reload_source_emits_a_reload_change() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("reload");

        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Reloadable Workspace"),
        )
        .expect("create workspace");

        fs::write(workspace.paths().source_file(), "externally changed").expect("write source");
        assert!(workspace.reload_source().expect("reload source"));
        assert_eq!(workspace.source_text(), "externally changed");
        assert_eq!(workspace.revision().get(), 1);

        let changes = workspace.changes_since(Revision::ZERO).expect("changes");
        assert_eq!(
            changes[0].changes(),
            &[
                WorkspaceChange::SourceReloaded,
                WorkspaceChange::DirtyStateChanged { is_dirty: true }
            ]
        );
    }

    #[test]
    fn successful_save_cleans_up_atomic_write_artifacts() {
        let temp_dir = TempDir::new().expect("temp dir");
        let workspace_root = temp_dir.path().join("atomic-cleanup");

        let mut workspace = create_workspace(
            &workspace_root,
            CreateWorkspaceOptions::new("Artifact Cleanup"),
        )
        .expect("create workspace");

        workspace
            .set_name("Artifact Cleanup Updated")
            .expect("set name");
        workspace.replace_source("source");
        workspace.save().expect("save");

        assert!(!temp_path_for(&workspace.paths().metadata_file()).exists());
        assert!(!backup_path_for(&workspace.paths().metadata_file()).exists());
        assert!(!temp_path_for(&workspace.paths().source_file()).exists());
        assert!(!backup_path_for(&workspace.paths().source_file()).exists());
    }

    #[test]
    fn transaction_applies_multiple_operations_and_reports_actor_intent_and_targets() {
        let mut workspace = transaction_workspace();
        let body = NodeId::new("body").expect("body");
        let width = ParamId::new("width").expect("width");
        let transaction = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Broaden body and move it".to_owned()),
            vec![
                WorkspaceOp::SetParameterScalar {
                    id: OperationId::new(),
                    parameter_id: width.clone(),
                    value: 4.25,
                },
                WorkspaceOp::SetTransformComponent {
                    id: OperationId::new(),
                    node_id: body.clone(),
                    property: TransformProperty::Translation,
                    axis: Axis::X,
                    value: 1.5,
                },
            ],
        )
        .expect("transaction");

        let commit = workspace
            .apply_transaction(&transaction)
            .expect("apply transaction");
        assert_eq!(commit.actor(), TransactionActor::User);
        assert_eq!(commit.intent(), Some("Broaden body and move it"));
        assert_eq!(commit.operation_ids().len(), 2);
        assert!(commit.affected_targets().node_ids().contains(&body));
        assert!(commit.affected_targets().parameter_ids().contains(&width));
        assert!(commit.revision_after().get() > commit.revision_before().get());

        let reopened = Workspace::open(workspace.root()).expect("reopen");
        let scene = geom_scene::parse_scene(reopened.source_text()).expect("parse scene");
        assert_eq!(scene.parameters()[&width].scalar_value(), 4.25);
        match &scene.nodes()[&body].transform().translation.x {
            geom_scene::ScalarExpr::Literal(value) => assert_eq!(*value, 1.5),
            other => panic!("unexpected translation value: {other:?}"),
        }
    }

    #[test]
    fn invalid_transaction_fails_atomically_without_partial_source_mutation() {
        let mut workspace = transaction_workspace();
        let original_source = workspace.source_text().to_owned();
        let transaction = WorkspaceTransaction::new(
            TransactionActor::CliAutomation,
            Some("Partial failure should not persist".to_owned()),
            vec![
                WorkspaceOp::SetParameterScalar {
                    id: OperationId::new(),
                    parameter_id: ParamId::new("width").expect("width"),
                    value: 9.0,
                },
                WorkspaceOp::RenameNode {
                    id: OperationId::new(),
                    from: NodeId::new("body").expect("body"),
                    to: NodeId::new("root").expect("root"),
                },
            ],
        )
        .expect("transaction");

        let error = workspace
            .apply_transaction(&transaction)
            .expect_err("transaction should fail");
        assert!(matches!(
            error,
            WorkspaceTransactionError::SceneValidation { .. }
        ));
        assert_eq!(workspace.source_text(), original_source);

        let reopened = Workspace::open(workspace.root()).expect("reopen");
        assert_eq!(reopened.source_text(), original_source);
        let scene = geom_scene::parse_scene(reopened.source_text()).expect("parse scene");
        assert_eq!(
            scene.parameters()[&ParamId::new("width").expect("width")].scalar_value(),
            3.0
        );
        assert!(
            scene
                .nodes()
                .contains_key(&NodeId::new("body").expect("body"))
        );
    }

    #[test]
    fn rename_transaction_updates_references_coherently() {
        let mut workspace = transaction_workspace();
        let transaction = WorkspaceTransaction::new(
            TransactionActor::Ai,
            Some("Rename cap to top".to_owned()),
            vec![WorkspaceOp::RenameNode {
                id: OperationId::new(),
                from: NodeId::new("cap").expect("cap"),
                to: NodeId::new("top").expect("top"),
            }],
        )
        .expect("transaction");

        workspace
            .apply_transaction(&transaction)
            .expect("apply rename");
        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert!(
            scene
                .nodes()
                .contains_key(&NodeId::new("top").expect("top"))
        );
        assert!(
            !scene
                .nodes()
                .contains_key(&NodeId::new("cap").expect("cap"))
        );
        match scene.nodes()[&NodeId::new("root").expect("root")].kind() {
            geom_scene::NodeKind::Union(composition) => {
                assert_eq!(composition.children[1].target().as_str(), "top");
            }
            other => panic!("unexpected root kind: {other:?}"),
        }
    }

    #[test]
    fn empty_transaction_is_rejected() {
        let error = WorkspaceTransaction::new(TransactionActor::SystemMigration, None, vec![])
            .expect_err("empty transaction should fail");
        assert!(matches!(error, WorkspaceTransactionError::EmptyTransaction));
    }

    #[test]
    fn single_op_undo_redo_round_trips_parameter_and_rename() {
        let mut workspace = transaction_workspace();
        let mut history = UndoRedoManager::default();

        let set_parameter = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Tune width".to_owned()),
            vec![WorkspaceOp::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: ParamId::new("width").expect("width"),
                value: 5.0,
            }],
        )
        .expect("transaction");
        let parameter_commit = workspace
            .apply_transaction(&set_parameter)
            .expect("apply parameter");
        history.record_commit(&parameter_commit);

        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert_eq!(
            scene.parameters()[&ParamId::new("width").expect("width")].scalar_value(),
            5.0
        );

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo")
            .expect("undo commit");
        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert_eq!(
            scene.parameters()[&ParamId::new("width").expect("width")].scalar_value(),
            3.0
        );

        history
            .redo(&mut workspace, TransactionActor::User)
            .expect("redo")
            .expect("redo commit");
        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert_eq!(
            scene.parameters()[&ParamId::new("width").expect("width")].scalar_value(),
            5.0
        );

        let rename = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Rename cap".to_owned()),
            vec![WorkspaceOp::RenameNode {
                id: OperationId::new(),
                from: NodeId::new("cap").expect("cap"),
                to: NodeId::new("top").expect("top"),
            }],
        )
        .expect("transaction");
        let rename_commit = workspace.apply_transaction(&rename).expect("apply rename");
        history.record_commit(&rename_commit);

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo rename")
            .expect("undo commit");
        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert!(
            scene
                .nodes()
                .contains_key(&NodeId::new("cap").expect("cap"))
        );
        assert!(
            !scene
                .nodes()
                .contains_key(&NodeId::new("top").expect("top"))
        );

        history
            .redo(&mut workspace, TransactionActor::User)
            .expect("redo rename")
            .expect("redo commit");
        let scene = geom_scene::parse_scene(workspace.source_text()).expect("parse scene");
        assert!(
            scene
                .nodes()
                .contains_key(&NodeId::new("top").expect("top"))
        );
        assert!(
            !scene
                .nodes()
                .contains_key(&NodeId::new("cap").expect("cap"))
        );
    }

    #[test]
    fn multi_operation_transaction_undo_and_redo_restore_exact_states() {
        let mut workspace = transaction_workspace();
        let before = geom_scene::parse_scene(workspace.source_text()).expect("before");
        let mut history = UndoRedoManager::default();

        let transaction = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Rename and retune".to_owned()),
            vec![
                WorkspaceOp::RenameNode {
                    id: OperationId::new(),
                    from: NodeId::new("cap").expect("cap"),
                    to: NodeId::new("top").expect("top"),
                },
                WorkspaceOp::SetTransformComponent {
                    id: OperationId::new(),
                    node_id: NodeId::new("top").expect("top"),
                    property: TransformProperty::Translation,
                    axis: Axis::Y,
                    value: 2.0,
                },
                WorkspaceOp::SetCompositionChildren {
                    id: OperationId::new(),
                    node_id: NodeId::new("root").expect("root"),
                    children: vec![
                        NodeId::new("top").expect("top"),
                        NodeId::new("body").expect("body"),
                    ],
                },
            ],
        )
        .expect("transaction");
        let commit = workspace
            .apply_transaction(&transaction)
            .expect("apply transaction");
        history.record_commit(&commit);
        let committed = geom_scene::parse_scene(workspace.source_text()).expect("committed");

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo")
            .expect("undo commit");
        let undone = geom_scene::parse_scene(workspace.source_text()).expect("undone");
        assert_eq!(undone, before);

        history
            .redo(&mut workspace, TransactionActor::User)
            .expect("redo")
            .expect("redo commit");
        let redone = geom_scene::parse_scene(workspace.source_text()).expect("redone");
        assert_eq!(redone, committed);
    }

    #[test]
    fn add_delete_undo_redo_restore_exact_nodes() {
        let mut workspace = transaction_workspace();
        let mut history = UndoRedoManager::default();

        let add = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Add spare sphere".to_owned()),
            vec![WorkspaceOp::AddNode {
                id: OperationId::new(),
                node_id: NodeId::new("extra").expect("extra"),
                draft: SceneNodeDraft::Sphere,
            }],
        )
        .expect("transaction");
        let add_commit = workspace.apply_transaction(&add).expect("apply add");
        history.record_commit(&add_commit);
        assert!(
            geom_scene::parse_scene(workspace.source_text())
                .expect("parse")
                .nodes()
                .contains_key(&NodeId::new("extra").expect("extra"))
        );

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo add")
            .expect("undo commit");
        assert!(
            !geom_scene::parse_scene(workspace.source_text())
                .expect("parse")
                .nodes()
                .contains_key(&NodeId::new("extra").expect("extra"))
        );

        history
            .redo(&mut workspace, TransactionActor::User)
            .expect("redo add")
            .expect("redo commit");
        assert!(
            geom_scene::parse_scene(workspace.source_text())
                .expect("parse")
                .nodes()
                .contains_key(&NodeId::new("extra").expect("extra"))
        );

        let deleted_before = geom_scene::parse_scene(workspace.source_text())
            .expect("parse")
            .nodes()[&NodeId::new("spare").expect("spare")]
            .clone();
        let delete = WorkspaceTransaction::new(
            TransactionActor::User,
            Some("Delete spare".to_owned()),
            vec![WorkspaceOp::DeleteNode {
                id: OperationId::new(),
                node_id: NodeId::new("spare").expect("spare"),
            }],
        )
        .expect("transaction");
        let delete_commit = workspace.apply_transaction(&delete).expect("apply delete");
        history.record_commit(&delete_commit);

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo delete")
            .expect("undo commit");
        let restored = geom_scene::parse_scene(workspace.source_text()).expect("parse");
        assert_eq!(
            restored.nodes()[&NodeId::new("spare").expect("spare")],
            deleted_before
        );

        history
            .redo(&mut workspace, TransactionActor::User)
            .expect("redo delete")
            .expect("redo commit");
        assert!(
            !geom_scene::parse_scene(workspace.source_text())
                .expect("parse")
                .nodes()
                .contains_key(&NodeId::new("spare").expect("spare"))
        );
    }

    #[test]
    fn redo_is_cleared_by_new_commit_after_undo() {
        let mut workspace = transaction_workspace();
        let mut history = UndoRedoManager::default();

        let commit_a = workspace
            .apply_transaction(
                &WorkspaceTransaction::new(
                    TransactionActor::User,
                    Some("A".to_owned()),
                    vec![WorkspaceOp::SetParameterScalar {
                        id: OperationId::new(),
                        parameter_id: ParamId::new("width").expect("width"),
                        value: 4.0,
                    }],
                )
                .expect("transaction"),
            )
            .expect("apply A");
        history.record_commit(&commit_a);

        let commit_b = workspace
            .apply_transaction(
                &WorkspaceTransaction::new(
                    TransactionActor::User,
                    Some("B".to_owned()),
                    vec![WorkspaceOp::SetTransformComponent {
                        id: OperationId::new(),
                        node_id: NodeId::new("body").expect("body"),
                        property: TransformProperty::Translation,
                        axis: Axis::X,
                        value: 2.0,
                    }],
                )
                .expect("transaction"),
            )
            .expect("apply B");
        history.record_commit(&commit_b);

        history
            .undo(&mut workspace, TransactionActor::User)
            .expect("undo B")
            .expect("undo commit");
        assert!(history.availability().can_redo);

        let commit_c = workspace
            .apply_transaction(
                &WorkspaceTransaction::new(
                    TransactionActor::User,
                    Some("C".to_owned()),
                    vec![WorkspaceOp::SetNodeLabel {
                        id: OperationId::new(),
                        node_id: NodeId::new("body").expect("body"),
                        label: Some("Retagged".to_owned()),
                    }],
                )
                .expect("transaction"),
            )
            .expect("apply C");
        history.record_commit(&commit_c);

        assert!(!history.availability().can_redo);
    }
}
