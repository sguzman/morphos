use geom_geometry::{BoolmeshBackend, Bounds, EvaluatedGeometry, GeometryEvaluator};
use geom_scene::{NodeId, SceneDocument, parse_scene};
use geom_workspace::Workspace;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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

    let mut output_format = OutputFormat::Text;
    let mut workspace: Option<PathBuf> = None;
    let mut requested_output: Option<NodeId> = None;
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
        _ => Err(format!(
            "unknown command `{command_name}`\n\n{}",
            usage(&program)
        )),
    }
}

fn usage(program: &str) -> String {
    format!(
        "Usage:\n  {program} validate <workspace> [--json]\n  {program} inspect <workspace> [--json]\n  {program} eval <workspace> [--output <node-id>] [--json]"
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
    }
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
}
