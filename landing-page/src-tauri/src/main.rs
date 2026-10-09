#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !args.is_empty() || std::env::var_os("CUTOKYO_PROXY_ONLY").is_some() {
        if let Err(error) = cutokyo_desktop::run_headless(args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    cutokyo_app_lib::run()
}
