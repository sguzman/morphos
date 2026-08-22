fn main() {
    let result = geom_app::cli::run(std::env::args_os());
    if !result.stdout.is_empty() {
        println!("{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        eprintln!("{}", result.stderr);
    }
    std::process::exit(result.exit_code.code());
}
