#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() -> std::process::ExitCode {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    podcast_editor::run_studio(path)
}
