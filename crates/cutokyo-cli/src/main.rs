//! Native Cutokyo CLI.

use std::{path::Path, process::ExitCode};

use cutokyo_core::{app::Application, plugin::PluginVerifier};

const USAGE: &str =
    "usage: cutokyo [version|contract|bundle --json|plugin verify <directory>|mcp manifest --json]";

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command] if matches!(command.as_str(), "version" | "--version" | "-V") => {
            println!("cutokyo {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [command] if command == "contract" => print_contract(),
        [bundle, json] if bundle == "bundle" && json == "--json" => print_bundle(),
        [plugin, verify, directory] if plugin == "plugin" && verify == "verify" => {
            verify_plugin(Path::new(directory))
        }
        [mcp, manifest, json] if mcp == "mcp" && manifest == "manifest" && json == "--json" => {
            print_mcp_manifest()
        }
        [] => {
            println!("Cutokyo local session observability (pre-1.0)");
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
    }
}

fn print_contract() -> ExitCode {
    let app = Application::new();
    match serde_json::to_string_pretty(&app.contract_snapshot()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to serialize contract snapshot: {error}");
            ExitCode::from(70)
        }
    }
}

fn print_bundle() -> ExitCode {
    let bundle = match cutokyo_core::bundle::build_diagnostic_bundle(&[]) {
        Ok(bundle) => bundle,
        Err(error) => {
            eprintln!("failed to construct sanitized diagnostic bundle: {error}");
            return ExitCode::from(70);
        }
    };
    if let Ok(json) = serde_json::to_string_pretty(&bundle) {
        println!("{json}");
        ExitCode::SUCCESS
    } else {
        eprintln!("failed to serialize sanitized diagnostic bundle");
        ExitCode::from(70)
    }
}

fn print_mcp_manifest() -> ExitCode {
    if let Ok(json) = serde_json::to_string_pretty(&cutokyo_core::mcp::surface_manifest()) {
        println!("{json}");
        ExitCode::SUCCESS
    } else {
        eprintln!("failed to serialize MCP surface manifest");
        ExitCode::from(70)
    }
}

fn verify_plugin(directory: &Path) -> ExitCode {
    let verifier = match PluginVerifier::new() {
        Ok(verifier) => verifier,
        Err(error) => return print_plugin_error(&error),
    };
    match verifier.verify_path(directory) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to serialize plugin verification report: {error}");
                ExitCode::from(70)
            }
        },
        Err(error) => print_plugin_error(&error),
    }
}

fn print_plugin_error(error: &cutokyo_core::plugin::PluginVerificationError) -> ExitCode {
    match serde_json::to_string_pretty(error) {
        Ok(json) => eprintln!("{json}"),
        Err(_) => eprintln!("plugin verification failed; structured diagnostic unavailable"),
    }
    ExitCode::from(65)
}

#[cfg(test)]
mod tests {
    use cutokyo_core::{
        bundle::{BundleDiagnostic, build_diagnostic_bundle},
        guards::REDACTION_MARKER,
    };

    #[test]
    fn bundle_has_no_content_or_secret() -> Result<(), Box<dyn std::error::Error>> {
        let secret = ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat();
        let bundle = build_diagnostic_bundle(&[BundleDiagnostic {
            component: "guard".to_owned(),
            category: secret.clone(),
            count: 1,
        }])?;
        let serialized = serde_json::to_string(&bundle)?;
        assert!(!serialized.contains(&secret));
        assert!(serialized.contains(REDACTION_MARKER));
        assert!(!serialized.contains("/home/example/private-project"));
        assert!(!serialized.contains("raw_header_value"));
        assert!(!serialized.contains("raw_url_value"));
        assert_eq!(
            bundle.coverage,
            cutokyo_core::guards::GuardCoverageState::Inspected
        );
        assert!(bundle.excludes.contains(&"prompts".to_owned()));
        assert!(bundle.excludes.contains(&"transcripts".to_owned()));
        assert!(bundle.excludes.contains(&"full_paths".to_owned()));
        Ok(())
    }
}
