#![windows_subsystem = "windows"]

mod battery_icon;
mod console;
mod controller;
mod debounce;
mod devices;
mod manager;
mod notify;
mod options;
mod reminder;
mod system;
mod tray;
mod wake;

use console::DebugConsole;
use options::Options;
use tray::TrayApp;

fn main() {
    let console = DebugConsole::new("Snake Charger Debug Console");

    std::env::set_var("RUST_LOG", "trace");
    pretty_env_logger::init();

    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(e) => {
            log::error!("{e}");
            std::process::exit(2);
        }
    };
    if options != Options::default() {
        log::info!("Options: {options:?}");
    }

    TrayApp::new(console, options).run();
}
