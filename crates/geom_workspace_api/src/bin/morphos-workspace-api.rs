use geom_workspace_api::protocol::{ApiProtocolRequest, ApiProtocolResponse, ProtocolServer};
use std::io::{self, BufRead, Write};

fn main() {
    if let Err(error) = run() {
        let _ = writeln!(io::stderr(), "{error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut server = ProtocolServer::default();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<ApiProtocolRequest>(&line) {
            Ok(request) => server.dispatch(request),
            Err(error) => ApiProtocolResponse::error(
                geom_workspace_api::API_PROTOCOL_VERSION,
                None,
                "malformed_request",
                error.to_string(),
                None,
            ),
        };

        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }

    Ok(())
}
