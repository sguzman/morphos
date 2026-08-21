fn main() {
    let workspace_path = geom_app::parse_workspace_path_from_args(std::env::args_os());
    geom_app::run_app(workspace_path);
}
