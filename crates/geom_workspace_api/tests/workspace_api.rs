use geom_workspace::Workspace;
use geom_workspace_api::protocol::{ApiProtocolRequest, dispatch};
use geom_workspace_api::{
    API_PROTOCOL_VERSION, CollectionBounds, EvaluateOutputRequest, NodeQuery, PreviewRequest,
    RecentHistoryQuery, SourceSnippetRequest, TransactionProposal, WorkspaceApi,
    WorkspaceOpRequest,
};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("workspaces")
        .join("viewport-smoke")
}

fn copy_directory_recursive(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("create target directory");
    for entry in fs::read_dir(source).expect("read source directory") {
        let entry = entry.expect("directory entry");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_directory_recursive(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("copy file");
        }
    }
}

fn clone_workspace_fixture() -> PathBuf {
    let target = std::env::temp_dir().join(format!(
        "workspace-api-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    copy_directory_recursive(&fixture_root(), &target);
    target
}

fn open_fixture_workspace() -> Workspace {
    Workspace::open(clone_workspace_fixture()).expect("open workspace")
}

#[test]
fn read_api_returns_bounded_selective_context() {
    let workspace = open_fixture_workspace();

    let summary = WorkspaceApi::get_workspace_summary(&workspace).expect("summary");
    assert_eq!(summary.revision, 0);
    assert_eq!(summary.value.workspace_name, "Viewport Smoke");
    assert_eq!(summary.value.root_output.as_deref(), Some("root"));
    assert_eq!(summary.value.node_count, 6);
    assert_eq!(summary.value.parameter_count, 1);
    assert_eq!(summary.value.diagnostics.total, 0);
    assert!(summary.value.capabilities.preview_available);

    let tree = WorkspaceApi::get_scene_tree(&workspace, CollectionBounds::new(3)).expect("tree");
    assert_eq!(tree.revision, 0);
    assert_eq!(tree.value.nodes.total_count, 6);
    assert!(tree.value.nodes.truncated);
    assert_eq!(tree.value.root_nodes, vec!["root"]);

    let node = WorkspaceApi::get_node(
        &workspace,
        &NodeQuery {
            node_id: "arm".to_owned(),
            include_source_snippet: true,
        },
    )
    .expect("node");
    assert_eq!(node.revision, 0);
    assert_eq!(node.value.node_id, "arm");
    assert_eq!(node.value.kind, "cylinder");
    assert_eq!(node.value.parameter_dependencies, vec!["arm_length"]);
    assert!(node.value.source_snippet.is_some());
    assert!(
        node.value
            .source_snippet
            .as_ref()
            .expect("snippet")
            .snippet
            .contains("[nodes.arm]")
    );

    let parameters =
        WorkspaceApi::get_parameters(&workspace, CollectionBounds::new(1)).expect("parameters");
    assert_eq!(parameters.value.total_count, 1);
    assert!(!parameters.value.truncated);
    assert_eq!(parameters.value.items[0].parameter_id, "arm_length");

    let stats = WorkspaceApi::evaluate_output(
        &workspace,
        EvaluateOutputRequest {
            node_id: Some("union_shape".to_owned()),
            parameter_limit: 1,
        },
    )
    .expect("eval");
    assert_eq!(stats.value.requested_output, "union_shape");
    assert!(stats.value.triangle_count > 0);
    assert_eq!(stats.value.resolved_parameters.total_count, 1);

    let snippet = WorkspaceApi::source_snippet(
        &workspace,
        SourceSnippetRequest {
            node_id: None,
            parameter_id: Some("arm_length".to_owned()),
            line_radius: 1,
        },
    )
    .expect("source snippet");
    assert_eq!(snippet.revision, 0);
    assert!(
        snippet
            .value
            .expect("parameter snippet")
            .snippet
            .contains("[params.arm_length]")
    );
}

#[test]
fn stale_revision_is_rejected_without_mutation_or_history() {
    let mut workspace = open_fixture_workspace();
    let before_revision = workspace.revision().get();
    let before_source = workspace.source_text().to_owned();
    let before_history_len = workspace.history_entries().expect("history").len();

    let result = WorkspaceApi::apply_transaction(
        &mut workspace,
        TransactionProposal {
            expected_revision: before_revision + 1,
            actor: "ai".to_owned(),
            intent: Some("stale edit".to_owned()),
            operations: vec![WorkspaceOpRequest::SetNodeLabel {
                node_id: "body".to_owned(),
                label: Some("Body".to_owned()),
            }],
        },
    );

    let error = result.expect_err("stale revision should fail");
    assert!(matches!(
        error,
        geom_workspace_api::WorkspaceApiError::StaleRevision { .. }
    ));
    assert_eq!(workspace.revision().get(), before_revision);
    assert_eq!(workspace.source_text(), before_source);
    assert_eq!(
        workspace.history_entries().expect("history").len(),
        before_history_len
    );
}

#[test]
fn dry_run_and_apply_share_transaction_path_and_history() {
    let mut workspace = open_fixture_workspace();
    let before_source = workspace.source_text().to_owned();

    let proposal = TransactionProposal {
        expected_revision: workspace.revision().get(),
        actor: "ai".to_owned(),
        intent: Some("rename cap label".to_owned()),
        operations: vec![
            WorkspaceOpRequest::SetNodeLabel {
                node_id: "cap".to_owned(),
                label: Some("Top Cap".to_owned()),
            },
            WorkspaceOpRequest::SetParameterScalar {
                parameter_id: "arm_length".to_owned(),
                value: 3.4,
            },
        ],
    };

    let dry_run = WorkspaceApi::dry_run_transaction(&workspace, proposal.clone()).expect("dry run");
    assert!(dry_run.value.accepted);
    assert_eq!(dry_run.revision, 0);
    assert!(dry_run.value.resulting_revision.is_none());
    assert_eq!(workspace.source_text(), before_source);
    assert!(workspace.history_entries().expect("history").is_empty());
    assert!(
        dry_run
            .value
            .affected_parameter_ids
            .contains(&"arm_length".to_owned())
    );

    let applied = WorkspaceApi::apply_transaction(&mut workspace, proposal).expect("apply");
    assert!(applied.value.accepted);
    assert_eq!(applied.value.base_revision, 0);
    assert_eq!(applied.value.resulting_revision, Some(2));
    assert_ne!(workspace.source_text(), before_source);

    let history = workspace.history_entries().expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(format!("{:?}", history[0].actor()).to_lowercase(), "ai");
    assert_eq!(history[0].intent(), Some("rename cap label"));

    let node = WorkspaceApi::get_node(
        &workspace,
        &NodeQuery {
            node_id: "cap".to_owned(),
            include_source_snippet: false,
        },
    )
    .expect("node");
    assert_eq!(node.value.label.as_deref(), Some("Top Cap"));
}

#[test]
fn protocol_dispatch_preserves_ids_and_reports_unknown_method() {
    let response = dispatch(ApiProtocolRequest {
        version: API_PROTOCOL_VERSION,
        id: Some("req-7".to_owned()),
        method: "nope".to_owned(),
        workspace_root: None,
        params: json!({}),
    });
    assert_eq!(response.id.as_deref(), Some("req-7"));
    let error = response.error.expect("error");
    assert_eq!(error.code, "unknown_method");

    let version_response = dispatch(ApiProtocolRequest {
        version: API_PROTOCOL_VERSION + 1,
        id: Some("req-8".to_owned()),
        method: "capabilities".to_owned(),
        workspace_root: None,
        params: json!({}),
    });
    let error = version_response.error.expect("version error");
    assert_eq!(error.code, "unsupported_version");
}

#[test]
fn invalid_proposal_returns_structured_diagnostics() {
    let mut workspace = open_fixture_workspace();
    let expected_revision = workspace.revision().get();
    let error = WorkspaceApi::apply_transaction(
        &mut workspace,
        TransactionProposal {
            expected_revision,
            actor: "ai".to_owned(),
            intent: Some("break graph".to_owned()),
            operations: vec![WorkspaceOpRequest::DeleteNode {
                node_id: "union_shape".to_owned(),
            }],
        },
    )
    .expect_err("invalid proposal should fail");

    match error {
        geom_workspace_api::WorkspaceApiError::Validation { diagnostics, .. } => {
            assert!(!diagnostics.is_empty());
            assert!(diagnostics.iter().any(|diagnostic| diagnostic.blocking));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn fake_agent_protocol_flow_uses_only_public_transport() {
    let workspace_root = clone_workspace_fixture();
    let mut child = Command::new(env!("CARGO_BIN_EXE_morphos-workspace-api"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn protocol binary");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout = BufReader::new(stdout);

    let summary = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "1",
            "method": "get_workspace_summary",
            "workspace_root": workspace_root,
            "params": {}
        }),
    );
    assert_eq!(summary["id"], "1");
    assert_eq!(
        summary["result"]["value"]["workspace_name"],
        "Viewport Smoke"
    );

    let tree = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "2",
            "method": "get_scene_tree",
            "workspace_root": workspace_root,
            "params": { "bounds": { "limit": 4 } }
        }),
    );
    assert_eq!(tree["result"]["value"]["nodes"]["truncated"], true);

    let node = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "3",
            "method": "get_node",
            "workspace_root": workspace_root,
            "params": { "node_id": "cap", "include_source_snippet": true }
        }),
    );
    assert_eq!(node["result"]["value"]["node_id"], "cap");

    let dry_run = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "4",
            "method": "dry_run_transaction",
            "workspace_root": workspace_root,
            "params": {
                "expected_revision": 0,
                "actor": "ai",
                "intent": "protocol edit",
                "operations": [
                    {
                        "kind": "set_node_label",
                        "node_id": "cap",
                        "label": "Protocol Cap"
                    }
                ]
            }
        }),
    );
    assert_eq!(dry_run["result"]["revision"], 0);
    assert_eq!(dry_run["result"]["value"]["accepted"], true);

    let applied = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "5",
            "method": "apply_transaction",
            "workspace_root": workspace_root,
            "params": {
                "expected_revision": 0,
                "actor": "ai",
                "intent": "protocol edit",
                "operations": [
                    {
                        "kind": "set_node_label",
                        "node_id": "cap",
                        "label": "Protocol Cap"
                    }
                ]
            }
        }),
    );
    assert_eq!(applied["result"]["value"]["resulting_revision"], 2);

    let history = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "6",
            "method": "get_recent_history",
            "workspace_root": workspace_root,
            "params": RecentHistoryQuery {
                actor: Some("ai".to_owned()),
                node_id: None,
                parameter_id: None,
                start_millis: None,
                end_millis: None,
                limit: Some(5),
            }
        }),
    );
    assert_eq!(history["result"]["value"]["items"][0]["actor"], "ai");

    let preview = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "7",
            "method": "request_preview",
            "workspace_root": workspace_root,
            "params": PreviewRequest {
                node_id: Some("root".to_owned()),
                width: 96,
                height: 96,
                destination: Some("ai/protocol-preview.png".to_owned()),
                overwrite: true,
            }
        }),
    );
    let preview_path = preview["result"]["value"]["absolute_path"]
        .as_str()
        .expect("preview path");
    assert!(Path::new(preview_path).is_file());

    let invalid = send_request(
        &mut stdin,
        &mut stdout,
        json!({
            "version": API_PROTOCOL_VERSION,
            "id": "8",
            "method": "apply_transaction",
            "workspace_root": workspace_root,
            "params": {
                "expected_revision": 2,
                "actor": "ai",
                "intent": "invalid delete",
                "operations": [
                    {
                        "kind": "delete_node",
                        "node_id": "union_shape"
                    }
                ]
            }
        }),
    );
    assert_eq!(invalid["error"]["code"], "validation_failed");
    assert!(
        invalid["error"]["data"]["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .iter()
            .any(|diagnostic| diagnostic["blocking"] == true)
    );

    drop(stdin);
    let status = child.wait().expect("wait");
    assert!(status.success());
}

#[test]
fn malformed_request_returns_structured_stdout_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_morphos-workspace-api"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn protocol binary");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout = BufReader::new(stdout);

    writeln!(stdin, "{{ not json").expect("write malformed");
    stdin.flush().expect("flush malformed");

    let mut line = String::new();
    stdout.read_line(&mut line).expect("read response");
    let response: Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(response["error"]["code"], "malformed_request");

    drop(stdin);
    let output = child.wait_with_output().expect("output");
    assert!(output.stderr.is_empty());
}

fn send_request(stdin: &mut impl Write, stdout: &mut impl BufRead, request: Value) -> Value {
    writeln!(
        stdin,
        "{}",
        serde_json::to_string(&request).expect("serialize request")
    )
    .expect("write request");
    stdin.flush().expect("flush request");

    let mut line = String::new();
    stdout.read_line(&mut line).expect("read response");
    serde_json::from_str(&line).expect("parse response")
}
