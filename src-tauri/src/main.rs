// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Shell-integration invocations are handled without starting the GUI.
    if let Some(code) = chute_lib::run_cli() {
        std::process::exit(code);
    }
    chute_lib::run()
}
