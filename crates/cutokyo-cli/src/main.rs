//! Native Cutokyo CLI foundation.

use std::process::ExitCode;

use cutokyo_core::app::Application;

fn main() -> ExitCode {
    let app = Application::new();
    match std::env::args().nth(1).as_deref() {
        Some("version" | "--version" | "-V") => {
            println!("cutokyo {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("contract") => match serde_json::to_string_pretty(&app.contract_snapshot()) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to serialize contract snapshot: {error}");
                ExitCode::from(70)
            }
        },
        Some(_) => {
            eprintln!("usage: cutokyo [version|contract]");
            ExitCode::from(64)
        }
        None => {
            println!("Cutokyo local session observability (pre-1.0)");
            println!("usage: cutokyo [version|contract]");
            ExitCode::SUCCESS
        }
    }
}
