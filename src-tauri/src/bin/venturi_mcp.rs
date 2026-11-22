use serde_json::json;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use venturi_cfd_lib::mcp::registry::handle_jsonrpc_request;
use venturi_cfd_lib::session::default_session_path;

fn parse_session_arg() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--session" {
            if let Some(val) = args.next() {
                if !val.trim().is_empty() {
                    return PathBuf::from(val);
                }
            }
        } else if let Some(val) = arg.strip_prefix("--session=") {
            if !val.trim().is_empty() {
                return PathBuf::from(val);
            }
        } else if arg == "--help" || arg == "-h" {
            eprintln!("venturi-mcp — Venturi CFD Engineering Studio MCP Server (JSON-RPC 2.0 over stdio)");
            eprintln!("Usage: venturi-mcp [--session <path/to/active_session.json>]");
            std::process::exit(0);
        }
    }
    default_session_path()
}

#[tokio::main]
async fn main() {
    let session_path = parse_session_arg();
    eprintln!(
        "[venturi-mcp] Starting MCP stdio server (session: {})",
        session_path.display()
    );

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Ok(Some(raw_line)) = lines.next_line().await {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(req) => handle_jsonrpc_request(&session_path, req).await,
            Err(err) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32700,
                    "message": format!("Parse error: {}", err)
                }
            })),
        };

        if let Some(resp) = response {
            match serde_json::to_string(&resp) {
                Ok(mut line) => {
                    line.push('\n');
                    if let Err(e) = stdout.write_all(line.as_bytes()).await {
                        eprintln!("[venturi-mcp] Failed writing response to stdout: {}", e);
                        break;
                    }
                    if let Err(e) = stdout.flush().await {
                        eprintln!("[venturi-mcp] Failed flushing stdout: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[venturi-mcp] Failed serializing response: {}", e);
                }
            }
        }
    }

    eprintln!("[venturi-mcp] stdin closed, exiting cleanly.");
}
