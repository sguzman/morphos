use crate::{
    CollectionBounds, DiagnosticFilter, EvaluateOutputRequest, NodeQuery, PreviewRequest,
    RecentHistoryQuery, SessionEventQuery, SessionProposalQuery, SessionQuery,
    SourceSnippetRequest, StartAiEditSessionRequest, SubmitAiProposalRequest, TransactionProposal,
    WorkspaceApi, WorkspaceApiError,
};
use geom_workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiProtocolRequest {
    pub version: u32,
    #[serde(default)]
    pub id: Option<String>,
    pub method: String,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiProtocolResponse {
    pub version: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiProtocolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Default)]
pub struct ProtocolServer {
    workspaces: BTreeMap<String, Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct BoundsRequest {
    #[serde(default)]
    bounds: CollectionBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct DiagnosticsRequest {
    #[serde(default)]
    filter: DiagnosticFilter,
    #[serde(default)]
    bounds: CollectionBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct SessionListRequest {
    #[serde(default)]
    bounds: CollectionBounds,
}

impl ApiProtocolResponse {
    pub fn success(id: Option<String>, result: Value) -> Self {
        Self {
            version: crate::API_PROTOCOL_VERSION,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(
        version: u32,
        id: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Self {
            version,
            id,
            result: None,
            error: Some(ApiProtocolError {
                code: code.into(),
                message: message.into(),
                data,
            }),
        }
    }
}

pub fn dispatch(request: ApiProtocolRequest) -> ApiProtocolResponse {
    ProtocolServer::default().dispatch(request)
}

impl ProtocolServer {
    pub fn dispatch(&mut self, request: ApiProtocolRequest) -> ApiProtocolResponse {
        let version = request.version;
        let id = request.id.clone();
        if version != crate::API_PROTOCOL_VERSION {
            return ApiProtocolResponse::error(
                crate::API_PROTOCOL_VERSION,
                id,
                "unsupported_version",
                format!(
                    "unsupported protocol version {version}; expected {}",
                    crate::API_PROTOCOL_VERSION
                ),
                Some(json!({
                    "requested_version": version,
                    "supported_version": crate::API_PROTOCOL_VERSION,
                })),
            );
        }

        let result = match request.method.as_str() {
            "capabilities" => serialize_result(&WorkspaceApi::capabilities()),
            "get_workspace_summary" => self.with_workspace(&request, |workspace| {
                serialize_result(&WorkspaceApi::get_workspace_summary(workspace)?)
            }),
            "get_scene_tree" => self.with_workspace(&request, |workspace| {
                let params: BoundsRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_scene_tree(workspace, params.bounds)?)
            }),
            "get_node" => self.with_workspace(&request, |workspace| {
                let params: NodeQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_node(workspace, &params)?)
            }),
            "get_parameters" => self.with_workspace(&request, |workspace| {
                let params: BoundsRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_parameters(workspace, params.bounds)?)
            }),
            "get_diagnostics" => self.with_workspace(&request, |workspace| {
                let params: DiagnosticsRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_diagnostics(
                    workspace,
                    params.filter,
                    params.bounds,
                )?)
            }),
            "get_recent_history" => self.with_workspace(&request, |workspace| {
                let params: RecentHistoryQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_recent_history(workspace, params)?)
            }),
            "evaluate_output" => self.with_workspace(&request, |workspace| {
                let params: EvaluateOutputRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::evaluate_output(workspace, params)?)
            }),
            "request_preview" => self.with_workspace(&request, |workspace| {
                let params: PreviewRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::request_preview(workspace, params)?)
            }),
            "source_snippet" => self.with_workspace(&request, |workspace| {
                let params: SourceSnippetRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::source_snippet(workspace, params)?)
            }),
            "start_edit_session" => self.with_workspace(&request, |workspace| {
                let params: StartAiEditSessionRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::start_ai_edit_session(workspace, params)?)
            }),
            "get_edit_session" => self.with_workspace(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_ai_edit_session(workspace, params)?)
            }),
            "list_edit_sessions" => self.with_workspace(&request, |workspace| {
                let params: SessionListRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::list_ai_edit_sessions(
                    workspace,
                    params.bounds,
                )?)
            }),
            "submit_proposal" => self.with_workspace_mut(&request, |workspace| {
                let params: SessionSubmitProposalRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::submit_ai_proposal(
                    workspace,
                    &params.session_id,
                    params.request,
                )?)
            }),
            "accept_proposal" => self.with_workspace_mut(&request, |workspace| {
                let params: SessionProposalQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::accept_ai_proposal(workspace, params)?)
            }),
            "reject_proposal" => self.with_workspace(&request, |workspace| {
                let params: SessionProposalQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::reject_ai_proposal(workspace, params)?)
            }),
            "accept_all" => self.with_workspace_mut(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::accept_all_ai_proposals(workspace, params)?)
            }),
            "reject_all" => self.with_workspace(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::reject_all_ai_proposals(workspace, params)?)
            }),
            "submit_live_step" => self.with_workspace_mut(&request, |workspace| {
                let params: SessionSubmitProposalRequest = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::submit_live_ai_step(
                    workspace,
                    &params.session_id,
                    params.request,
                )?)
            }),
            "cancel_edit_session" => self.with_workspace(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::cancel_ai_edit_session(workspace, params)?)
            }),
            "complete_edit_session" => self.with_workspace(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::complete_ai_edit_session(workspace, params)?)
            }),
            "revert_edit_session" => self.with_workspace_mut(&request, |workspace| {
                let params: SessionQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::revert_ai_edit_session(workspace, params)?)
            }),
            "get_edit_session_events" => self.with_workspace(&request, |workspace| {
                let params: SessionEventQuery = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::get_ai_edit_session_events(
                    workspace, params,
                )?)
            }),
            "dry_run_transaction" => self.with_workspace(&request, |workspace| {
                let params: TransactionProposal = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::dry_run_transaction(workspace, params)?)
            }),
            "apply_transaction" => self.with_workspace_mut(&request, |workspace| {
                let params: TransactionProposal = parse_params(request.params.clone())?;
                serialize_result(&WorkspaceApi::apply_transaction(workspace, params)?)
            }),
            other => Err(ApiProtocolError {
                code: "unknown_method".to_owned(),
                message: format!("unknown method `{other}`"),
                data: None,
            }),
        };

        match result {
            Ok(result) => ApiProtocolResponse::success(id, result),
            Err(error) => ApiProtocolResponse::error(
                crate::API_PROTOCOL_VERSION,
                id,
                error.code,
                error.message,
                error.data,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SessionSubmitProposalRequest {
    session_id: String,
    request: SubmitAiProposalRequest,
}

fn parse_params<T>(value: Value) -> Result<T, ApiProtocolError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(value).map_err(|error| ApiProtocolError {
        code: "malformed_params".to_owned(),
        message: error.to_string(),
        data: None,
    })
}

fn serialize_result<T>(value: &T) -> Result<Value, ApiProtocolError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| ApiProtocolError {
        code: "serialization_failed".to_owned(),
        message: error.to_string(),
        data: None,
    })
}

impl ProtocolServer {
    fn with_workspace<F>(
        &mut self,
        request: &ApiProtocolRequest,
        f: F,
    ) -> Result<Value, ApiProtocolError>
    where
        F: FnOnce(&Workspace) -> Result<Value, ApiProtocolError>,
    {
        let workspace_root = self.workspace_root(request)?;
        if !self.workspaces.contains_key(&workspace_root) {
            let workspace =
                Workspace::open(Path::new(&workspace_root)).map_err(|error| ApiProtocolError {
                    code: "workspace_error".to_owned(),
                    message: error.to_string(),
                    data: None,
                })?;
            self.workspaces.insert(workspace_root.clone(), workspace);
        }
        let workspace = self
            .workspaces
            .get(&workspace_root)
            .expect("workspace should be cached");
        f(workspace)
    }

    fn with_workspace_mut<F>(
        &mut self,
        request: &ApiProtocolRequest,
        f: F,
    ) -> Result<Value, ApiProtocolError>
    where
        F: FnOnce(&mut Workspace) -> Result<Value, ApiProtocolError>,
    {
        let workspace_root = self.workspace_root(request)?;
        if !self.workspaces.contains_key(&workspace_root) {
            let workspace =
                Workspace::open(Path::new(&workspace_root)).map_err(|error| ApiProtocolError {
                    code: "workspace_error".to_owned(),
                    message: error.to_string(),
                    data: None,
                })?;
            self.workspaces.insert(workspace_root.clone(), workspace);
        }
        let workspace = self
            .workspaces
            .get_mut(&workspace_root)
            .expect("workspace should be cached");
        f(workspace)
    }

    fn workspace_root(&self, request: &ApiProtocolRequest) -> Result<String, ApiProtocolError> {
        request
            .workspace_root
            .as_deref()
            .map(str::to_owned)
            .ok_or_else(|| ApiProtocolError {
                code: "workspace_required".to_owned(),
                message: "workspace_root is required for this method".to_owned(),
                data: None,
            })
    }
}

fn protocol_error_from_workspace(error: impl Into<WorkspaceApiError>) -> ApiProtocolError {
    let error = error.into();
    match error {
        WorkspaceApiError::InvalidRequest { message } => ApiProtocolError {
            code: "invalid_request".to_owned(),
            message,
            data: None,
        },
        WorkspaceApiError::ApprovalRequired { message } => ApiProtocolError {
            code: "approval_required".to_owned(),
            message,
            data: None,
        },
        WorkspaceApiError::StaleRevision { expected, current } => ApiProtocolError {
            code: "stale_revision".to_owned(),
            message: format!("expected revision {expected}, current revision {current}"),
            data: Some(json!({
                "expected_revision": expected,
                "current_revision": current,
            })),
        },
        WorkspaceApiError::InvalidSessionState { message } => ApiProtocolError {
            code: "invalid_session_state".to_owned(),
            message,
            data: None,
        },
        WorkspaceApiError::RevertConflict {
            message,
            conflicting_transaction_ids,
        } => ApiProtocolError {
            code: "revert_conflict".to_owned(),
            message,
            data: Some(json!({
                "conflicting_transaction_ids": conflicting_transaction_ids,
            })),
        },
        WorkspaceApiError::Validation {
            message,
            diagnostics,
        } => ApiProtocolError {
            code: "validation_failed".to_owned(),
            message,
            data: Some(json!({
                "diagnostics": diagnostics,
            })),
        },
        WorkspaceApiError::Workspace { message } => ApiProtocolError {
            code: "workspace_error".to_owned(),
            message,
            data: None,
        },
    }
}

impl From<WorkspaceApiError> for ApiProtocolError {
    fn from(value: WorkspaceApiError) -> Self {
        protocol_error_from_workspace(value)
    }
}
