use geom_geometry::{BoolmeshBackend, Bounds, EvaluatedGeometry, GeometryEvaluator, Mesh};
use geom_scene::{Node, NodeId, ParamId, SceneDocument, parse_scene};
use geom_workspace::{
    HistoryQuery, OperationId, SnapshotId, TransactionActor, Workspace, WorkspaceDirectory,
    WorkspaceHistoryEntry, WorkspaceOp, WorkspaceSceneChange, WorkspaceSceneDiff,
    WorkspaceSnapshot, WorkspaceTransaction, WorkspaceTransactionCommit,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliExitCode {
    Success = 0,
    Usage = 1,
    Io = 2,
    Source = 3,
    Geometry = 4,
    Internal = 5,
}

impl CliExitCode {
    pub const fn code(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRunResult {
    pub exit_code: CliExitCode,
    pub stdout: String,
    pub stderr: String,
}

impl CliRunResult {
    fn success(stdout: String) -> Self {
        Self {
            exit_code: CliExitCode::Success,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(exit_code: CliExitCode, stderr: String) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Validate {
        workspace: PathBuf,
        format: OutputFormat,
    },
    Inspect {
        workspace: PathBuf,
        format: OutputFormat,
    },
    Eval {
        workspace: PathBuf,
        output: Option<NodeId>,
        format: OutputFormat,
    },
    Export {
        workspace: PathBuf,
        output: Option<NodeId>,
        destination: Option<PathBuf>,
        mesh_format: MeshExportFormat,
        overwrite: bool,
        format: OutputFormat,
    },
    TxApply {
        workspace: PathBuf,
        file: PathBuf,
        format: OutputFormat,
    },
    TxDryRun {
        workspace: PathBuf,
        file: PathBuf,
        format: OutputFormat,
    },
    History {
        workspace: PathBuf,
        actor: Option<TransactionActor>,
        node: Option<NodeId>,
        parameter: Option<ParamId>,
        format: OutputFormat,
    },
    SnapshotCreate {
        workspace: PathBuf,
        name: String,
        format: OutputFormat,
    },
    SnapshotList {
        workspace: PathBuf,
        format: OutputFormat,
    },
    SnapshotRestore {
        workspace: PathBuf,
        snapshot_id: SnapshotId,
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshExportFormat {
    Obj,
    Stl,
}

impl MeshExportFormat {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "obj" => Ok(Self::Obj),
            "stl" => Ok(Self::Stl),
            _ => Err(format!(
                "unsupported export format `{raw}`; supported formats: obj, stl"
            )),
        }
    }

    const fn extension(self) -> &'static str {
        match self {
            Self::Obj => "obj",
            Self::Stl => "stl",
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Obj => "obj",
            Self::Stl => "stl",
        }
    }
}

pub fn run<I>(args: I) -> CliRunResult
where
    I: IntoIterator<Item = OsString>,
{
    let parsed = match parse_command(args) {
        Ok(command) => command,
        Err(error) => return CliRunResult::failure(CliExitCode::Usage, error),
    };

    match execute_command(parsed) {
        Ok(stdout) => CliRunResult::success(stdout),
        Err(error) => CliRunResult::failure(error.exit_code, error.message),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliError {
    exit_code: CliExitCode,
    message: String,
}

#[derive(Debug, Clone, PartialEq)]
struct TransactionExecutionRecord {
    mode: &'static str,
    commit: WorkspaceTransactionCommit,
    diff: Option<WorkspaceSceneDiff>,
}

#[derive(Debug, Deserialize)]
struct TransactionRequestFile {
    #[serde(default)]
    actor: Option<TransactionActorRequest>,
    #[serde(default)]
    intent: Option<String>,
    operations: Vec<WorkspaceOpRequest>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransactionActorRequest {
    User,
    Ai,
    CliAutomation,
    SystemMigration,
}

impl TransactionActorRequest {
    const fn into_actor(self) -> TransactionActor {
        match self {
            Self::User => TransactionActor::User,
            Self::Ai => TransactionActor::Ai,
            Self::CliAutomation => TransactionActor::CliAutomation,
            Self::SystemMigration => TransactionActor::SystemMigration,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WorkspaceOpRequest {
    ReplaceScene {
        scene_source: String,
    },
    AddNode {
        node_id: String,
        draft: SceneNodeDraftRequest,
    },
    ReplaceNode {
        node_id: String,
        scene_source: String,
    },
    DeleteNode {
        node_id: String,
    },
    RenameNode {
        from: String,
        to: String,
    },
    DuplicateNode {
        source_node: String,
        duplicate: String,
    },
    SetNodeLabel {
        node_id: String,
        label: Option<String>,
    },
    SetCompositionChildren {
        node_id: String,
        children: Vec<String>,
    },
    SetParameterScalar {
        parameter_id: String,
        value: f64,
    },
    SetTransformComponent {
        node_id: String,
        property: TransformPropertyRequest,
        axis: AxisRequest,
        value: f64,
    },
    SetPrimitiveScalar {
        node_id: String,
        field: PrimitiveScalarFieldRequest,
        value: f64,
    },
    SetRootNode {
        node_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SceneNodeDraftRequest {
    Box,
    Sphere,
    Cylinder,
    Capsule,
    Plane,
    Profile,
    Union { children: Vec<String> },
    Difference { children: Vec<String> },
    Intersection { children: Vec<String> },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TransformPropertyRequest {
    Translation,
    RotationDeg,
    Scale,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum AxisRequest {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PrimitiveScalarFieldRequest {
    BoxX,
    BoxY,
    BoxZ,
    SphereRadius,
    CylinderRadius,
    CylinderHeight,
    CapsuleRadius,
    CapsuleHeight,
    PlaneWidth,
    PlaneDepth,
    ProfileWidth,
    ProfileHeight,
}

impl CliError {
    fn new(exit_code: CliExitCode, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
        }
    }
}

fn parse_command<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = OsString>,
{
    let collected: Vec<OsString> = args.into_iter().collect();
    let program = collected
        .first()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "morphos".to_owned());
    let Some(command_name) = collected
        .get(1)
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return Err(usage(&program));
    };

    if command_name == "tx" {
        return parse_tx_command(&collected, &program);
    }

    if command_name == "snapshot" {
        return parse_snapshot_command(&collected, &program);
    }

    let mut output_format = OutputFormat::Text;
    let mut workspace: Option<PathBuf> = None;
    let mut requested_output: Option<NodeId> = None;
    let mut destination: Option<PathBuf> = None;
    let mut mesh_format = MeshExportFormat::Obj;
    let mut overwrite = false;
    let mut file: Option<PathBuf> = None;
    let mut actor_filter: Option<TransactionActor> = None;
    let mut parameter_filter: Option<ParamId> = None;
    let mut node_filter: Option<NodeId> = None;
    let mut snapshot_name: Option<String> = None;
    let mut snapshot_id: Option<SnapshotId> = None;
    let mut index = 2usize;

    while index < collected.len() {
        let current = &collected[index];
        let flag = current.to_string_lossy();
        match flag.as_ref() {
            "--json" => {
                output_format = OutputFormat::Json;
                index += 1;
            }
            "--output" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!(
                        "missing value for `--output`\n\n{}",
                        usage(&program)
                    ));
                };
                let raw = value.to_string_lossy();
                let output = NodeId::new(raw.as_ref()).map_err(|error| {
                    format!(
                        "invalid output node `{raw}`: {error}\n\n{}",
                        usage(&program)
                    )
                })?;
                requested_output = Some(output);
                index += 2;
            }
            "--destination" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!(
                        "missing value for `--destination`\n\n{}",
                        usage(&program)
                    ));
                };
                destination = Some(PathBuf::from(value));
                index += 2;
            }
            "--format" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!(
                        "missing value for `--format`\n\n{}",
                        usage(&program)
                    ));
                };
                let raw = value.to_string_lossy();
                mesh_format = MeshExportFormat::parse(raw.as_ref())
                    .map_err(|error| format!("{error}\n\n{}", usage(&program)))?;
                index += 2;
            }
            "--overwrite" => {
                overwrite = true;
                index += 1;
            }
            "--file" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--file`\n\n{}", usage(&program)));
                };
                file = Some(PathBuf::from(value));
                index += 2;
            }
            "--actor" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!(
                        "missing value for `--actor`\n\n{}",
                        usage(&program)
                    ));
                };
                let raw = value.to_string_lossy();
                actor_filter = Some(
                    parse_actor_flag(raw.as_ref())
                        .map_err(|error| format!("{error}\n\n{}", usage(&program)))?,
                );
                index += 2;
            }
            "--node" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--node`\n\n{}", usage(&program)));
                };
                let raw = value.to_string_lossy();
                node_filter = Some(parse_node_id(raw.as_ref(), "--node", &program)?);
                index += 2;
            }
            "--parameter" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!(
                        "missing value for `--parameter`\n\n{}",
                        usage(&program)
                    ));
                };
                let raw = value.to_string_lossy();
                parameter_filter = Some(parse_param_id(raw.as_ref(), "--parameter", &program)?);
                index += 2;
            }
            "--name" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--name`\n\n{}", usage(&program)));
                };
                snapshot_name = Some(value.to_string_lossy().into_owned());
                index += 2;
            }
            "--id" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--id`\n\n{}", usage(&program)));
                };
                let raw = value.to_string_lossy();
                snapshot_id = Some(parse_snapshot_id(raw.as_ref(), &program)?);
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown flag `{value}`\n\n{}", usage(&program)));
            }
            _ => {
                if workspace.is_some() {
                    let duplicate = current.to_string_lossy();
                    return Err(format!(
                        "unexpected extra positional argument `{duplicate}`\n\n{}",
                        usage(&program)
                    ));
                }
                workspace = Some(PathBuf::from(current));
                index += 1;
            }
        }
    }

    let workspace = workspace.ok_or_else(|| {
        format!(
            "missing workspace path for `{command_name}`\n\n{}",
            usage(&program)
        )
    })?;

    match command_name.as_str() {
        "validate" => {
            if requested_output.is_some() {
                return Err(format!(
                    "`validate` does not accept `--output`\n\n{}",
                    usage(&program)
                ));
            }
            Ok(Command::Validate {
                workspace,
                format: output_format,
            })
        }
        "inspect" => {
            if requested_output.is_some() {
                return Err(format!(
                    "`inspect` does not accept `--output`\n\n{}",
                    usage(&program)
                ));
            }
            Ok(Command::Inspect {
                workspace,
                format: output_format,
            })
        }
        "eval" => Ok(Command::Eval {
            workspace,
            output: requested_output,
            format: output_format,
        }),
        "export" => Ok(Command::Export {
            workspace,
            output: requested_output,
            destination,
            mesh_format,
            overwrite,
            format: output_format,
        }),
        "history" => {
            if requested_output.is_some()
                || destination.is_some()
                || file.is_some()
                || snapshot_name.is_some()
                || snapshot_id.is_some()
            {
                return Err(format!(
                    "`history` received unsupported flags\n\n{}",
                    usage(&program)
                ));
            }
            Ok(Command::History {
                workspace,
                actor: actor_filter,
                node: node_filter,
                parameter: parameter_filter,
                format: output_format,
            })
        }
        "tx" => {
            let Some(mode) = collected
                .get(2)
                .map(|value| value.to_string_lossy().into_owned())
            else {
                return Err(format!("missing transaction mode\n\n{}", usage(&program)));
            };
            let workspace = collected.get(3).map(PathBuf::from).ok_or_else(|| {
                format!(
                    "missing workspace path for `tx {mode}`\n\n{}",
                    usage(&program)
                )
            })?;
            let mut tx_file: Option<PathBuf> = None;
            let mut tx_format = OutputFormat::Text;
            let mut tx_index = 4usize;
            while tx_index < collected.len() {
                let current = &collected[tx_index];
                let flag = current.to_string_lossy();
                match flag.as_ref() {
                    "--json" => {
                        tx_format = OutputFormat::Json;
                        tx_index += 1;
                    }
                    "--file" => {
                        let Some(value) = collected.get(tx_index + 1) else {
                            return Err(format!(
                                "missing value for `--file`\n\n{}",
                                usage(&program)
                            ));
                        };
                        tx_file = Some(PathBuf::from(value));
                        tx_index += 2;
                    }
                    value if value.starts_with("--") => {
                        return Err(format!("unknown flag `{value}`\n\n{}", usage(&program)));
                    }
                    other => {
                        return Err(format!(
                            "unexpected extra positional argument `{}`\n\n{}",
                            other,
                            usage(&program)
                        ));
                    }
                }
            }
            let file = tx_file.ok_or_else(|| {
                format!("missing `--file` for `tx {mode}`\n\n{}", usage(&program))
            })?;
            match mode.as_str() {
                "apply" => Ok(Command::TxApply {
                    workspace,
                    file,
                    format: tx_format,
                }),
                "dry-run" => Ok(Command::TxDryRun {
                    workspace,
                    file,
                    format: tx_format,
                }),
                _ => Err(format!(
                    "unknown transaction mode `{mode}`\n\n{}",
                    usage(&program)
                )),
            }
        }
        "snapshot" => {
            let Some(mode) = collected
                .get(2)
                .map(|value| value.to_string_lossy().into_owned())
            else {
                return Err(format!("missing snapshot mode\n\n{}", usage(&program)));
            };
            let workspace = collected.get(3).map(PathBuf::from).ok_or_else(|| {
                format!(
                    "missing workspace path for `snapshot {mode}`\n\n{}",
                    usage(&program)
                )
            })?;
            let mut snap_name: Option<String> = None;
            let mut snap_id: Option<SnapshotId> = None;
            let mut snap_format = OutputFormat::Text;
            let mut snap_index = 4usize;
            while snap_index < collected.len() {
                let current = &collected[snap_index];
                let flag = current.to_string_lossy();
                match flag.as_ref() {
                    "--json" => {
                        snap_format = OutputFormat::Json;
                        snap_index += 1;
                    }
                    "--name" => {
                        let Some(value) = collected.get(snap_index + 1) else {
                            return Err(format!(
                                "missing value for `--name`\n\n{}",
                                usage(&program)
                            ));
                        };
                        snap_name = Some(value.to_string_lossy().into_owned());
                        snap_index += 2;
                    }
                    "--id" => {
                        let Some(value) = collected.get(snap_index + 1) else {
                            return Err(format!("missing value for `--id`\n\n{}", usage(&program)));
                        };
                        snap_id = Some(parse_snapshot_id(&value.to_string_lossy(), &program)?);
                        snap_index += 2;
                    }
                    value if value.starts_with("--") => {
                        return Err(format!("unknown flag `{value}`\n\n{}", usage(&program)));
                    }
                    other => {
                        return Err(format!(
                            "unexpected extra positional argument `{}`\n\n{}",
                            other,
                            usage(&program)
                        ));
                    }
                }
            }
            match mode.as_str() {
                "create" => Ok(Command::SnapshotCreate {
                    workspace,
                    name: snap_name.ok_or_else(|| {
                        format!(
                            "missing `--name` for `snapshot create`\n\n{}",
                            usage(&program)
                        )
                    })?,
                    format: snap_format,
                }),
                "list" => Ok(Command::SnapshotList {
                    workspace,
                    format: snap_format,
                }),
                "restore" => Ok(Command::SnapshotRestore {
                    workspace,
                    snapshot_id: snap_id.ok_or_else(|| {
                        format!(
                            "missing `--id` for `snapshot restore`\n\n{}",
                            usage(&program)
                        )
                    })?,
                    format: snap_format,
                }),
                _ => Err(format!(
                    "unknown snapshot mode `{mode}`\n\n{}",
                    usage(&program)
                )),
            }
        }
        _ => Err(format!(
            "unknown command `{command_name}`\n\n{}",
            usage(&program)
        )),
    }
}

fn parse_tx_command(collected: &[OsString], program: &str) -> Result<Command, String> {
    let Some(mode) = collected
        .get(2)
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return Err(format!("missing transaction mode\n\n{}", usage(program)));
    };
    let workspace = collected.get(3).map(PathBuf::from).ok_or_else(|| {
        format!(
            "missing workspace path for `tx {mode}`\n\n{}",
            usage(program)
        )
    })?;
    let mut tx_file: Option<PathBuf> = None;
    let mut tx_format = OutputFormat::Text;
    let mut index = 4usize;
    while index < collected.len() {
        let current = &collected[index];
        let flag = current.to_string_lossy();
        match flag.as_ref() {
            "--json" => {
                tx_format = OutputFormat::Json;
                index += 1;
            }
            "--file" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--file`\n\n{}", usage(program)));
                };
                tx_file = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown flag `{value}`\n\n{}", usage(program)));
            }
            other => {
                return Err(format!(
                    "unexpected extra positional argument `{}`\n\n{}",
                    other,
                    usage(program)
                ));
            }
        }
    }
    let file =
        tx_file.ok_or_else(|| format!("missing `--file` for `tx {mode}`\n\n{}", usage(program)))?;
    match mode.as_str() {
        "apply" => Ok(Command::TxApply {
            workspace,
            file,
            format: tx_format,
        }),
        "dry-run" => Ok(Command::TxDryRun {
            workspace,
            file,
            format: tx_format,
        }),
        _ => Err(format!(
            "unknown transaction mode `{mode}`\n\n{}",
            usage(program)
        )),
    }
}

fn parse_snapshot_command(collected: &[OsString], program: &str) -> Result<Command, String> {
    let Some(mode) = collected
        .get(2)
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return Err(format!("missing snapshot mode\n\n{}", usage(program)));
    };
    let workspace = collected.get(3).map(PathBuf::from).ok_or_else(|| {
        format!(
            "missing workspace path for `snapshot {mode}`\n\n{}",
            usage(program)
        )
    })?;
    let mut snap_name: Option<String> = None;
    let mut snap_id: Option<SnapshotId> = None;
    let mut snap_format = OutputFormat::Text;
    let mut index = 4usize;
    while index < collected.len() {
        let current = &collected[index];
        let flag = current.to_string_lossy();
        match flag.as_ref() {
            "--json" => {
                snap_format = OutputFormat::Json;
                index += 1;
            }
            "--name" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--name`\n\n{}", usage(program)));
                };
                snap_name = Some(value.to_string_lossy().into_owned());
                index += 2;
            }
            "--id" => {
                let Some(value) = collected.get(index + 1) else {
                    return Err(format!("missing value for `--id`\n\n{}", usage(program)));
                };
                snap_id = Some(parse_snapshot_id(&value.to_string_lossy(), program)?);
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown flag `{value}`\n\n{}", usage(program)));
            }
            other => {
                return Err(format!(
                    "unexpected extra positional argument `{}`\n\n{}",
                    other,
                    usage(program)
                ));
            }
        }
    }
    match mode.as_str() {
        "create" => Ok(Command::SnapshotCreate {
            workspace,
            name: snap_name.ok_or_else(|| {
                format!(
                    "missing `--name` for `snapshot create`\n\n{}",
                    usage(program)
                )
            })?,
            format: snap_format,
        }),
        "list" => Ok(Command::SnapshotList {
            workspace,
            format: snap_format,
        }),
        "restore" => Ok(Command::SnapshotRestore {
            workspace,
            snapshot_id: snap_id.ok_or_else(|| {
                format!(
                    "missing `--id` for `snapshot restore`\n\n{}",
                    usage(program)
                )
            })?,
            format: snap_format,
        }),
        _ => Err(format!(
            "unknown snapshot mode `{mode}`\n\n{}",
            usage(program)
        )),
    }
}

fn usage(program: &str) -> String {
    format!(
        "Usage:\n  {program} validate <workspace> [--json]\n  {program} inspect <workspace> [--json]\n  {program} eval <workspace> [--output <node-id>] [--json]\n  {program} export <workspace> [--output <node-id>] [--format obj|stl] [--destination <relative-path>] [--overwrite] [--json]\n  {program} tx apply <workspace> --file <transaction.json> [--json]\n  {program} tx dry-run <workspace> --file <transaction.json> [--json]\n  {program} history <workspace> [--actor <actor>] [--node <node-id>] [--parameter <param-id>] [--json]\n  {program} snapshot create <workspace> --name <snapshot-name> [--json]\n  {program} snapshot list <workspace> [--json]\n  {program} snapshot restore <workspace> --id <snapshot-id> [--json]"
    )
}

fn execute_command(command: Command) -> Result<String, CliError> {
    match command {
        Command::Validate { workspace, format } => {
            let workspace = open_workspace(&workspace)?;
            let scene = parse_workspace_scene(&workspace)?;
            render_output(
                format,
                render_validate_text(&workspace, &scene),
                json!({
                    "command": "validate",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "scene": scene_json(&scene),
                }),
            )
        }
        Command::Inspect { workspace, format } => {
            let workspace = open_workspace(&workspace)?;
            let scene = parse_workspace_scene(&workspace)?;
            render_output(
                format,
                render_inspect_text(&workspace, &scene),
                json!({
                    "command": "inspect",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "scene": scene_json(&scene),
                }),
            )
        }
        Command::Eval {
            workspace,
            output,
            format,
        } => {
            let workspace = open_workspace(&workspace)?;
            let scene = parse_workspace_scene(&workspace)?;
            let evaluation = evaluate_scene(&scene, output.as_ref())?;
            render_output(
                format,
                render_eval_text(&workspace, &scene, &evaluation),
                json!({
                    "command": "eval",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "scene": scene_json(&scene),
                    "evaluation": evaluated_geometry_json(&evaluation),
                }),
            )
        }
        Command::Export {
            workspace,
            output,
            destination,
            mesh_format,
            overwrite,
            format,
        } => {
            let workspace = open_workspace(&workspace)?;
            let scene = parse_workspace_scene(&workspace)?;
            let evaluation = evaluate_scene(&scene, output.as_ref())?;
            let export = export_mesh(
                &workspace,
                &evaluation,
                destination.as_deref(),
                mesh_format,
                overwrite,
            )?;
            render_output(
                format,
                render_export_text(&workspace, &scene, &evaluation, &export),
                json!({
                    "command": "export",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "scene": scene_json(&scene),
                    "evaluation": evaluated_geometry_json(&evaluation),
                    "export": export_record_json(&export),
                }),
            )
        }
        Command::TxApply {
            workspace,
            file,
            format,
        } => {
            let mut workspace = open_workspace(&workspace)?;
            let transaction = load_transaction_file(&file)?;
            let record = execute_transaction_apply(&mut workspace, &transaction)?;
            render_output(
                format,
                render_transaction_text(&workspace, &record),
                json!({
                    "command": "tx_apply",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "transaction": transaction_execution_json(&record),
                }),
            )
        }
        Command::TxDryRun {
            workspace,
            file,
            format,
        } => {
            let workspace = open_workspace(&workspace)?;
            let transaction = load_transaction_file(&file)?;
            let record = execute_transaction_dry_run(&workspace, &transaction)?;
            render_output(
                format,
                render_transaction_text(&workspace, &record),
                json!({
                    "command": "tx_dry_run",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "transaction": transaction_execution_json(&record),
                }),
            )
        }
        Command::History {
            workspace,
            actor,
            node,
            parameter,
            format,
        } => {
            let workspace = open_workspace(&workspace)?;
            let entries = read_history(&workspace, actor, node, parameter)?;
            render_output(
                format,
                render_history_text(&workspace, &entries),
                json!({
                    "command": "history",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "history": history_entries_json(&entries),
                }),
            )
        }
        Command::SnapshotCreate {
            workspace,
            name,
            format,
        } => {
            let workspace = open_workspace(&workspace)?;
            let snapshot = workspace
                .create_snapshot(&name, TransactionActor::CliAutomation)
                .map_err(|error| {
                    CliError::new(
                        CliExitCode::Io,
                        format!("snapshot creation failed: {error}"),
                    )
                })?;
            render_output(
                format,
                render_snapshot_create_text(&workspace, &snapshot),
                json!({
                    "command": "snapshot_create",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "snapshot": snapshot_json(&snapshot),
                }),
            )
        }
        Command::SnapshotList { workspace, format } => {
            let workspace = open_workspace(&workspace)?;
            let snapshots = workspace.snapshots().map_err(|error| {
                CliError::new(CliExitCode::Io, format!("snapshot listing failed: {error}"))
            })?;
            render_output(
                format,
                render_snapshot_list_text(&workspace, &snapshots),
                json!({
                    "command": "snapshot_list",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "snapshots": snapshots.iter().map(snapshot_json).collect::<Vec<_>>(),
                }),
            )
        }
        Command::SnapshotRestore {
            workspace,
            snapshot_id,
            format,
        } => {
            let mut workspace = open_workspace(&workspace)?;
            let record = execute_snapshot_restore(&mut workspace, &snapshot_id)?;
            render_output(
                format,
                render_transaction_text(&workspace, &record),
                json!({
                    "command": "snapshot_restore",
                    "status": "ok",
                    "workspace": workspace_json(&workspace),
                    "transaction": transaction_execution_json(&record),
                }),
            )
        }
    }
}

fn parse_actor_flag(raw: &str) -> Result<TransactionActor, String> {
    match raw {
        "user" => Ok(TransactionActor::User),
        "ai" => Ok(TransactionActor::Ai),
        "cli-automation" => Ok(TransactionActor::CliAutomation),
        "system-migration" => Ok(TransactionActor::SystemMigration),
        _ => Err(format!(
            "unsupported actor `{raw}`; supported actors: user, ai, cli-automation, system-migration"
        )),
    }
}

fn parse_node_id(raw: &str, flag: &str, program: &str) -> Result<NodeId, String> {
    NodeId::new(raw).map_err(|error| {
        format!(
            "invalid node ID for `{flag}`: {error}\n\n{}",
            usage(program)
        )
    })
}

fn parse_param_id(raw: &str, flag: &str, program: &str) -> Result<ParamId, String> {
    ParamId::new(raw).map_err(|error| {
        format!(
            "invalid parameter ID for `{flag}`: {error}\n\n{}",
            usage(program)
        )
    })
}

fn parse_snapshot_id(raw: &str, program: &str) -> Result<SnapshotId, String> {
    serde_json::from_value::<SnapshotId>(Value::String(raw.to_owned()))
        .map_err(|error| format!("invalid snapshot ID `{raw}`: {error}\n\n{}", usage(program)))
}

fn load_transaction_file(path: &Path) -> Result<WorkspaceTransaction, CliError> {
    let text = fs::read_to_string(path).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!(
                "failed to read transaction file `{}`: {error}",
                path.display()
            ),
        )
    })?;
    let request: TransactionRequestFile = serde_json::from_str(&text).map_err(|error| {
        CliError::new(
            CliExitCode::Usage,
            format!(
                "failed to parse transaction file `{}` as JSON: {error}",
                path.display()
            ),
        )
    })?;
    let actor = request
        .actor
        .unwrap_or(TransactionActorRequest::CliAutomation)
        .into_actor();
    let operations = request
        .operations
        .into_iter()
        .map(WorkspaceOpRequest::into_workspace_op)
        .collect::<Result<Vec<_>, _>>()?;
    WorkspaceTransaction::new(actor, request.intent, operations).map_err(|error| {
        CliError::new(
            CliExitCode::Usage,
            format!("transaction file `{}` is invalid: {error}", path.display()),
        )
    })
}

impl WorkspaceOpRequest {
    fn into_workspace_op(self) -> Result<WorkspaceOp, CliError> {
        match self {
            Self::ReplaceScene { scene_source } => {
                let scene = parse_scene(&scene_source).map_err(|error| {
                    CliError::new(
                        CliExitCode::Usage,
                        format!("replace_scene scene_source is invalid: {error}"),
                    )
                })?;
                Ok(WorkspaceOp::ReplaceScene {
                    id: OperationId::new(),
                    scene: Box::new(scene),
                })
            }
            Self::AddNode { node_id, draft } => Ok(WorkspaceOp::AddNode {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "add_node.node_id")?,
                draft: draft.into_scene_node_draft()?,
            }),
            Self::ReplaceNode {
                node_id,
                scene_source,
            } => Ok(WorkspaceOp::ReplaceNode {
                id: OperationId::new(),
                node: Box::new(extract_node_from_scene_source(
                    &scene_source,
                    &node_id,
                    "replace_node.scene_source",
                )?),
            }),
            Self::DeleteNode { node_id } => Ok(WorkspaceOp::DeleteNode {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "delete_node.node_id")?,
            }),
            Self::RenameNode { from, to } => Ok(WorkspaceOp::RenameNode {
                id: OperationId::new(),
                from: parse_workspace_node_id(&from, "rename_node.from")?,
                to: parse_workspace_node_id(&to, "rename_node.to")?,
            }),
            Self::DuplicateNode {
                source_node,
                duplicate,
            } => Ok(WorkspaceOp::DuplicateNode {
                id: OperationId::new(),
                source_node: parse_workspace_node_id(&source_node, "duplicate_node.source_node")?,
                duplicate: parse_workspace_node_id(&duplicate, "duplicate_node.duplicate")?,
            }),
            Self::SetNodeLabel { node_id, label } => Ok(WorkspaceOp::SetNodeLabel {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "set_node_label.node_id")?,
                label,
            }),
            Self::SetCompositionChildren { node_id, children } => {
                Ok(WorkspaceOp::SetCompositionChildren {
                    id: OperationId::new(),
                    node_id: parse_workspace_node_id(&node_id, "set_composition_children.node_id")?,
                    children: children
                        .into_iter()
                        .map(|child| {
                            parse_workspace_node_id(&child, "set_composition_children.children")
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            }
            Self::SetParameterScalar {
                parameter_id,
                value,
            } => Ok(WorkspaceOp::SetParameterScalar {
                id: OperationId::new(),
                parameter_id: parse_workspace_param_id(
                    &parameter_id,
                    "set_parameter_scalar.parameter_id",
                )?,
                value,
            }),
            Self::SetTransformComponent {
                node_id,
                property,
                axis,
                value,
            } => Ok(WorkspaceOp::SetTransformComponent {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "set_transform_component.node_id")?,
                property: property.into_transform_property(),
                axis: axis.into_axis(),
                value,
            }),
            Self::SetPrimitiveScalar {
                node_id,
                field,
                value,
            } => Ok(WorkspaceOp::SetPrimitiveScalar {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "set_primitive_scalar.node_id")?,
                field: field.into_primitive_scalar_field(),
                value,
            }),
            Self::SetRootNode { node_id } => Ok(WorkspaceOp::SetRootNode {
                id: OperationId::new(),
                node_id: parse_workspace_node_id(&node_id, "set_root_node.node_id")?,
            }),
        }
    }
}

impl SceneNodeDraftRequest {
    fn into_scene_node_draft(self) -> Result<geom_scene::SceneNodeDraft, CliError> {
        Ok(match self {
            Self::Box => geom_scene::SceneNodeDraft::Box,
            Self::Sphere => geom_scene::SceneNodeDraft::Sphere,
            Self::Cylinder => geom_scene::SceneNodeDraft::Cylinder,
            Self::Capsule => geom_scene::SceneNodeDraft::Capsule,
            Self::Plane => geom_scene::SceneNodeDraft::Plane,
            Self::Profile => geom_scene::SceneNodeDraft::Profile,
            Self::Union { children } => geom_scene::SceneNodeDraft::Union {
                children: children
                    .into_iter()
                    .map(|child| parse_workspace_node_id(&child, "scene_node_draft.union.children"))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Difference { children } => geom_scene::SceneNodeDraft::Difference {
                children: children
                    .into_iter()
                    .map(|child| {
                        parse_workspace_node_id(&child, "scene_node_draft.difference.children")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            Self::Intersection { children } => geom_scene::SceneNodeDraft::Intersection {
                children: children
                    .into_iter()
                    .map(|child| {
                        parse_workspace_node_id(&child, "scene_node_draft.intersection.children")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })
    }
}

impl TransformPropertyRequest {
    const fn into_transform_property(self) -> geom_scene::TransformProperty {
        match self {
            Self::Translation => geom_scene::TransformProperty::Translation,
            Self::RotationDeg => geom_scene::TransformProperty::RotationDegrees,
            Self::Scale => geom_scene::TransformProperty::Scale,
        }
    }
}

impl AxisRequest {
    const fn into_axis(self) -> geom_scene::Axis {
        match self {
            Self::X => geom_scene::Axis::X,
            Self::Y => geom_scene::Axis::Y,
            Self::Z => geom_scene::Axis::Z,
        }
    }
}

impl PrimitiveScalarFieldRequest {
    const fn into_primitive_scalar_field(self) -> geom_scene::PrimitiveScalarField {
        match self {
            Self::BoxX => geom_scene::PrimitiveScalarField::BoxX,
            Self::BoxY => geom_scene::PrimitiveScalarField::BoxY,
            Self::BoxZ => geom_scene::PrimitiveScalarField::BoxZ,
            Self::SphereRadius => geom_scene::PrimitiveScalarField::SphereRadius,
            Self::CylinderRadius => geom_scene::PrimitiveScalarField::CylinderRadius,
            Self::CylinderHeight => geom_scene::PrimitiveScalarField::CylinderHeight,
            Self::CapsuleRadius => geom_scene::PrimitiveScalarField::CapsuleRadius,
            Self::CapsuleHeight => geom_scene::PrimitiveScalarField::CapsuleHeight,
            Self::PlaneWidth => geom_scene::PrimitiveScalarField::PlaneWidth,
            Self::PlaneDepth => geom_scene::PrimitiveScalarField::PlaneDepth,
            Self::ProfileWidth => geom_scene::PrimitiveScalarField::ProfileWidth,
            Self::ProfileHeight => geom_scene::PrimitiveScalarField::ProfileHeight,
        }
    }
}

fn parse_workspace_node_id(raw: &str, field: &str) -> Result<NodeId, CliError> {
    NodeId::new(raw).map_err(|error| {
        CliError::new(
            CliExitCode::Usage,
            format!("invalid node ID for `{field}`: {error}"),
        )
    })
}

fn parse_workspace_param_id(raw: &str, field: &str) -> Result<ParamId, CliError> {
    ParamId::new(raw).map_err(|error| {
        CliError::new(
            CliExitCode::Usage,
            format!("invalid parameter ID for `{field}`: {error}"),
        )
    })
}

fn extract_node_from_scene_source(
    scene_source: &str,
    node_id: &str,
    field: &str,
) -> Result<Node, CliError> {
    let scene = parse_scene(scene_source).map_err(|error| {
        CliError::new(
            CliExitCode::Usage,
            format!("invalid scene source for `{field}`: {error}"),
        )
    })?;
    let node_id = parse_workspace_node_id(node_id, "replace_node.node_id")?;
    scene.nodes().get(&node_id).cloned().ok_or_else(|| {
        CliError::new(
            CliExitCode::Usage,
            format!("scene source for `{field}` does not contain node `{node_id}`"),
        )
    })
}

fn execute_transaction_apply(
    workspace: &mut Workspace,
    transaction: &WorkspaceTransaction,
) -> Result<TransactionExecutionRecord, CliError> {
    let commit = workspace.apply_transaction(transaction).map_err(|error| {
        CliError::new(
            CliExitCode::Source,
            format!("transaction apply failed: {error}"),
        )
    })?;
    let diff = workspace
        .transaction_diff(commit.transaction_id())
        .map_err(|error| {
            CliError::new(
                CliExitCode::Io,
                format!("failed to read transaction diff: {error}"),
            )
        })?;
    Ok(TransactionExecutionRecord {
        mode: "apply",
        commit,
        diff,
    })
}

fn execute_transaction_dry_run(
    workspace: &Workspace,
    transaction: &WorkspaceTransaction,
) -> Result<TransactionExecutionRecord, CliError> {
    let temp_root = clone_workspace_to_temp(workspace.root())?;
    let mut temp_workspace = open_workspace(&temp_root)?;
    let result = execute_transaction_apply(&mut temp_workspace, transaction);
    let _ = fs::remove_dir_all(&temp_root);
    let mut record = result?;
    record.mode = "dry-run";
    Ok(record)
}

fn execute_snapshot_restore(
    workspace: &mut Workspace,
    snapshot_id: &SnapshotId,
) -> Result<TransactionExecutionRecord, CliError> {
    let commit = workspace
        .restore_snapshot(snapshot_id, TransactionActor::CliAutomation)
        .map_err(|error| {
            CliError::new(
                CliExitCode::Source,
                format!("snapshot restore failed: {error}"),
            )
        })?;
    let diff = workspace
        .transaction_diff(commit.transaction_id())
        .map_err(|error| {
            CliError::new(
                CliExitCode::Io,
                format!("failed to read transaction diff: {error}"),
            )
        })?;
    Ok(TransactionExecutionRecord {
        mode: "snapshot-restore",
        commit,
        diff,
    })
}

fn clone_workspace_to_temp(source_root: &Path) -> Result<PathBuf, CliError> {
    let temp_root = std::env::temp_dir().join(format!(
        "morphos-cli-dry-run-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    copy_directory_recursive(source_root, &temp_root)?;
    Ok(temp_root)
}

fn copy_directory_recursive(source: &Path, target: &Path) -> Result<(), CliError> {
    fs::create_dir_all(target).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!("failed to create directory `{}`: {error}", target.display()),
        )
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!("failed to read directory `{}`: {error}", source.display()),
        )
    })? {
        let entry = entry.map_err(|error| {
            CliError::new(
                CliExitCode::Io,
                format!("failed to read entry under `{}`: {error}", source.display()),
            )
        })?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| {
                CliError::new(
                    CliExitCode::Io,
                    format!("failed to inspect `{}`: {error}", source_path.display()),
                )
            })?
            .is_dir()
        {
            copy_directory_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).map_err(|error| {
                CliError::new(
                    CliExitCode::Io,
                    format!(
                        "failed to copy `{}` to `{}`: {error}",
                        source_path.display(),
                        target_path.display()
                    ),
                )
            })?;
        }
    }
    Ok(())
}

fn read_history(
    workspace: &Workspace,
    actor: Option<TransactionActor>,
    node: Option<NodeId>,
    parameter: Option<ParamId>,
) -> Result<Vec<WorkspaceHistoryEntry>, CliError> {
    let entries = if actor.is_none() && node.is_none() && parameter.is_none() {
        workspace.history_entries().map_err(|error| {
            CliError::new(CliExitCode::Io, format!("history query failed: {error}"))
        })?
    } else {
        let mut query = HistoryQuery::default();
        if let Some(actor) = actor {
            query = query.with_actor(actor);
        }
        if let Some(node) = node {
            query = query.with_node(node);
        }
        if let Some(parameter) = parameter {
            query = query.with_parameter(parameter);
        }
        workspace.query_history(&query).map_err(|error| {
            CliError::new(CliExitCode::Io, format!("history query failed: {error}"))
        })?
    };
    Ok(entries)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MeshExportRecord {
    format: MeshExportFormat,
    relative_path: PathBuf,
    absolute_path: PathBuf,
    byte_count: u64,
    vertex_count: usize,
    triangle_count: usize,
}

fn open_workspace(path: &Path) -> Result<Workspace, CliError> {
    Workspace::open(path).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!("failed to open workspace `{}`: {error}", path.display()),
        )
    })
}

fn parse_workspace_scene(workspace: &Workspace) -> Result<SceneDocument, CliError> {
    parse_scene(workspace.source_text()).map_err(|error| {
        CliError::new(
            CliExitCode::Source,
            format!(
                "scene validation failed for `{}`: {error}",
                workspace.root().display()
            ),
        )
    })
}

fn evaluate_scene(
    scene: &SceneDocument,
    output: Option<&NodeId>,
) -> Result<EvaluatedGeometry, CliError> {
    let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
    match output {
        Some(node) => evaluator.evaluate_node(scene, node),
        None => evaluator.evaluate_root(scene),
    }
    .map_err(|error| {
        CliError::new(
            CliExitCode::Geometry,
            format!("geometry evaluation failed: {error}"),
        )
    })
}

fn render_output(format: OutputFormat, text: String, value: Value) -> Result<String, CliError> {
    match format {
        OutputFormat::Text => Ok(text),
        OutputFormat::Json => serde_json::to_string_pretty(&value).map_err(|error| {
            CliError::new(
                CliExitCode::Internal,
                format!("failed to render JSON output: {error}"),
            )
        }),
    }
}

fn export_mesh(
    workspace: &Workspace,
    evaluation: &EvaluatedGeometry,
    destination: Option<&Path>,
    mesh_format: MeshExportFormat,
    overwrite: bool,
) -> Result<MeshExportRecord, CliError> {
    let relative_path = match destination {
        Some(path) => workspace
            .resolve_path(WorkspaceDirectory::Exports, path)
            .map_err(|error| {
                CliError::new(
                    CliExitCode::Io,
                    format!("invalid export destination `{}`: {error}", path.display()),
                )
            })?
            .strip_prefix(workspace.paths().exports_dir())
            .expect("resolved export path stays under exports dir")
            .to_path_buf(),
        None => default_export_relative_path(&evaluation.requested_output, mesh_format),
    };
    let absolute_path = workspace
        .resolve_path(WorkspaceDirectory::Exports, &relative_path)
        .map_err(|error| {
            CliError::new(
                CliExitCode::Io,
                format!(
                    "failed to resolve export path `{}`: {error}",
                    relative_path.display()
                ),
            )
        })?;

    if absolute_path.exists() && !overwrite {
        return Err(CliError::new(
            CliExitCode::Io,
            format!(
                "export destination already exists at `{}`; pass `--overwrite` to replace it",
                absolute_path.display()
            ),
        ));
    }

    let text = render_mesh_export(
        &evaluation.mesh,
        mesh_format,
        &evaluation.requested_output,
        &evaluation.bounds,
    );
    let parent = absolute_path.parent().ok_or_else(|| {
        CliError::new(
            CliExitCode::Internal,
            format!(
                "export destination `{}` has no parent directory",
                absolute_path.display()
            ),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!(
                "failed to create export directory `{}`: {error}",
                parent.display()
            ),
        )
    })?;
    fs::write(&absolute_path, text.as_bytes()).map_err(|error| {
        CliError::new(
            CliExitCode::Io,
            format!(
                "failed to write export `{}`: {error}",
                absolute_path.display()
            ),
        )
    })?;
    let byte_count = fs::metadata(&absolute_path)
        .map_err(|error| {
            CliError::new(
                CliExitCode::Io,
                format!(
                    "failed to read export metadata `{}`: {error}",
                    absolute_path.display()
                ),
            )
        })?
        .len();

    Ok(MeshExportRecord {
        format: mesh_format,
        relative_path,
        absolute_path,
        byte_count,
        vertex_count: evaluation.mesh.positions().len(),
        triangle_count: evaluation.mesh.triangle_indices().len(),
    })
}

fn default_export_relative_path(output: &NodeId, mesh_format: MeshExportFormat) -> PathBuf {
    PathBuf::from(format!("{}.{}", output.as_str(), mesh_format.extension()))
}

fn render_mesh_export(
    mesh: &Mesh,
    mesh_format: MeshExportFormat,
    requested_output: &NodeId,
    bounds: &Bounds,
) -> String {
    match mesh_format {
        MeshExportFormat::Obj => render_obj_export(mesh, requested_output, bounds),
        MeshExportFormat::Stl => render_stl_export(mesh, requested_output, bounds),
    }
}

fn render_obj_export(mesh: &Mesh, requested_output: &NodeId, bounds: &Bounds) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "# Morphos OBJ export");
    let _ = writeln!(&mut output, "# requested_output: {}", requested_output);
    let _ = writeln!(&mut output, "# vertex_count: {}", mesh.positions().len());
    let _ = writeln!(
        &mut output,
        "# triangle_count: {}",
        mesh.triangle_indices().len()
    );
    let _ = writeln!(&mut output, "# bounds: {}", render_bounds_text(bounds));
    let _ = writeln!(&mut output, "o {}", requested_output);
    for position in mesh.positions() {
        let _ = writeln!(
            &mut output,
            "v {:.9} {:.9} {:.9}",
            position[0], position[1], position[2]
        );
    }
    for triangle in mesh.triangle_indices() {
        let _ = writeln!(
            &mut output,
            "f {} {} {}",
            triangle[0] + 1,
            triangle[1] + 1,
            triangle[2] + 1
        );
    }
    output
}

fn render_stl_export(mesh: &Mesh, requested_output: &NodeId, bounds: &Bounds) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "solid {}", requested_output);
    let _ = writeln!(&mut output, "  // Morphos STL export");
    let _ = writeln!(&mut output, "  // bounds {}", render_bounds_text(bounds));
    for triangle in mesh.triangle_indices() {
        let vertices = [
            mesh.positions()[triangle[0] as usize],
            mesh.positions()[triangle[1] as usize],
            mesh.positions()[triangle[2] as usize],
        ];
        let normal = triangle_normal(vertices[0], vertices[1], vertices[2]);
        let _ = writeln!(
            &mut output,
            "  facet normal {:.9} {:.9} {:.9}",
            normal[0], normal[1], normal[2]
        );
        let _ = writeln!(&mut output, "    outer loop");
        for vertex in vertices {
            let _ = writeln!(
                &mut output,
                "      vertex {:.9} {:.9} {:.9}",
                vertex[0], vertex[1], vertex[2]
            );
        }
        let _ = writeln!(&mut output, "    endloop");
        let _ = writeln!(&mut output, "  endfacet");
    }
    let _ = write!(&mut output, "endsolid {}", requested_output);
    output
}

fn triangle_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let magnitude = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if magnitude <= f64::EPSILON {
        [0.0, 0.0, 0.0]
    } else {
        [
            cross[0] / magnitude,
            cross[1] / magnitude,
            cross[2] / magnitude,
        ]
    }
}

fn render_validate_text(workspace: &Workspace, scene: &SceneDocument) -> String {
    format!(
        "Workspace `{}` is valid.\nPath: {}\nRevision: {}\nRoot output: {}\nNodes: {}\nParameters: {}",
        workspace.summary().name(),
        workspace.root().display(),
        workspace.revision(),
        scene.root(),
        scene.nodes().len(),
        scene.parameters().len(),
    )
}

fn render_inspect_text(workspace: &Workspace, scene: &SceneDocument) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "Workspace: {}", workspace.summary().name());
    let _ = writeln!(&mut output, "Path: {}", workspace.root().display());
    let _ = writeln!(
        &mut output,
        "Workspace ID: {}",
        workspace.summary().workspace_id()
    );
    let _ = writeln!(&mut output, "Revision: {}", workspace.revision());
    let _ = writeln!(&mut output, "Dirty: {}", workspace.is_dirty());
    let _ = writeln!(&mut output, "Scene root: {}", scene.root());
    let _ = writeln!(&mut output, "Node count: {}", scene.nodes().len());
    let _ = writeln!(&mut output, "Parameter count: {}", scene.parameters().len());
    let _ = writeln!(
        &mut output,
        "Nodes: {}",
        join_node_ids(scene.nodes().keys())
    );
    let _ = write!(
        &mut output,
        "Parameters: {}",
        join_param_ids(scene.parameters().keys())
    );
    output
}

fn render_eval_text(
    workspace: &Workspace,
    scene: &SceneDocument,
    evaluation: &EvaluatedGeometry,
) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "Workspace: {}", workspace.summary().name());
    let _ = writeln!(&mut output, "Path: {}", workspace.root().display());
    let _ = writeln!(&mut output, "Scene root: {}", scene.root());
    let _ = writeln!(
        &mut output,
        "Requested output: {}",
        evaluation.requested_output
    );
    let _ = writeln!(
        &mut output,
        "Evaluation revision: {}",
        evaluation.evaluation_revision
    );
    let _ = writeln!(&mut output, "Vertices: {}", evaluation.stats.vertex_count);
    let _ = writeln!(
        &mut output,
        "Triangles: {}",
        evaluation.stats.triangle_count
    );
    let _ = writeln!(
        &mut output,
        "Evaluated nodes: {}",
        evaluation.stats.evaluated_node_count
    );
    let _ = writeln!(&mut output, "Cache hits: {}", evaluation.stats.cache_hits);
    let _ = writeln!(
        &mut output,
        "Cache misses: {}",
        evaluation.stats.cache_misses
    );
    let _ = writeln!(
        &mut output,
        "Participating nodes: {}",
        join_node_ids(evaluation.participating_node_ids.iter())
    );
    let _ = writeln!(
        &mut output,
        "Resolved parameters: {}",
        join_param_ids(evaluation.resolved_parameters.keys())
    );
    let _ = write!(
        &mut output,
        "Bounds: {}",
        render_bounds_text(&evaluation.bounds)
    );
    output
}

fn render_export_text(
    workspace: &Workspace,
    scene: &SceneDocument,
    evaluation: &EvaluatedGeometry,
    export: &MeshExportRecord,
) -> String {
    let mut output = render_eval_text(workspace, scene, evaluation);
    let _ = writeln!(&mut output);
    let _ = writeln!(&mut output, "Export format: {}", export.format.as_str());
    let _ = writeln!(
        &mut output,
        "Export path: {}",
        export.absolute_path.display()
    );
    let _ = writeln!(
        &mut output,
        "Export relative path: {}",
        export.relative_path.display()
    );
    let _ = write!(&mut output, "Export bytes: {}", export.byte_count);
    output
}

fn render_transaction_text(workspace: &Workspace, record: &TransactionExecutionRecord) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "Workspace: {}", workspace.summary().name());
    let _ = writeln!(&mut output, "Path: {}", workspace.root().display());
    let _ = writeln!(&mut output, "Mode: {}", record.mode);
    let _ = writeln!(
        &mut output,
        "Transaction ID: {}",
        record.commit.transaction_id()
    );
    let _ = writeln!(&mut output, "Actor: {}", actor_label(record.commit.actor()));
    let _ = writeln!(
        &mut output,
        "Intent: {}",
        record.commit.intent().unwrap_or("(none)")
    );
    let _ = writeln!(
        &mut output,
        "Revision: {} -> {}",
        record.commit.revision_before(),
        record.commit.revision_after()
    );
    let _ = writeln!(
        &mut output,
        "Affected nodes: {}",
        join_node_ids(record.commit.affected_targets().node_ids().iter())
    );
    let _ = writeln!(
        &mut output,
        "Affected parameters: {}",
        join_param_ids(record.commit.affected_targets().parameter_ids().iter())
    );
    if let Some(diff) = &record.diff {
        let _ = writeln!(&mut output, "Diff summary: {}", diff.summary());
        let _ = write!(&mut output, "Diff changes: {}", render_diff_changes(diff));
    }
    output
}

fn render_history_text(workspace: &Workspace, entries: &[WorkspaceHistoryEntry]) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "Workspace: {}", workspace.summary().name());
    let _ = writeln!(&mut output, "Path: {}", workspace.root().display());
    let _ = writeln!(&mut output, "Entries: {}", entries.len());
    for entry in entries {
        let _ = writeln!(
            &mut output,
            "- rev {} -> {} | {} | {} | {}",
            entry.revision_before(),
            entry.revision_after(),
            actor_label(entry.actor()),
            entry.transaction_id(),
            entry.intent().unwrap_or("(no intent)")
        );
    }
    output
}

fn render_snapshot_create_text(workspace: &Workspace, snapshot: &WorkspaceSnapshot) -> String {
    format!(
        "Workspace: {}\nPath: {}\nCreated snapshot: {}\nSnapshot ID: {}\nActor: {}\nCreated from revision: {}",
        workspace.summary().name(),
        workspace.root().display(),
        snapshot.name(),
        snapshot.id(),
        actor_label(snapshot.actor()),
        snapshot.created_from_revision()
    )
}

fn render_snapshot_list_text(workspace: &Workspace, snapshots: &[WorkspaceSnapshot]) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "Workspace: {}", workspace.summary().name());
    let _ = writeln!(&mut output, "Path: {}", workspace.root().display());
    let _ = writeln!(&mut output, "Snapshots: {}", snapshots.len());
    for snapshot in snapshots {
        let _ = writeln!(
            &mut output,
            "- {} | {} | {} | rev {}",
            snapshot.id(),
            snapshot.name(),
            actor_label(snapshot.actor()),
            snapshot.created_from_revision()
        );
    }
    output
}

fn workspace_json(workspace: &Workspace) -> Value {
    let summary = workspace.summary();
    json!({
        "path": workspace.root(),
        "name": summary.name(),
        "workspace_id": summary.workspace_id().to_string(),
        "format_version": summary.format_version(),
        "revision": summary.revision().get(),
        "is_dirty": summary.is_dirty(),
    })
}

fn scene_json(scene: &SceneDocument) -> Value {
    let mut node_ids: Vec<_> = scene.nodes().keys().map(|node| node.to_string()).collect();
    node_ids.sort();
    let mut parameter_ids: Vec<_> = scene
        .parameters()
        .keys()
        .map(|parameter| parameter.to_string())
        .collect();
    parameter_ids.sort();

    json!({
        "root": scene.root().to_string(),
        "node_count": scene.nodes().len(),
        "parameter_count": scene.parameters().len(),
        "node_ids": node_ids,
        "parameter_ids": parameter_ids,
    })
}

fn evaluated_geometry_json(evaluation: &EvaluatedGeometry) -> Value {
    let resolved_parameters: Vec<_> = evaluation
        .resolved_parameters
        .values()
        .map(|parameter| {
            json!({
                "id": parameter.id().to_string(),
                "value": parameter.value(),
            })
        })
        .collect();
    let mut participating_nodes: Vec<_> = evaluation
        .participating_node_ids
        .iter()
        .map(ToString::to_string)
        .collect();
    participating_nodes.sort();

    json!({
        "requested_output": evaluation.requested_output.to_string(),
        "evaluation_revision": evaluation.evaluation_revision,
        "mesh": {
            "vertex_count": evaluation.mesh.positions().len(),
            "triangle_count": evaluation.mesh.triangle_indices().len(),
        },
        "bounds": bounds_json(&evaluation.bounds),
        "stats": {
            "vertex_count": evaluation.stats.vertex_count,
            "triangle_count": evaluation.stats.triangle_count,
            "evaluated_node_count": evaluation.stats.evaluated_node_count,
            "cache_hits": evaluation.stats.cache_hits,
            "cache_misses": evaluation.stats.cache_misses,
        },
        "resolved_parameters": resolved_parameters,
        "participating_node_ids": participating_nodes,
    })
}

fn export_record_json(export: &MeshExportRecord) -> Value {
    json!({
        "format": export.format.as_str(),
        "relative_path": portable_path_string(&export.relative_path),
        "absolute_path": export.absolute_path,
        "byte_count": export.byte_count,
        "vertex_count": export.vertex_count,
        "triangle_count": export.triangle_count,
    })
}

fn transaction_execution_json(record: &TransactionExecutionRecord) -> Value {
    json!({
        "mode": record.mode,
        "transaction_id": record.commit.transaction_id().to_string(),
        "actor": actor_label(record.commit.actor()),
        "intent": record.commit.intent(),
        "revision_before": record.commit.revision_before().get(),
        "revision_after": record.commit.revision_after().get(),
        "operation_ids": record.commit.operation_ids().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "affected_targets": affected_targets_json(record.commit.affected_targets()),
        "diff": record.diff.as_ref().map(diff_json).unwrap_or(Value::Null),
    })
}

fn history_entries_json(entries: &[WorkspaceHistoryEntry]) -> Value {
    Value::Array(entries.iter().map(history_entry_json).collect())
}

fn history_entry_json(entry: &WorkspaceHistoryEntry) -> Value {
    json!({
        "transaction_id": entry.transaction_id().to_string(),
        "actor": actor_label(entry.actor()),
        "intent": entry.intent(),
        "timestamp_millis": entry.timestamp_millis(),
        "revision_before": entry.revision_before().get(),
        "revision_after": entry.revision_after().get(),
        "affected_targets": affected_targets_json(entry.affected_targets()),
        "operations": entry.operations().iter().map(|operation| {
            json!({
                "id": operation.id().to_string(),
                "kind": operation.kind(),
                "summary": operation.summary(),
                "affected_targets": affected_targets_json(operation.affected_targets()),
            })
        }).collect::<Vec<_>>(),
    })
}

fn snapshot_json(snapshot: &WorkspaceSnapshot) -> Value {
    json!({
        "id": snapshot.id().to_string(),
        "name": snapshot.name(),
        "actor": actor_label(snapshot.actor()),
        "created_from_revision": snapshot.created_from_revision().get(),
        "created_at_millis": snapshot.created_at_millis(),
    })
}

fn diff_json(diff: &WorkspaceSceneDiff) -> Value {
    json!({
        "before_revision": diff.before_revision().get(),
        "after_revision": diff.after_revision().get(),
        "summary": diff.summary(),
        "affected_targets": affected_targets_json(diff.affected_targets()),
        "changes": diff.changes().iter().map(scene_change_json).collect::<Vec<_>>(),
    })
}

fn affected_targets_json(targets: &geom_workspace::AffectedTargets) -> Value {
    json!({
        "node_ids": targets.node_ids().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "parameter_ids": targets.parameter_ids().iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

fn scene_change_json(change: &WorkspaceSceneChange) -> Value {
    match change {
        WorkspaceSceneChange::RootChanged { before, after } => json!({
            "kind": "root_changed",
            "before": before.to_string(),
            "after": after.to_string(),
        }),
        WorkspaceSceneChange::ParameterAdded { id, value } => json!({
            "kind": "parameter_added",
            "id": id.to_string(),
            "value": value,
        }),
        WorkspaceSceneChange::ParameterRemoved { id, value } => json!({
            "kind": "parameter_removed",
            "id": id.to_string(),
            "value": value,
        }),
        WorkspaceSceneChange::ParameterChanged { id, before, after } => json!({
            "kind": "parameter_changed",
            "id": id.to_string(),
            "before": before,
            "after": after,
        }),
        WorkspaceSceneChange::NodeAdded { id, kind } => json!({
            "kind": "node_added",
            "id": id.to_string(),
            "node_kind": kind,
        }),
        WorkspaceSceneChange::NodeRemoved { id, kind } => json!({
            "kind": "node_removed",
            "id": id.to_string(),
            "node_kind": kind,
        }),
        WorkspaceSceneChange::NodeChanged {
            id,
            before_kind,
            after_kind,
            fields,
        } => json!({
            "kind": "node_changed",
            "id": id.to_string(),
            "before_kind": before_kind,
            "after_kind": after_kind,
            "fields": fields.iter().map(node_change_field_label).collect::<Vec<_>>(),
        }),
    }
}

fn bounds_json(bounds: &Bounds) -> Value {
    match bounds {
        Bounds::Empty => json!({
            "kind": "empty",
            "min": Value::Null,
            "max": Value::Null,
            "center": Value::Null,
            "size": Value::Null,
        }),
        Bounds::Finite { min, max } => json!({
            "kind": "finite",
            "min": min,
            "max": max,
            "center": bounds.center(),
            "size": bounds.size(),
        }),
    }
}

fn render_bounds_text(bounds: &Bounds) -> String {
    match bounds {
        Bounds::Empty => "empty".to_owned(),
        Bounds::Finite { min, max } => format!(
            "min={min:?}, max={max:?}, center={:?}, size={:?}",
            bounds.center(),
            bounds.size()
        ),
    }
}

fn actor_label(actor: TransactionActor) -> &'static str {
    match actor {
        TransactionActor::User => "user",
        TransactionActor::Ai => "ai",
        TransactionActor::CliAutomation => "cli-automation",
        TransactionActor::SystemMigration => "system-migration",
    }
}

fn render_diff_changes(diff: &WorkspaceSceneDiff) -> String {
    diff.changes()
        .iter()
        .map(|change| match change {
            WorkspaceSceneChange::RootChanged { before, after } => {
                format!("root {} -> {}", before, after)
            }
            WorkspaceSceneChange::ParameterAdded { id, value } => {
                format!("parameter {} added={}", id, value)
            }
            WorkspaceSceneChange::ParameterRemoved { id, value } => {
                format!("parameter {} removed={}", id, value)
            }
            WorkspaceSceneChange::ParameterChanged { id, before, after } => {
                format!("parameter {} {} -> {}", id, before, after)
            }
            WorkspaceSceneChange::NodeAdded { id, kind } => {
                format!("node {} added ({})", id, kind)
            }
            WorkspaceSceneChange::NodeRemoved { id, kind } => {
                format!("node {} removed ({})", id, kind)
            }
            WorkspaceSceneChange::NodeChanged {
                id,
                before_kind,
                after_kind,
                fields,
            } => format!(
                "node {} {} -> {} [{}]",
                id,
                before_kind,
                after_kind,
                fields
                    .iter()
                    .map(node_change_field_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn node_change_field_label(field: &geom_workspace::NodeChangeField) -> &'static str {
    match field {
        geom_workspace::NodeChangeField::Label => "label",
        geom_workspace::NodeChangeField::Kind => "kind",
        geom_workspace::NodeChangeField::Transform => "transform",
        geom_workspace::NodeChangeField::CompositionChildren => "composition_children",
        geom_workspace::NodeChangeField::PrimitiveShape => "primitive_shape",
        geom_workspace::NodeChangeField::Extensions => "extensions",
    }
}

fn join_node_ids<'a>(values: impl Iterator<Item = &'a NodeId>) -> String {
    let mut node_ids: Vec<_> = values.map(ToString::to_string).collect();
    node_ids.sort();
    node_ids.join(", ")
}

fn join_param_ids<'a>(values: impl Iterator<Item = &'a geom_scene::ParamId>) -> String {
    let mut parameter_ids: Vec<_> = values.map(ToString::to_string).collect();
    parameter_ids.sort();
    if parameter_ids.is_empty() {
        "(none)".to_owned()
    } else {
        parameter_ids.join(", ")
    }
}

fn portable_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn smoke_workspace_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("workspaces")
            .join("viewport-smoke")
    }

    fn clone_workspace_fixture() -> PathBuf {
        let source_root = smoke_workspace_path();
        let unique_suffix = format!(
            "geom-app-cli-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let target_root = std::env::temp_dir().join(unique_suffix);
        copy_directory_recursive(&source_root, &target_root);
        target_root
    }

    fn copy_directory_recursive(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("create target directory");
        for entry in fs::read_dir(source).expect("read source directory") {
            let entry = entry.expect("directory entry");
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_directory_recursive(&source_path, &target_path);
            } else {
                fs::copy(&source_path, &target_path).expect("copy file");
            }
        }
    }

    fn write_transaction_file(workspace_root: &Path, name: &str, contents: &str) -> PathBuf {
        let path = workspace_root.join(name);
        fs::write(&path, contents).expect("write transaction file");
        path
    }

    #[test]
    fn validate_json_succeeds_without_errors() {
        let result = run([
            OsString::from("morphos"),
            OsString::from("validate"),
            smoke_workspace_path().into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        assert!(result.stderr.is_empty());
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["command"], "validate");
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["scene"]["root"], "root");
    }

    #[test]
    fn inspect_text_lists_nodes_and_parameters() {
        let result = run([
            OsString::from("morphos"),
            OsString::from("inspect"),
            smoke_workspace_path().into_os_string(),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        assert!(result.stdout.contains("Node count:"));
        assert!(result.stdout.contains("Parameters:"));
    }

    #[test]
    fn eval_json_supports_selected_output() {
        let result = run([
            OsString::from("morphos"),
            OsString::from("eval"),
            smoke_workspace_path().into_os_string(),
            OsString::from("--output"),
            OsString::from("union_shape"),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["evaluation"]["requested_output"], "union_shape");
        assert!(
            parsed["evaluation"]["mesh"]["triangle_count"]
                .as_u64()
                .expect("triangle count")
                > 0
        );
    }

    #[test]
    fn missing_workspace_returns_usage_exit_code() {
        let result = run([OsString::from("morphos"), OsString::from("validate")]);
        assert_eq!(result.exit_code, CliExitCode::Usage);
        assert!(result.stderr.contains("Usage:"));
    }

    #[test]
    fn invalid_scene_returns_source_exit_code() {
        let workspace_root = clone_workspace_fixture();
        let source_path = workspace_root.join("source").join("scene.toml");
        fs::write(&source_path, "schema_version = 1\nroot = \"broken\"\n").expect("write invalid");

        let result = run([
            OsString::from("morphos"),
            OsString::from("validate"),
            workspace_root.into_os_string(),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Source);
        assert!(result.stderr.contains("scene validation failed"));
    }

    #[test]
    fn unknown_output_returns_geometry_exit_code() {
        let result = run([
            OsString::from("morphos"),
            OsString::from("eval"),
            smoke_workspace_path().into_os_string(),
            OsString::from("--output"),
            OsString::from("missing_output"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Geometry);
        assert!(result.stderr.contains("geometry evaluation failed"));
    }

    #[test]
    fn missing_workspace_directory_returns_io_exit_code() {
        let missing = std::env::temp_dir().join(format!(
            "geom-app-cli-missing-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let result = run([
            OsString::from("morphos"),
            OsString::from("inspect"),
            missing.into_os_string(),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Io);
        assert!(result.stderr.contains("failed to open workspace"));
    }

    #[test]
    fn export_obj_writes_default_output_under_workspace_exports() {
        let workspace_root = clone_workspace_fixture();
        let result = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["export"]["format"], "obj");
        assert_eq!(parsed["evaluation"]["requested_output"], "root");

        let export_path = workspace_root.join("exports").join("root.obj");
        let text = fs::read_to_string(export_path).expect("export file");
        assert!(text.starts_with("# Morphos OBJ export\n"));
        assert!(text.contains("\no root\n"));
        assert!(text.contains("\nv "));
        assert!(text.contains("\nf "));
    }

    #[test]
    fn export_supports_selected_output_and_custom_destination() {
        let workspace_root = clone_workspace_fixture();
        let result = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--output"),
            OsString::from("union_shape"),
            OsString::from("--destination"),
            OsString::from("mesh/selected.obj"),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["evaluation"]["requested_output"], "union_shape");
        assert_eq!(parsed["export"]["relative_path"], "mesh/selected.obj");
        let text = fs::read_to_string(
            workspace_root
                .join("exports")
                .join("mesh")
                .join("selected.obj"),
        )
        .expect("selected export");
        assert!(text.contains("# requested_output: union_shape"));
        assert!(text.contains("\no union_shape\n"));
    }

    #[test]
    fn export_stl_writes_ascii_mesh_with_expected_suffix() {
        let workspace_root = clone_workspace_fixture();
        let result = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--format"),
            OsString::from("stl"),
            OsString::from("--destination"),
            OsString::from("mesh/root.stl"),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["export"]["format"], "stl");
        assert_eq!(parsed["export"]["relative_path"], "mesh/root.stl");

        let text = fs::read_to_string(workspace_root.join("exports").join("mesh").join("root.stl"))
            .expect("stl export");
        assert!(text.starts_with("solid root\n"));
        assert!(text.contains("\n  facet normal "));
        assert!(text.contains("\n      vertex "));
        assert!(text.ends_with("endsolid root"));
    }

    #[test]
    fn export_requires_overwrite_flag_when_destination_exists() {
        let workspace_root = clone_workspace_fixture();
        let export_path = workspace_root.join("exports").join("root.obj");
        fs::write(&export_path, "existing export").expect("seed export");

        let result = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.into_os_string(),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Io);
        assert!(result.stderr.contains("--overwrite"));
    }

    #[test]
    fn export_is_deterministic_for_same_workspace_and_destination() {
        let workspace_root = clone_workspace_fixture();
        let first = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--destination"),
            OsString::from("deterministic/root.obj"),
            OsString::from("--json"),
        ]);
        assert_eq!(first.exit_code, CliExitCode::Success);
        let first_text = fs::read_to_string(
            workspace_root
                .join("exports")
                .join("deterministic")
                .join("root.obj"),
        )
        .expect("first export");

        let second = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--destination"),
            OsString::from("deterministic/root.obj"),
            OsString::from("--overwrite"),
            OsString::from("--json"),
        ]);
        assert_eq!(second.exit_code, CliExitCode::Success);
        let second_text = fs::read_to_string(
            workspace_root
                .join("exports")
                .join("deterministic")
                .join("root.obj"),
        )
        .expect("second export");

        assert_eq!(first_text, second_text);
    }

    #[test]
    fn export_stl_is_deterministic_for_same_workspace_and_destination() {
        let workspace_root = clone_workspace_fixture();
        let first = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--format"),
            OsString::from("stl"),
            OsString::from("--destination"),
            OsString::from("deterministic/root.stl"),
            OsString::from("--json"),
        ]);
        assert_eq!(first.exit_code, CliExitCode::Success);
        let first_text = fs::read_to_string(
            workspace_root
                .join("exports")
                .join("deterministic")
                .join("root.stl"),
        )
        .expect("first stl export");

        let second = run([
            OsString::from("morphos"),
            OsString::from("export"),
            workspace_root.clone().into_os_string(),
            OsString::from("--format"),
            OsString::from("stl"),
            OsString::from("--destination"),
            OsString::from("deterministic/root.stl"),
            OsString::from("--overwrite"),
            OsString::from("--json"),
        ]);
        assert_eq!(second.exit_code, CliExitCode::Success);
        let second_text = fs::read_to_string(
            workspace_root
                .join("exports")
                .join("deterministic")
                .join("root.stl"),
        )
        .expect("second stl export");

        assert_eq!(first_text, second_text);
    }

    #[test]
    fn tx_dry_run_reports_diff_without_mutating_workspace() {
        let workspace_root = clone_workspace_fixture();
        let request_path = write_transaction_file(
            &workspace_root,
            "set-param.json",
            r#"{
  "intent": "Dry run parameter change",
  "operations": [
    {
      "kind": "set_parameter_scalar",
      "parameter_id": "arm_length",
      "value": 4.2
    }
  ]
}"#,
        );

        let result = run([
            OsString::from("morphos"),
            OsString::from("tx"),
            OsString::from("dry-run"),
            workspace_root.clone().into_os_string(),
            OsString::from("--file"),
            request_path.into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["transaction"]["mode"], "dry-run");
        assert_eq!(
            parsed["transaction"]["diff"]["summary"],
            "0 root changes, 1 parameter changes, 0 nodes added, 0 nodes removed, 0 nodes changed"
        );
        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("scene");
        assert_eq!(
            reopened.parameters()[&ParamId::new("arm_length").expect("param")].scalar_value(),
            2.6
        );
    }

    #[test]
    fn tx_apply_mutates_workspace_and_history() {
        let workspace_root = clone_workspace_fixture();
        let request_path = write_transaction_file(
            &workspace_root,
            "apply-param.json",
            r#"{
  "actor": "cli-automation",
  "intent": "Apply parameter change",
  "operations": [
    {
      "kind": "set_parameter_scalar",
      "parameter_id": "arm_length",
      "value": 4.8
    }
  ]
}"#,
        );

        let result = run([
            OsString::from("morphos"),
            OsString::from("tx"),
            OsString::from("apply"),
            workspace_root.clone().into_os_string(),
            OsString::from("--file"),
            request_path.into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(result.exit_code, CliExitCode::Success);
        let parsed: Value = serde_json::from_str(&result.stdout).expect("json");
        assert_eq!(parsed["transaction"]["mode"], "apply");
        assert_eq!(parsed["transaction"]["actor"], "cli-automation");

        let reopened = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("scene");
        assert_eq!(
            reopened.parameters()[&ParamId::new("arm_length").expect("param")].scalar_value(),
            4.8
        );

        let history = run([
            OsString::from("morphos"),
            OsString::from("history"),
            workspace_root.into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(history.exit_code, CliExitCode::Success);
        let history_json: Value = serde_json::from_str(&history.stdout).expect("history json");
        assert_eq!(
            history_json["history"].as_array().expect("entries").len(),
            1
        );
        assert_eq!(
            history_json["history"][0]["intent"],
            "Apply parameter change"
        );
    }

    #[test]
    fn snapshot_create_list_and_restore_round_trip() {
        let workspace_root = clone_workspace_fixture();
        let create = run([
            OsString::from("morphos"),
            OsString::from("snapshot"),
            OsString::from("create"),
            workspace_root.clone().into_os_string(),
            OsString::from("--name"),
            OsString::from("Baseline"),
            OsString::from("--json"),
        ]);
        assert_eq!(create.exit_code, CliExitCode::Success);
        let created: Value = serde_json::from_str(&create.stdout).expect("create json");
        let snapshot_id = created["snapshot"]["id"]
            .as_str()
            .expect("snapshot id")
            .to_owned();

        let request_path = write_transaction_file(
            &workspace_root,
            "mutate.json",
            r#"{
  "intent": "Mutate after snapshot",
  "operations": [
    {
      "kind": "set_parameter_scalar",
      "parameter_id": "arm_length",
      "value": 5.1
    }
  ]
}"#,
        );
        let apply = run([
            OsString::from("morphos"),
            OsString::from("tx"),
            OsString::from("apply"),
            workspace_root.clone().into_os_string(),
            OsString::from("--file"),
            request_path.into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(apply.exit_code, CliExitCode::Success);

        let list = run([
            OsString::from("morphos"),
            OsString::from("snapshot"),
            OsString::from("list"),
            workspace_root.clone().into_os_string(),
            OsString::from("--json"),
        ]);
        assert_eq!(list.exit_code, CliExitCode::Success);
        let listed: Value = serde_json::from_str(&list.stdout).expect("list json");
        assert_eq!(listed["snapshots"].as_array().expect("snapshots").len(), 1);
        assert_eq!(listed["snapshots"][0]["name"], "Baseline");

        let restore = run([
            OsString::from("morphos"),
            OsString::from("snapshot"),
            OsString::from("restore"),
            workspace_root.clone().into_os_string(),
            OsString::from("--id"),
            OsString::from(snapshot_id),
            OsString::from("--json"),
        ]);
        assert_eq!(restore.exit_code, CliExitCode::Success);
        let restored = geom_scene::parse_scene(
            &fs::read_to_string(workspace_root.join("source").join("scene.toml")).expect("source"),
        )
        .expect("scene");
        assert_eq!(
            restored.parameters()[&ParamId::new("arm_length").expect("param")].scalar_value(),
            2.6
        );
    }
}
