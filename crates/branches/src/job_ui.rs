use crate::fs_backend::format_size;
use crate::jobs::{
    apply_undo_entry, spawn_job, ConflictChoice, ConflictDecision, ConflictPrompt, JobHandle,
    JobProgress, JobRequest, UndoEntry, WorkerEvent,
};
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, CheckButton, Label, MenuButton, Orientation,
    Popover, ProgressBar, Separator, Window,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

struct ActiveJobRow {
    handle: JobHandle,
    rx: mpsc::Receiver<WorkerEvent>,
    clear_cut_clipboard: bool,
    row_box: GtkBox,
    title_label: Label,
    detail_label: Label,
    progress_bar: ProgressBar,
}

pub struct JobUiController {
    parent_window: ApplicationWindow,
    pill_btn: MenuButton,
    pill_label: Label,
    jobs_list_box: GtkBox,
    active_jobs: Rc<RefCell<HashMap<u64, ActiveJobRow>>>,
    undo_stack: Rc<RefCell<Vec<UndoEntry>>>,
    polling_active: Rc<Cell<bool>>,
    on_refresh: Rc<dyn Fn(Option<String>, bool)>,
    on_error: Rc<dyn Fn(String)>,
}

impl JobUiController {
    pub fn new(
        parent_window: ApplicationWindow,
        on_refresh: Rc<dyn Fn(Option<String>, bool)>,
        on_error: Rc<dyn Fn(String)>,
    ) -> Rc<Self> {
        let pill_content = GtkBox::new(Orientation::Horizontal, 6);
        let spinner_icon = gtk4::Image::from_icon_name("emblem-synchronizing-symbolic");
        spinner_icon.set_pixel_size(14);
        let pill_label = Label::builder()
            .label("0%")
            .css_classes(["branches-jobs-pill-label"])
            .build();
        pill_content.append(&spinner_icon);
        pill_content.append(&pill_label);

        let jobs_list_box = GtkBox::new(Orientation::Vertical, 8);
        jobs_list_box.set_margin_top(10);
        jobs_list_box.set_margin_bottom(10);
        jobs_list_box.set_margin_start(12);
        jobs_list_box.set_margin_end(12);
        jobs_list_box.set_size_request(310, -1);

        let popover = Popover::builder()
            .has_arrow(true)
            .css_classes(["malus-popover-menu", "branches-jobs-popover"])
            .child(&jobs_list_box)
            .build();

        let pill_btn = MenuButton::builder()
            .child(&pill_content)
            .popover(&popover)
            .valign(Align::Center)
            .visible(false)
            .tooltip_text(if malus_de_common::is_spanish_locale() {
                "Operaciones en curso"
            } else {
                "Active file operations"
            })
            .css_classes(["branches-tool-btn", "branches-jobs-pill-btn"])
            .build();

        Rc::new(Self {
            parent_window,
            pill_btn,
            pill_label,
            jobs_list_box,
            active_jobs: Rc::new(RefCell::new(HashMap::new())),
            undo_stack: Rc::new(RefCell::new(Vec::new())),
            polling_active: Rc::new(Cell::new(false)),
            on_refresh,
            on_error,
        })
    }

    pub fn pill_widget(&self) -> &MenuButton {
        &self.pill_btn
    }

    pub fn push_undo(&self, entry: UndoEntry) {
        let mut stack = self.undo_stack.borrow_mut();
        stack.push(entry);
        if stack.len() > 40 {
            stack.remove(0);
        }
    }

    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.borrow().is_empty()
    }

    #[allow(dead_code)]
    pub fn top_undo_label(&self, es: bool) -> Option<&'static str> {
        self.undo_stack
            .borrow()
            .last()
            .map(|e| e.description(es))
    }

    pub fn undo_last(&self) {
        let Some(entry) = self.undo_stack.borrow_mut().pop() else {
            return;
        };
        match apply_undo_entry(entry) {
            Ok(affected) => {
                let select_name = affected
                    .first()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string());
                (self.on_refresh)(select_name, false);
            }
            Err(err) => {
                (self.on_error)(err);
            }
        }
    }

    pub fn start_job(self: &Rc<Self>, request: JobRequest, clear_cut_clipboard: bool) {
        let es = malus_de_common::is_spanish_locale();
        let initial_title = request.title(es);
        let (handle, rx) = spawn_job(request, clear_cut_clipboard, es);
        let job_id = handle.id;

        let row_box = GtkBox::new(Orientation::Vertical, 4);
        row_box.add_css_class("branches-job-row");

        let top_row = GtkBox::new(Orientation::Horizontal, 8);
        let title_label = Label::builder()
            .label(&initial_title)
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["branches-job-title"])
            .build();

        let cancel_flag = handle.cancel_flag.clone();
        let cancel_btn = Button::builder()
            .icon_name("window-close-symbolic")
            .has_frame(false)
            .valign(Align::Center)
            .tooltip_text(if es { "Cancelar" } else { "Cancel" })
            .css_classes(["flat", "circular", "branches-job-cancel-btn"])
            .build();
        cancel_btn.connect_clicked(move |_| {
            cancel_flag.store(true, Ordering::Relaxed);
        });
        top_row.append(&title_label);
        top_row.append(&cancel_btn);

        let progress_bar = ProgressBar::builder()
            .hexpand(true)
            .css_classes(["branches-job-progress"])
            .build();
        progress_bar.pulse();

        let detail_label = Label::builder()
            .label(if es { "Preparando…" } else { "Preparing…" })
            .halign(Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .css_classes(["dim-label", "branches-job-detail"])
            .build();

        row_box.append(&top_row);
        row_box.append(&progress_bar);
        row_box.append(&detail_label);

        self.jobs_list_box.append(&row_box);
        self.pill_btn.set_visible(true);

        self.active_jobs.borrow_mut().insert(
            job_id,
            ActiveJobRow {
                handle,
                rx,
                clear_cut_clipboard,
                row_box,
                title_label,
                detail_label,
                progress_bar,
            },
        );

        self.ensure_polling();
    }

    fn ensure_polling(self: &Rc<Self>) {
        if self.polling_active.replace(true) {
            return;
        }
        let this = Rc::clone(self);
        glib::timeout_add_local(std::time::Duration::from_millis(45), move || {
            if !this.drain_once() {
                this.polling_active.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn drain_once(self: &Rc<Self>) -> bool {
        let mut completed_ids = Vec::new();
        let mut conflicts = Vec::new();
        let mut updates: Vec<JobProgress> = Vec::new();
        let mut completions = Vec::new();

        {
            let jobs = self.active_jobs.borrow();
            for (&job_id, job) in jobs.iter() {
                while let Ok(ev) = job.rx.try_recv() {
                    match ev {
                        WorkerEvent::Progress(prog) => updates.push(prog),
                        WorkerEvent::Conflict {
                            job_id: jid,
                            prompt,
                            reply,
                        } => conflicts.push((jid, prompt, reply)),
                        WorkerEvent::Finished(comp) => {
                            completed_ids.push(job_id);
                            completions.push(comp);
                            break;
                        }
                    }
                }
            }
        }

        let mut overall_fraction = 0.0f64;
        let mut active_frac_count = 0usize;

        {
            let jobs = self.active_jobs.borrow();
            for prog in updates {
                if let Some(job) = jobs.get(&prog.id) {
                    job.title_label.set_label(&prog.title);
                    let frac = if prog.total_bytes > 0 {
                        (prog.done_bytes as f64 / prog.total_bytes as f64).clamp(0.0, 1.0)
                    } else if prog.total_items > 0 {
                        (prog.done_items as f64 / prog.total_items as f64).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    if prog.total_bytes > 0 || prog.total_items > 1 {
                        job.progress_bar.set_fraction(frac);
                        overall_fraction += frac;
                        active_frac_count += 1;
                    } else {
                        job.progress_bar.pulse();
                    }

                    let detail = if prog.total_bytes > 0 {
                        format!(
                            "{} — {} / {}",
                            prog.current_item,
                            format_size(prog.done_bytes),
                            format_size(prog.total_bytes)
                        )
                    } else if !prog.current_item.is_empty() {
                        format!(
                            "{} ({}/{})",
                            prog.current_item, prog.done_items, prog.total_items
                        )
                    } else {
                        format!("{}/{}", prog.done_items, prog.total_items)
                    };
                    job.detail_label.set_label(&detail);
                }
            }
        }

        if active_frac_count > 0 {
            let pct = ((overall_fraction / active_frac_count as f64) * 100.0).round() as u32;
            self.pill_label.set_label(&format!("{}%", pct));
        }

        for (job_id, prompt, reply) in conflicts {
            let _ = job_id;
            show_conflict_dialog(&self.parent_window, prompt, reply);
        }

        for comp in completions {
            let removed = self.active_jobs.borrow_mut().remove(&comp.id);
            if let Some(job) = removed {
                let _ = &job.handle;
                let _ = job.clear_cut_clipboard;
                self.jobs_list_box.remove(&job.row_box);
            }
            if let Some(undo_entry) = comp.undo_entry {
                self.push_undo(undo_entry);
            }
            if comp.cancelled {
                tracing::info!("Operación {} cancelada por el usuario", comp.id);
            }
            if comp.changed || comp.clear_cut_clipboard {
                (self.on_refresh)(comp.select_name, comp.clear_cut_clipboard);
            }
            if let Some(err) = comp.error {
                (self.on_error)(err);
            }
        }

        let remaining = !self.active_jobs.borrow().is_empty();
        if !remaining {
            self.pill_btn.set_visible(false);
            self.pill_label.set_label("0%");
            if let Some(pop) = self.pill_btn.popover() {
                pop.popdown();
            }
        }
        remaining
    }
}

fn show_conflict_dialog(
    parent: &ApplicationWindow,
    prompt: ConflictPrompt,
    reply: mpsc::SyncSender<ConflictDecision>,
) {
    let es = malus_de_common::is_spanish_locale();
    let name = prompt
        .target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "archivo".to_string());

    let dialog = Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .resizable(false)
        .title(if es {
            "Conflicto de archivo"
        } else {
            "File Conflict"
        })
        .default_width(400)
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(18);
    vbox.set_margin_end(18);

    let msg = if es {
        if prompt.is_dir {
            format!("Ya existe una carpeta llamada «{}» en el destino. ¿Qué deseas hacer?", name)
        } else {
            format!("Ya existe un archivo llamado «{}» en el destino. ¿Qué deseas hacer?", name)
        }
    } else if prompt.is_dir {
        format!("A folder named \"{}\" already exists at the destination. What would you like to do?", name)
    } else {
        format!("A file named \"{}\" already exists at the destination. What would you like to do?", name)
    };

    let lbl = Label::builder()
        .label(&msg)
        .wrap(true)
        .xalign(0.0)
        .build();
    vbox.append(&lbl);

    let src_info = format!(
        "{}: {}",
        if es { "Origen" } else { "Source" },
        prompt.source.to_string_lossy()
    );
    let dst_info = format!(
        "{}: {}",
        if es { "Destino" } else { "Destination" },
        prompt.target.to_string_lossy()
    );

    let src_lbl = Label::builder()
        .label(&src_info)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    vbox.append(&src_lbl);

    let sub = Label::builder()
        .label(&dst_info)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    vbox.append(&sub);

    let apply_all = CheckButton::with_label(if es {
        "Aplicar a todo"
    } else {
        "Apply to all"
    });
    vbox.append(&apply_all);
    vbox.append(&Separator::new(Orientation::Horizontal));

    let btn_box = GtkBox::new(Orientation::Horizontal, 8);
    btn_box.set_halign(Align::End);

    let skip_btn = Button::with_label(if es { "Saltar" } else { "Skip" });
    let both_btn = Button::with_label(if es {
        "Mantener ambos"
    } else {
        "Keep Both"
    });
    let replace_btn = Button::builder()
        .label(if es { "Reemplazar" } else { "Replace" })
        .css_classes(["destructive-action"])
        .build();

    btn_box.append(&skip_btn);
    btn_box.append(&both_btn);
    btn_box.append(&replace_btn);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));

    let sent = Rc::new(Cell::new(false));
    let send_choice = {
        let sent = sent.clone();
        let reply = reply.clone();
        let apply_all = apply_all.clone();
        let dialog = dialog.clone();
        move |choice: ConflictChoice| {
            if !sent.replace(true) {
                let _ = reply.send(ConflictDecision {
                    choice,
                    apply_to_all: apply_all.is_active(),
                });
            }
            dialog.close();
        }
    };

    {
        let f = send_choice.clone();
        skip_btn.connect_clicked(move |_| f(ConflictChoice::Skip));
    }
    {
        let f = send_choice.clone();
        both_btn.connect_clicked(move |_| f(ConflictChoice::KeepBoth));
    }
    {
        let f = send_choice;
        replace_btn.connect_clicked(move |_| f(ConflictChoice::Replace));
    }

    let sent_close = sent;
    let reply_close = reply;
    let apply_close = apply_all;
    dialog.connect_close_request(move |_| {
        if !sent_close.replace(true) {
            let _ = reply_close.send(ConflictDecision {
                choice: ConflictChoice::Skip,
                apply_to_all: apply_close.is_active(),
            });
        }
        glib::Propagation::Proceed
    });

    dialog.present();
}
