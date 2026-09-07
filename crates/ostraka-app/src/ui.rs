//! Drawing the state.
//!
//! Reads [`State`] and renders it. It decides nothing: promotion goes through
//! the same gate as the command line, and a button cannot approve what the gate
//! refuses.

use crate::state::{Detail, State, outcome_word};
use egui::{CentralPanel, Color32, Panel, RichText, ScrollArea, Ui};
use ostraka_core::record::Event;
use ostraka_runtime::index::BackendUsage;

/// Green, red, amber and grey, chosen to survive both egui themes rather than
/// to match one of them.
const APPROVED: Color32 = Color32::from_rgb(0x3f, 0xa2, 0x5f);
const REFUSED: Color32 = Color32::from_rgb(0xc0, 0x4a, 0x4a);
const UNFINISHED: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x8a);
const CONTEXT: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);

pub fn draw(ui: &mut Ui, state: &mut State) {
    keys(ui.ctx(), state);
    state.ensure_loaded();

    Panel::top("head").show(ui, |ui| head(ui, state));
    Panel::bottom("foot").show(ui, |ui| foot(ui, state));
    // Wide enough that a task reads on one line at the default window size.
    // The list is for scanning; the detail side is for reading.
    Panel::left("runs")
        .resizable(true)
        .default_size(360.0)
        .min_size(220.0)
        .show(ui, |ui| runs(ui, state));
    CentralPanel::default().show(ui, |ui| detail(ui, state));
}

/// The keys the terminal browser uses, so muscle memory carries between them.
fn keys(ctx: &egui::Context, state: &mut State) {
    // With the filter box focused, `j` is a letter rather than a movement.
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    ctx.input(|i| {
        use egui::Key;
        if i.key_pressed(Key::J) || i.key_pressed(Key::ArrowDown) {
            state.move_by(1);
        }
        if i.key_pressed(Key::K) || i.key_pressed(Key::ArrowUp) {
            state.move_by(-1);
        }
        if i.key_pressed(Key::R) {
            state.reload();
        }
    });
}

fn head(ui: &mut Ui, state: &State) {
    ui.horizontal(|ui| {
        ui.heading("Ostraka");
        ui.label(RichText::new(state.project.display().to_string()).color(CONTEXT));
        let (shown, total) = (state.matching.len(), state.runs.len());
        let count = if shown == total {
            format!("{total} run{}", if total == 1 { "" } else { "s" })
        } else {
            format!("{shown} of {total} runs")
        };
        ui.label(RichText::new(count).color(CONTEXT));
    });
}

fn foot(ui: &mut Ui, state: &State) {
    let backends = state.backends();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("tokens").color(CONTEXT));
        if backends.is_empty() {
            ui.label(
                RichText::new("no backend on these runs reported what it spent").color(CONTEXT),
            );
        }
        for (i, backend) in backends.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("·").color(CONTEXT));
            }
            ui.label(RichText::new(&backend.adapter).strong());
            ui.label(describe_backend(backend));
        }
    });
    if let Some(status) = &state.status {
        ui.label(RichText::new(status).color(Color32::from_rgb(0xb8, 0x8a, 0x2a)));
    }
}

/// A backend's totals, said the way its vendor said them.
///
/// A vendor reporting one combined figure is shown as a total, not as a split
/// with a zero it never claimed; a vendor that rounds is marked with a tilde.
pub fn describe_backend(backend: &BackendUsage) -> String {
    let about = if backend.approximate { "~" } else { "" };
    if backend.input == 0 && backend.output == 0 {
        return format!("{about}{} total", compact(backend.total));
    }
    let split = format!(
        "{about}{} in / {about}{} out",
        compact(backend.input),
        compact(backend.output)
    );
    if backend.total == 0 {
        split
    } else {
        format!("{split} · {about}{} total", compact(backend.total))
    }
}

/// A token count at a glance. Exact below a thousand, magnitude above it.
pub fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn runs(ui: &mut Ui, state: &mut State) {
    ui.add_space(4.0);
    let filter = ui.add(
        egui::TextEdit::singleline(&mut state.filter)
            .hint_text("filter by task, id or outcome")
            .desired_width(f32::INFINITY),
    );
    if filter.changed() {
        state.refilter();
    }
    ui.add_space(4.0);
    ui.separator();

    if state.matching.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(if state.runs.is_empty() {
            "No runs yet. `ostraka run` makes one."
        } else {
            "Nothing matches this filter."
        }));
        return;
    }

    let mut clicked = None;
    ScrollArea::vertical().show(ui, |ui| {
        for position in 0..state.matching.len() {
            let Some(run) = state.runs.get(state.matching[position]) else {
                continue;
            };
            let colour = match outcome_word(run) {
                "approved" => APPROVED,
                "refused" | "failed" => REFUSED,
                _ => UNFINISHED,
            };
            let label = format!(
                "{}  {}",
                outcome_word(run),
                first_line(&described(&run.prompt))
            );
            // Truncated rather than wrapped: a list whose every entry is four
            // lines tall cannot be scanned, and the whole task is on the detail
            // side anyway. The hover carries the rest.
            let response = ui
                .selectable_label(
                    state.selected == Some(position),
                    RichText::new(label).color(colour),
                )
                .on_hover_text(described(&run.prompt));
            if response.clicked() {
                clicked = Some(position);
            }
        }
    });
    if let Some(position) = clicked {
        state.select(position);
    }
}

fn detail(ui: &mut Ui, state: &mut State) {
    let Some(run) = state.selected_run().cloned() else {
        ui.label("Select a run.");
        return;
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(&run.run_id).strong());
        let colour = match outcome_word(&run) {
            "approved" => APPROVED,
            "refused" | "failed" => REFUSED,
            _ => UNFINISHED,
        };
        ui.label(RichText::new(outcome_word(&run)).color(colour));
        // The gate decides; this only stops someone asking a question whose
        // answer is already known.
        ui.add_enabled_ui(state.can_promote(), |ui| {
            if ui
                .button("Promote")
                .on_hover_text("Gives the run's commit a branch. Merges nothing.")
                .clicked()
            {
                state.promote_selected();
            }
        });
    });

    ui.add_space(4.0);
    // Selectable so a run id or an error can be copied out, which is most of
    // what a window offers over the terminal view.
    ui.label(RichText::new(described(&run.prompt)));
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} ({})", run.author, run.adapter)).color(CONTEXT));
        if let Some(reviewer) = &run.reviewer {
            ui.label(RichText::new(format!("reviewed by {reviewer}")).color(CONTEXT));
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for tab in Detail::ALL {
            if ui
                .selectable_label(state.detail == tab, tab.title())
                .clicked()
            {
                state.show(tab);
            }
        }
    });
    ui.separator();

    ScrollArea::vertical().show(ui, |ui| match state.detail {
        Detail::Checks => checks(ui, state),
        Detail::Events => events(ui, state),
        Detail::Diff => diff(ui, state),
    });
}

fn checks(ui: &mut Ui, state: &State) {
    let Some(record) = &state.record else {
        ui.label(RichText::new("reading the record…").color(CONTEXT));
        return;
    };
    if record.checks.is_empty() {
        ui.label(RichText::new("no checks ran").color(CONTEXT));
        return;
    }
    for check in &record.checks {
        let (word, colour) = if check.passed() {
            ("pass", APPROVED)
        } else {
            ("FAIL", REFUSED)
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(word).color(colour).monospace());
            ui.label(RichText::new(&check.name).monospace());
            ui.label(RichText::new(format!("{}ms", check.duration_ms)).color(CONTEXT));
        });
        // Only for a failure: a passing check's output is noise, and a failing
        // one is why someone opened this.
        if !check.passed() {
            for stream in [&check.stderr, &check.stdout] {
                if !stream.trim().is_empty() {
                    ui.label(RichText::new(stream.trim()).monospace().color(CONTEXT));
                }
            }
        }
    }
}

fn events(ui: &mut Ui, state: &State) {
    if state.events.is_empty() {
        ui.label(RichText::new("no events recorded").color(CONTEXT));
        return;
    }
    for event in &state.events {
        let (kind, text) = match event {
            Event::Message { text, .. } => ("said", text.clone()),
            Event::ToolUse { name, .. } => ("tool", name.clone()),
            Event::Error { message, .. } => ("error", message.clone()),
            Event::Finished {
                exit_code,
                files_touched,
            } => (
                "done",
                format!(
                    "exit {}, {} file(s)",
                    exit_code.map(|c| c.to_string()).unwrap_or("?".into()),
                    files_touched.len()
                ),
            ),
        };
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(kind).color(CONTEXT).monospace());
            ui.label(RichText::new(text).monospace());
        });
    }
}

fn diff(ui: &mut Ui, state: &State) {
    match &state.diff {
        None => {
            ui.label(RichText::new("reading the change…").color(CONTEXT));
        }
        // Two causes, one appearance: a refused run never committed, and an old
        // approved one may have been merged away since.
        Some(None) => {
            ui.label(RichText::new("This run produced no commit.").color(CONTEXT));
            ui.label(
                RichText::new("It was refused, or its branch has since been merged away.")
                    .color(CONTEXT),
            );
        }
        Some(Some(text)) => {
            for line in text.lines() {
                let colour = match line.as_bytes().first() {
                    Some(b'+') if !line.starts_with("+++") => APPROVED,
                    Some(b'-') if !line.starts_with("---") => REFUSED,
                    _ => CONTEXT,
                };
                ui.label(RichText::new(line).monospace().color(colour));
            }
        }
    }
}

/// A run recorded before the record kept what was asked.
pub fn described(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        "(recorded before runs kept the task text)".to_string()
    } else {
        prompt.to_string()
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 46 {
        format!("{}…", line.chars().take(45).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(input: u64, output: u64, total: u64, approximate: bool) -> BackendUsage {
        BackendUsage {
            adapter: "v".into(),
            input,
            output,
            total,
            runs: 1,
            approximate,
        }
    }

    #[test]
    fn a_combined_figure_is_shown_as_a_total_not_as_a_split() {
        // "14.7k in / 0 out" would be a zero the vendor never claimed.
        assert_eq!(
            describe_backend(&backend(0, 0, 14_709, false)),
            "14.7k total"
        );
    }

    #[test]
    fn a_rounding_vendor_is_marked_as_an_estimate() {
        assert_eq!(
            describe_backend(&backend(10_600, 296, 0, true)),
            "~10.6k in / ~296 out"
        );
    }

    #[test]
    fn counts_stay_exact_below_a_thousand() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(159_460), "159.5k");
        assert_eq!(compact(2_500_000), "2.5M");
    }

    #[test]
    fn an_old_record_says_why_its_task_is_blank() {
        assert!(described("   ").contains("before runs kept"));
        assert_eq!(described("add a test"), "add a test");
    }
}
