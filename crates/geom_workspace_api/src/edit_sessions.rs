use crate::{
    BoundedList, CollectionBounds, RebuildScopeView, SceneDiffView, TransactionProposal,
    WorkspaceApi, WorkspaceApiError, WorkspaceReadContext, dry_run_diff,
    workspace_transaction_error,
};
use geom_diagnostics::Diagnostic;
use geom_workspace::{TransactionActor, TransactionCorrelation, Workspace, WorkspaceDirectory};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const AI_EDIT_SESSION_FORMAT_VERSION: u32 = 1;
const AI_EDIT_SESSIONS_DIR: &str = "sessions";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiEditSessionId(String);

impl AiEditSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AiEditSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AiEditSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiProposalId(String);

impl AiProposalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AiProposalId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AiProposalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEditSessionPolicy {
    ProposeOnly,
    AutoApply,
    FailOnApprovalRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEditSessionStatus {
    Open,
    AwaitingReview,
    Applying,
    Completed,
    Cancelled,
    Failed,
    Reverted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProposalState {
    Proposed,
    Accepted,
    Rejected,
    Applied,
    Failed,
    Stale,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProposalRevalidation {
    NotNeeded,
    StillValid,
    StaleButRevalidated,
    Conflicts,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEditSessionOutcomeKind {
    Completed,
    Cancelled,
    Failed,
    Reverted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEditSessionOutcome {
    pub kind: AiEditSessionOutcomeKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiRestorePoint {
    pub snapshot_id: String,
    pub created_from_revision: u64,
    pub created_at_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedAiChange {
    pub proposal_id: AiProposalId,
    pub transaction_id: String,
    pub revision_before: u64,
    pub revision_after: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiEditProposal {
    pub proposal_id: AiProposalId,
    pub session_id: AiEditSessionId,
    pub base_revision: u64,
    pub rationale: Option<String>,
    pub proposal: TransactionProposal,
    pub diff: Option<SceneDiffView>,
    pub diagnostics: Vec<Diagnostic>,
    pub affected_node_ids: Vec<String>,
    pub affected_parameter_ids: Vec<String>,
    pub expected_rebuild_scope: RebuildScopeView,
    pub state: AiProposalState,
    pub revalidation: Option<AiProposalRevalidation>,
    pub transaction_id: Option<String>,
    pub resulting_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiEditSessionEventKind {
    SessionStarted,
    ProposalAdded,
    ProposalAccepted,
    ProposalRejected,
    ProposalFailed,
    ProposalStale,
    TransactionApplied,
    SessionCancelled,
    SessionCompleted,
    SessionReverted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiEditSessionEvent {
    pub sequence: u64,
    pub session_id: AiEditSessionId,
    pub proposal_id: Option<AiProposalId>,
    pub workspace_revision: Option<u64>,
    pub kind: AiEditSessionEventKind,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiEditSession {
    pub session_id: AiEditSessionId,
    pub user_intent: String,
    pub base_revision: u64,
    pub created_at_millis: u64,
    pub policy: AiEditSessionPolicy,
    pub status: AiEditSessionStatus,
    pub proposals: Vec<AiEditProposal>,
    pub accepted_proposal_ids: Vec<AiProposalId>,
    pub rejected_proposal_ids: Vec<AiProposalId>,
    pub applied_changes: Vec<AppliedAiChange>,
    pub failed_proposal_ids: Vec<AiProposalId>,
    pub diagnostics: Vec<Diagnostic>,
    pub restore_point: Option<AiRestorePoint>,
    pub final_outcome: Option<AiEditSessionOutcome>,
    pub events: Vec<AiEditSessionEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartAiEditSessionRequest {
    pub user_intent: String,
    pub policy: AiEditSessionPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitAiProposalRequest {
    pub rationale: Option<String>,
    pub proposal: TransactionProposal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEventQuery {
    pub session_id: String,
    pub after_sequence: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProposalQuery {
    pub session_id: String,
    pub proposal_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionQuery {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PersistedAiEditSession {
    format_version: u32,
    session: AiEditSession,
}

pub(crate) fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn ai_edit_sessions_dir(workspace: &Workspace) -> Result<PathBuf, WorkspaceApiError> {
    workspace
        .resolve_path(WorkspaceDirectory::AiData, AI_EDIT_SESSIONS_DIR)
        .map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })
}

pub(crate) fn ai_edit_session_path(
    workspace: &Workspace,
    session_id: &AiEditSessionId,
) -> Result<PathBuf, WorkspaceApiError> {
    Ok(ai_edit_sessions_dir(workspace)?.join(format!("{}.json", session_id.as_str())))
}

pub(crate) fn load_ai_edit_sessions(
    workspace: &Workspace,
) -> Result<Vec<AiEditSession>, WorkspaceApiError> {
    let dir = ai_edit_sessions_dir(workspace)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        sessions.push(load_ai_edit_session_from_path(&path)?);
    }
    sessions.sort_by_key(|session| (session.created_at_millis, session.session_id.clone()));
    Ok(sessions)
}

pub(crate) fn load_ai_edit_session(
    workspace: &Workspace,
    session_id: &AiEditSessionId,
) -> Result<AiEditSession, WorkspaceApiError> {
    let path = ai_edit_session_path(workspace, session_id)?;
    load_ai_edit_session_from_path(&path)
}

fn load_ai_edit_session_from_path(path: &Path) -> Result<AiEditSession, WorkspaceApiError> {
    let text = fs::read_to_string(path).map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })?;
    let persisted: PersistedAiEditSession =
        serde_json::from_str(&text).map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    if persisted.format_version != AI_EDIT_SESSION_FORMAT_VERSION {
        return Err(WorkspaceApiError::Workspace {
            message: format!(
                "unsupported ai edit session format version {} at `{}`",
                persisted.format_version,
                path.display()
            ),
        });
    }
    Ok(persisted.session)
}

pub(crate) fn save_ai_edit_session(
    workspace: &Workspace,
    session: &AiEditSession,
) -> Result<(), WorkspaceApiError> {
    let path = ai_edit_session_path(workspace, &session.session_id)?;
    let parent = path.parent().ok_or_else(|| WorkspaceApiError::Workspace {
        message: "ai edit session path has no parent".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })?;
    let payload = PersistedAiEditSession {
        format_version: AI_EDIT_SESSION_FORMAT_VERSION,
        session: session.clone(),
    };
    let text =
        serde_json::to_string_pretty(&payload).map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    fs::write(&path, text).map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })
}

pub(crate) fn push_session_event(
    session: &mut AiEditSession,
    proposal_id: Option<AiProposalId>,
    workspace_revision: Option<u64>,
    kind: AiEditSessionEventKind,
    message: impl Into<String>,
    diagnostics: Vec<Diagnostic>,
) {
    let next_sequence = session
        .events
        .last()
        .map(|event| event.sequence + 1)
        .unwrap_or(1);
    session.events.push(AiEditSessionEvent {
        sequence: next_sequence,
        session_id: session.session_id.clone(),
        proposal_id,
        workspace_revision,
        kind,
        message: message.into(),
        diagnostics,
    });
}

pub(crate) fn session_is_terminal(status: AiEditSessionStatus) -> bool {
    matches!(
        status,
        AiEditSessionStatus::Completed
            | AiEditSessionStatus::Cancelled
            | AiEditSessionStatus::Failed
            | AiEditSessionStatus::Reverted
    )
}

pub(crate) fn record_session_diagnostics(session: &mut AiEditSession, diagnostics: &[Diagnostic]) {
    session.diagnostics.extend_from_slice(diagnostics);
}

pub(crate) fn list_sessions_bounded(
    sessions: Vec<AiEditSession>,
    bounds: CollectionBounds,
) -> BoundedList<AiEditSession> {
    BoundedList {
        total_count: sessions.len(),
        truncated: sessions.len() > bounds.limit,
        items: sessions.into_iter().take(bounds.limit).collect(),
    }
}

impl WorkspaceApi {
    pub fn start_ai_edit_session(
        workspace: &Workspace,
        request: StartAiEditSessionRequest,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let mut session = AiEditSession {
            session_id: AiEditSessionId::new(),
            user_intent: request.user_intent.trim().to_owned(),
            base_revision: workspace.revision().get(),
            created_at_millis: current_time_millis(),
            policy: request.policy,
            status: AiEditSessionStatus::Open,
            proposals: Vec::new(),
            accepted_proposal_ids: Vec::new(),
            rejected_proposal_ids: Vec::new(),
            applied_changes: Vec::new(),
            failed_proposal_ids: Vec::new(),
            diagnostics: Vec::new(),
            restore_point: None,
            final_outcome: None,
            events: Vec::new(),
        };
        if session.user_intent.is_empty() {
            return Err(WorkspaceApiError::InvalidRequest {
                message: "ai edit session user intent must not be empty".to_owned(),
            });
        }
        let start_message = format!("Started AI edit session for `{}`", session.user_intent);
        push_session_event(
            &mut session,
            None,
            Some(workspace.revision().get()),
            AiEditSessionEventKind::SessionStarted,
            start_message,
            Vec::new(),
        );
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn get_ai_edit_session(
        workspace: &Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let session = load_ai_edit_session(workspace, &session_id)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn list_ai_edit_sessions(
        workspace: &Workspace,
        bounds: CollectionBounds,
    ) -> Result<WorkspaceReadContext<BoundedList<AiEditSession>>, WorkspaceApiError> {
        let sessions = load_ai_edit_sessions(workspace)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: list_sessions_bounded(sessions, bounds),
        })
    }

    pub fn submit_ai_proposal(
        workspace: &mut Workspace,
        session_id: &str,
        request: SubmitAiProposalRequest,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(session_id.to_owned());
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        let proposal = build_session_proposal(workspace, &session, request)?;
        let proposal_id = proposal.proposal_id.clone();
        let proposal_state = proposal.state;
        let proposal_diagnostics = proposal.diagnostics.clone();
        session.proposals.push(proposal);
        match proposal_state {
            AiProposalState::Failed => {
                session.failed_proposal_ids.push(proposal_id.clone());
                record_session_diagnostics(&mut session, &proposal_diagnostics);
                push_session_event(
                    &mut session,
                    Some(proposal_id),
                    Some(workspace.revision().get()),
                    AiEditSessionEventKind::ProposalFailed,
                    "AI proposal failed validation",
                    proposal_diagnostics,
                );
            }
            AiProposalState::Stale => {
                push_session_event(
                    &mut session,
                    Some(proposal_id),
                    Some(workspace.revision().get()),
                    AiEditSessionEventKind::ProposalStale,
                    "AI proposal was submitted against a stale workspace revision",
                    proposal_diagnostics,
                );
            }
            _ => {
                push_session_event(
                    &mut session,
                    Some(proposal_id.clone()),
                    Some(workspace.revision().get()),
                    AiEditSessionEventKind::ProposalAdded,
                    "AI proposal added to session",
                    Vec::new(),
                );
                if session.policy == AiEditSessionPolicy::FailOnApprovalRequired {
                    mark_proposal_failed(
                        &mut session,
                        &proposal_id,
                        "session policy requires failure instead of waiting for approval",
                    )?;
                } else if session.policy == AiEditSessionPolicy::AutoApply {
                    apply_session_proposal(workspace, &mut session, &proposal_id, false)?;
                }
            }
        }
        update_session_status(&mut session);
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn submit_live_ai_step(
        workspace: &mut Workspace,
        session_id: &str,
        request: SubmitAiProposalRequest,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session = load_ai_edit_session(workspace, &AiEditSessionId(session_id.to_owned()))?;
        if session.policy != AiEditSessionPolicy::AutoApply {
            return Err(WorkspaceApiError::InvalidSessionState {
                message: "live AI steps require an auto-apply session".to_owned(),
            });
        }
        Self::submit_ai_proposal(workspace, session_id, request)
    }

    pub fn accept_ai_proposal(
        workspace: &mut Workspace,
        query: SessionProposalQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let proposal_id = AiProposalId(query.proposal_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        apply_session_proposal(workspace, &mut session, &proposal_id, true)?;
        update_session_status(&mut session);
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn reject_ai_proposal(
        workspace: &Workspace,
        query: SessionProposalQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let proposal_id = AiProposalId(query.proposal_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        let proposal = proposal_mut(&mut session, &proposal_id)?;
        if !matches!(
            proposal.state,
            AiProposalState::Proposed | AiProposalState::Stale
        ) {
            return Err(WorkspaceApiError::InvalidSessionState {
                message: format!(
                    "proposal `{}` cannot be rejected from state `{:?}`",
                    proposal_id.as_str(),
                    proposal.state
                ),
            });
        }
        proposal.state = AiProposalState::Rejected;
        if !session.rejected_proposal_ids.contains(&proposal_id) {
            session.rejected_proposal_ids.push(proposal_id.clone());
        }
        push_session_event(
            &mut session,
            Some(proposal_id),
            Some(workspace.revision().get()),
            AiEditSessionEventKind::ProposalRejected,
            "AI proposal rejected",
            Vec::new(),
        );
        update_session_status(&mut session);
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn accept_all_ai_proposals(
        workspace: &mut Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        let proposal_ids = session
            .proposals
            .iter()
            .filter(|proposal| {
                matches!(
                    proposal.state,
                    AiProposalState::Proposed | AiProposalState::Stale
                )
            })
            .map(|proposal| proposal.proposal_id.clone())
            .collect::<Vec<_>>();
        for proposal_id in proposal_ids {
            apply_session_proposal(workspace, &mut session, &proposal_id, true)?;
        }
        update_session_status(&mut session);
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn reject_all_ai_proposals(
        workspace: &Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        let proposal_ids = session
            .proposals
            .iter()
            .filter(|proposal| {
                matches!(
                    proposal.state,
                    AiProposalState::Proposed | AiProposalState::Stale
                )
            })
            .map(|proposal| proposal.proposal_id.clone())
            .collect::<Vec<_>>();
        for proposal_id in proposal_ids {
            let proposal = proposal_mut(&mut session, &proposal_id)?;
            proposal.state = AiProposalState::Rejected;
            if !session.rejected_proposal_ids.contains(&proposal_id) {
                session.rejected_proposal_ids.push(proposal_id.clone());
            }
            push_session_event(
                &mut session,
                Some(proposal_id),
                Some(workspace.revision().get()),
                AiEditSessionEventKind::ProposalRejected,
                "AI proposal rejected",
                Vec::new(),
            );
        }
        update_session_status(&mut session);
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn cancel_ai_edit_session(
        workspace: &Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        for proposal in &mut session.proposals {
            if matches!(
                proposal.state,
                AiProposalState::Proposed | AiProposalState::Stale
            ) {
                proposal.state = AiProposalState::Cancelled;
            }
        }
        session.status = AiEditSessionStatus::Cancelled;
        session.final_outcome = Some(AiEditSessionOutcome {
            kind: AiEditSessionOutcomeKind::Cancelled,
            message: "AI edit session cancelled".to_owned(),
        });
        push_session_event(
            &mut session,
            None,
            Some(workspace.revision().get()),
            AiEditSessionEventKind::SessionCancelled,
            "AI edit session cancelled",
            Vec::new(),
        );
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn complete_ai_edit_session(
        workspace: &Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        ensure_session_mutable(&session)?;
        if session.proposals.iter().any(|proposal| {
            matches!(
                proposal.state,
                AiProposalState::Proposed | AiProposalState::Stale
            )
        }) {
            return Err(WorkspaceApiError::InvalidSessionState {
                message: "cannot complete an AI session while reviewable proposals remain"
                    .to_owned(),
            });
        }
        session.status = AiEditSessionStatus::Completed;
        session.final_outcome = Some(AiEditSessionOutcome {
            kind: AiEditSessionOutcomeKind::Completed,
            message: "AI edit session completed".to_owned(),
        });
        push_session_event(
            &mut session,
            None,
            Some(workspace.revision().get()),
            AiEditSessionEventKind::SessionCompleted,
            "AI edit session completed",
            Vec::new(),
        );
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn revert_ai_edit_session(
        workspace: &mut Workspace,
        query: SessionQuery,
    ) -> Result<WorkspaceReadContext<AiEditSession>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let mut session = load_ai_edit_session(workspace, &session_id)?;
        if session.applied_changes.is_empty() {
            return Err(WorkspaceApiError::InvalidSessionState {
                message: "cannot revert an AI session with no committed changes".to_owned(),
            });
        }
        let restore_point = session.restore_point.clone().ok_or_else(|| {
            WorkspaceApiError::InvalidSessionState {
                message: "ai session has no restore point".to_owned(),
            }
        })?;
        let conflicts = find_non_session_conflicts(workspace, &session)?;
        if !conflicts.is_empty() {
            return Err(WorkspaceApiError::RevertConflict {
                message: "cannot safely revert AI session because later non-session edits exist"
                    .to_owned(),
                conflicting_transaction_ids: conflicts,
            });
        }
        let snapshot = workspace
            .snapshots()
            .map_err(|error| WorkspaceApiError::Workspace {
                message: error.to_string(),
            })?
            .into_iter()
            .find(|snapshot| snapshot.id().to_string() == restore_point.snapshot_id)
            .ok_or_else(|| WorkspaceApiError::InvalidRequest {
                message: format!(
                    "restore snapshot `{}` is unavailable",
                    restore_point.snapshot_id
                ),
            })?;
        let commit = workspace
            .restore_snapshot(snapshot.id(), TransactionActor::Ai)
            .map_err(workspace_transaction_error)?;
        session.status = AiEditSessionStatus::Reverted;
        session.final_outcome = Some(AiEditSessionOutcome {
            kind: AiEditSessionOutcomeKind::Reverted,
            message: format!(
                "AI edit session reverted to snapshot {}",
                restore_point.snapshot_id
            ),
        });
        push_session_event(
            &mut session,
            None,
            Some(workspace.revision().get()),
            AiEditSessionEventKind::SessionReverted,
            format!(
                "AI edit session reverted with transaction {}",
                commit.transaction_id()
            ),
            Vec::new(),
        );
        save_ai_edit_session(workspace, &session)?;
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: session,
        })
    }

    pub fn get_ai_edit_session_events(
        workspace: &Workspace,
        query: SessionEventQuery,
    ) -> Result<WorkspaceReadContext<BoundedList<AiEditSessionEvent>>, WorkspaceApiError> {
        let session_id = AiEditSessionId(query.session_id);
        let session = load_ai_edit_session(workspace, &session_id)?;
        let events = session
            .events
            .into_iter()
            .filter(|event| {
                query
                    .after_sequence
                    .is_none_or(|after| event.sequence > after)
            })
            .collect::<Vec<_>>();
        Ok(WorkspaceReadContext {
            revision: workspace.revision().get(),
            value: BoundedList {
                total_count: events.len(),
                truncated: events.len() > query.limit.unwrap_or(64),
                items: events.into_iter().take(query.limit.unwrap_or(64)).collect(),
            },
        })
    }
}

fn build_session_proposal(
    workspace: &Workspace,
    session: &AiEditSession,
    request: SubmitAiProposalRequest,
) -> Result<AiEditProposal, WorkspaceApiError> {
    let mut proposal = AiEditProposal {
        proposal_id: AiProposalId::new(),
        session_id: session.session_id.clone(),
        base_revision: request.proposal.expected_revision,
        rationale: request.rationale,
        proposal: request.proposal.clone(),
        diff: None,
        diagnostics: Vec::new(),
        affected_node_ids: Vec::new(),
        affected_parameter_ids: Vec::new(),
        expected_rebuild_scope: RebuildScopeView {
            affected_node_ids: Vec::new(),
            affected_parameter_ids: Vec::new(),
        },
        state: AiProposalState::Proposed,
        revalidation: Some(AiProposalRevalidation::NotNeeded),
        transaction_id: None,
        resulting_revision: None,
    };

    if !matches!(request.proposal.actor.as_str(), "ai") {
        proposal.state = AiProposalState::Failed;
        proposal.diagnostics = Vec::new();
        return Ok(proposal);
    }

    if request.proposal.expected_revision != workspace.revision().get() {
        proposal.state = AiProposalState::Stale;
        proposal.revalidation = Some(AiProposalRevalidation::Conflicts);
        return Ok(proposal);
    }

    match WorkspaceApi::dry_run_transaction(workspace, request.proposal) {
        Ok(result) => {
            proposal.diff = result.value.diff.clone();
            proposal.affected_node_ids = result.value.affected_node_ids;
            proposal.affected_parameter_ids = result.value.affected_parameter_ids;
            proposal.expected_rebuild_scope = result.value.expected_rebuild_scope;
        }
        Err(WorkspaceApiError::Validation { diagnostics, .. }) => {
            proposal.state = AiProposalState::Failed;
            proposal.diagnostics = diagnostics;
        }
        Err(error) => return Err(error),
    }

    Ok(proposal)
}

fn apply_session_proposal(
    workspace: &mut Workspace,
    session: &mut AiEditSession,
    proposal_id: &AiProposalId,
    allow_revalidate: bool,
) -> Result<(), WorkspaceApiError> {
    ensure_restore_point(workspace, session)?;
    let current_revision = workspace.revision().get();
    let proposal = proposal_mut(session, proposal_id)?;
    if matches!(
        proposal.state,
        AiProposalState::Applied | AiProposalState::Rejected | AiProposalState::Cancelled
    ) {
        return Err(WorkspaceApiError::InvalidSessionState {
            message: format!(
                "proposal `{}` is no longer reviewable",
                proposal_id.as_str()
            ),
        });
    }

    let mut to_apply = proposal.proposal.clone();
    let mut revalidation = AiProposalRevalidation::StillValid;
    if proposal.base_revision != current_revision {
        if !allow_revalidate {
            proposal.state = AiProposalState::Stale;
            proposal.revalidation = Some(AiProposalRevalidation::Conflicts);
            return Ok(());
        }
        to_apply.expected_revision = current_revision;
        match WorkspaceApi::dry_run_transaction(workspace, to_apply.clone()) {
            Ok(result) => {
                proposal.diff = result.value.diff.clone();
                proposal.affected_node_ids = result.value.affected_node_ids;
                proposal.affected_parameter_ids = result.value.affected_parameter_ids;
                proposal.expected_rebuild_scope = result.value.expected_rebuild_scope;
                revalidation = AiProposalRevalidation::StaleButRevalidated;
            }
            Err(WorkspaceApiError::Validation { diagnostics, .. }) => {
                proposal.state = AiProposalState::Stale;
                proposal.revalidation = Some(AiProposalRevalidation::Invalid);
                proposal.diagnostics = diagnostics.clone();
                record_session_diagnostics(session, &diagnostics);
                push_session_event(
                    session,
                    Some(proposal_id.clone()),
                    Some(workspace.revision().get()),
                    AiEditSessionEventKind::ProposalStale,
                    "AI proposal could not be safely revalidated",
                    diagnostics,
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    session.status = AiEditSessionStatus::Applying;
    let transaction = transaction_from_proposal_with_correlation(
        &to_apply,
        current_revision,
        Some(TransactionCorrelation {
            ai_session_id: Some(session.session_id.as_str().to_owned()),
            ai_proposal_id: Some(proposal_id.as_str().to_owned()),
        }),
    )?;
    let diff = dry_run_diff(workspace, &transaction)?;
    let commit = workspace
        .apply_transaction(&transaction)
        .map_err(workspace_transaction_error)?;
    let proposal = proposal_mut(session, proposal_id)?;
    proposal.state = AiProposalState::Applied;
    proposal.revalidation = Some(revalidation);
    proposal.transaction_id = Some(commit.transaction_id().to_string());
    proposal.resulting_revision = Some(commit.revision_after().get());
    proposal.diff = Some(diff);
    if !session.accepted_proposal_ids.contains(proposal_id) {
        session.accepted_proposal_ids.push(proposal_id.clone());
    }
    session.applied_changes.push(AppliedAiChange {
        proposal_id: proposal_id.clone(),
        transaction_id: commit.transaction_id().to_string(),
        revision_before: commit.revision_before().get(),
        revision_after: commit.revision_after().get(),
    });
    push_session_event(
        session,
        Some(proposal_id.clone()),
        Some(workspace.revision().get()),
        AiEditSessionEventKind::ProposalAccepted,
        "AI proposal accepted",
        Vec::new(),
    );
    push_session_event(
        session,
        Some(proposal_id.clone()),
        Some(workspace.revision().get()),
        AiEditSessionEventKind::TransactionApplied,
        format!("Applied transaction {}", commit.transaction_id()),
        Vec::new(),
    );
    Ok(())
}

fn ensure_restore_point(
    workspace: &Workspace,
    session: &mut AiEditSession,
) -> Result<(), WorkspaceApiError> {
    if session.restore_point.is_some() {
        return Ok(());
    }
    let snapshot = workspace
        .create_snapshot(
            format!("AI session {} restore point", session.session_id.as_str()),
            TransactionActor::Ai,
        )
        .map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    session.restore_point = Some(AiRestorePoint {
        snapshot_id: snapshot.id().to_string(),
        created_from_revision: snapshot.created_from_revision().get(),
        created_at_millis: snapshot.created_at_millis(),
    });
    Ok(())
}

fn find_non_session_conflicts(
    workspace: &Workspace,
    session: &AiEditSession,
) -> Result<Vec<String>, WorkspaceApiError> {
    let entries = workspace
        .history_entries()
        .map_err(|error| WorkspaceApiError::Workspace {
            message: error.to_string(),
        })?;
    Ok(entries
        .into_iter()
        .filter(|entry| {
            entry.revision_after().get() > session.base_revision
                && entry
                    .correlation()
                    .and_then(|correlation| correlation.ai_session_id.as_deref())
                    != Some(session.session_id.as_str())
        })
        .map(|entry| entry.transaction_id().to_string())
        .collect())
}

fn ensure_session_mutable(session: &AiEditSession) -> Result<(), WorkspaceApiError> {
    if session_is_terminal(session.status) {
        return Err(WorkspaceApiError::InvalidSessionState {
            message: format!(
                "ai edit session `{}` is already in terminal state `{:?}`",
                session.session_id.as_str(),
                session.status
            ),
        });
    }
    Ok(())
}

fn update_session_status(session: &mut AiEditSession) {
    if session_is_terminal(session.status) {
        return;
    }
    session.status = if session.proposals.iter().any(|proposal| {
        matches!(
            proposal.state,
            AiProposalState::Proposed | AiProposalState::Stale
        )
    }) {
        AiEditSessionStatus::AwaitingReview
    } else {
        AiEditSessionStatus::Open
    };
}

fn proposal_mut<'a>(
    session: &'a mut AiEditSession,
    proposal_id: &AiProposalId,
) -> Result<&'a mut AiEditProposal, WorkspaceApiError> {
    session
        .proposals
        .iter_mut()
        .find(|proposal| &proposal.proposal_id == proposal_id)
        .ok_or_else(|| WorkspaceApiError::InvalidRequest {
            message: format!("unknown ai proposal `{}`", proposal_id.as_str()),
        })
}

fn mark_proposal_failed(
    session: &mut AiEditSession,
    proposal_id: &AiProposalId,
    message: &str,
) -> Result<(), WorkspaceApiError> {
    let proposal = proposal_mut(session, proposal_id)?;
    proposal.state = AiProposalState::Failed;
    proposal.revalidation = Some(AiProposalRevalidation::Invalid);
    if !session.failed_proposal_ids.contains(proposal_id) {
        session.failed_proposal_ids.push(proposal_id.clone());
    }
    push_session_event(
        session,
        Some(proposal_id.clone()),
        None,
        AiEditSessionEventKind::ProposalFailed,
        message,
        Vec::new(),
    );
    Ok(())
}

fn transaction_from_proposal_with_correlation(
    proposal: &TransactionProposal,
    current_revision: u64,
    correlation: Option<TransactionCorrelation>,
) -> Result<geom_workspace::WorkspaceTransaction, WorkspaceApiError> {
    if proposal.expected_revision != current_revision {
        return Err(WorkspaceApiError::StaleRevision {
            expected: proposal.expected_revision,
            current: current_revision,
        });
    }
    let operations = proposal
        .operations
        .iter()
        .map(crate::workspace_op_from_request)
        .collect::<Result<Vec<_>, _>>()?;
    geom_workspace::WorkspaceTransaction::new_with_correlation(
        parse_actor(proposal.actor.as_str())?,
        proposal.intent.clone(),
        correlation,
        operations,
    )
    .map_err(|error| WorkspaceApiError::Workspace {
        message: error.to_string(),
    })
}

fn parse_actor(raw: &str) -> Result<TransactionActor, WorkspaceApiError> {
    match raw {
        "user" => Ok(TransactionActor::User),
        "ai" => Ok(TransactionActor::Ai),
        "cli_automation" | "cli-automation" => Ok(TransactionActor::CliAutomation),
        "system_migration" | "system-migration" => Ok(TransactionActor::SystemMigration),
        other => Err(WorkspaceApiError::InvalidRequest {
            message: format!("unknown transaction actor `{other}`"),
        }),
    }
}
