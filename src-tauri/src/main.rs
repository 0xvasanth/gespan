#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("speech-helper") {
        gespan_speech::helper_main();
    }
    if let Some(code) = gespan_lib::ssh_askpass::maybe_run() {
        std::process::exit(code);
    }
    if std::env::args().nth(1).as_deref() == Some("control") {
        std::process::exit(gespan_lib::control_cli::run(
            std::env::args().skip(2).collect(),
        ));
    }
    if std::env::args().nth(1).as_deref() == Some("app") {
        std::process::exit(gespan_lib::control_cli::run_app(
            std::env::args().skip(2).collect(),
        ));
    }
    gespan_lib::run()
}
