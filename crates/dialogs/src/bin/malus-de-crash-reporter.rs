use std::path::PathBuf;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::Application;
use malus_de_common::crash::{archive_pending_report, list_pending_reports};
use malus_de_common::logging::init_logging;

fn resolve_target_report() -> Option<(PathBuf, String, usize)> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(idx) = args.iter().position(|a| a == "--report") {
        if let Some(path_str) = args.get(idx + 1) {
            let path = PathBuf::from(path_str);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let archived = archive_pending_report(&path);
                return Some((archived, content, 0));
            }
        }
        return None;
    }

    let pending = list_pending_reports();
    let newest = pending.first()?.clone();
    let content = std::fs::read_to_string(&newest).ok()?;
    let archived = archive_pending_report(&newest);
    let mut extra = 0usize;
    for path in pending.iter().skip(1) {
        let _ = archive_pending_report(path);
        extra += 1;
    }
    Some((archived, content, extra))
}

fn main() {
    init_logging("malus_de_crash_reporter");

    let Some((report_path, report_text, extra_count)) = resolve_target_report() else {
        return;
    };

    let app = Application::builder()
        .application_id("dev.malus.CrashReporter")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let path_cell = std::cell::RefCell::new(Some((report_path, report_text)));
    app.connect_activate(move |app| {
        if let Some((path, text)) = path_cell.borrow_mut().take() {
            malus_de_dialogs::show_crash_report_dialog(app, path, text, extra_count);
        }
    });

    app.run_with_args::<&str>(&[]);
}

