//! `ostraka-app` — the desktop window onto the same records.
//!
//! A second view, not a second implementation. Everything it shows comes from
//! `ostraka-runtime`, and promoting goes through the gate exactly as the command
//! line does, so nothing here can approve a run the gate refuses.
//!
//! Rust throughout, with no web runtime and no second toolchain: `egui` draws
//! into a window `eframe` opens. That is the condition under which this project
//! takes a dependency at all — a binary someone can download must not turn into
//! a binary that then needs an installer for something else.

mod state;
mod ui;

use state::State;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Same convention as the command line: the project is where you are, unless
    // you say otherwise.
    let project = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut state = match State::load(&project) {
        Ok(state) => state,
        Err(e) => {
            // Before a window exists, the terminal is the only place to say so.
            eprintln!("ostraka-app: {}: {e}", project.display());
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([720.0, 420.0])
            .with_title("Ostraka"),
        ..Default::default()
    };

    eframe::run_ui_native("Ostraka", options, move |ui, _frame| {
        ui::draw(ui, &mut state);
    })
}
