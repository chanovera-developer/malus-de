use gio::prelude::FileExt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);
const COPY_BUF_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Replace,
    Skip,
    KeepBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictDecision {
    pub choice: ConflictChoice,
    pub apply_to_all: bool,
}

#[derive(Debug, Clone)]
pub struct ConflictPrompt {
    pub source: PathBuf,
    pub target: PathBuf,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::TarGz => "tar.gz",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedUndoRecord {
    pub original_path: PathBuf,
    pub trash_files_path: PathBuf,
    pub trash_info_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoEntry {
    Rename { from: PathBuf, to: PathBuf },
    Moves { pairs: Vec<(PathBuf, PathBuf)> },
    CreatedPaths { paths: Vec<PathBuf> },
    Trashed { records: Vec<TrashedUndoRecord> },
}

impl UndoEntry {
    pub fn description(&self, es: bool) -> &'static str {
        match self {
            UndoEntry::Rename { .. } => {
                if es {
                    "Deshacer renombrado"
                } else {
                    "Undo Rename"
                }
            }
            UndoEntry::Moves { .. } => {
                if es {
                    "Deshacer movimiento"
                } else {
                    "Undo Move"
                }
            }
            UndoEntry::CreatedPaths { .. } => {
                if es {
                    "Deshacer creación/copia"
                } else {
                    "Undo Copy/Create"
                }
            }
            UndoEntry::Trashed { .. } => {
                if es {
                    "Deshacer mover a papelera"
                } else {
                    "Undo Move to Trash"
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobRequest {
    Copy {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    Duplicate {
        sources: Vec<PathBuf>,
    },
    Trash {
        sources: Vec<PathBuf>,
    },
    Compress {
        sources: Vec<PathBuf>,
        dest_dir: PathBuf,
        format: ArchiveFormat,
    },
    Extract {
        archive: PathBuf,
        dest_dir: PathBuf,
    },
}

impl JobRequest {
    pub fn title(&self, es: bool) -> String {
        match self {
            JobRequest::Copy { sources, .. } => {
                if es {
                    format!("Copiando {} elemento(s)", sources.len())
                } else {
                    format!("Copying {} item(s)", sources.len())
                }
            }
            JobRequest::Move { sources, .. } => {
                if es {
                    format!("Moviendo {} elemento(s)", sources.len())
                } else {
                    format!("Moving {} item(s)", sources.len())
                }
            }
            JobRequest::Duplicate { sources } => {
                if es {
                    format!("Duplicando {} elemento(s)", sources.len())
                } else {
                    format!("Duplicating {} item(s)", sources.len())
                }
            }
            JobRequest::Trash { sources } => {
                if es {
                    format!("Moviendo {} a la Papelera", sources.len())
                } else {
                    format!("Moving {} to Trash", sources.len())
                }
            }
            JobRequest::Compress {
                sources, format, ..
            } => {
                if es {
                    format!(
                        "Comprimiendo {} elemento(s) (.{})",
                        sources.len(),
                        format.extension()
                    )
                } else {
                    format!(
                        "Compressing {} item(s) (.{})",
                        sources.len(),
                        format.extension()
                    )
                }
            }
            JobRequest::Extract { archive, .. } => {
                let name = archive
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "archivo".to_string());
                if es {
                    format!("Extrayendo {}", name)
                } else {
                    format!("Extracting {}", name)
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobProgress {
    pub id: u64,
    pub title: String,
    pub current_item: String,
    pub done_items: usize,
    pub total_items: usize,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

pub struct JobCompletion {
    pub id: u64,
    pub changed: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub undo_entry: Option<UndoEntry>,
    pub select_name: Option<String>,
    pub clear_cut_clipboard: bool,
}

pub enum WorkerEvent {
    Progress(JobProgress),
    Conflict {
        job_id: u64,
        prompt: ConflictPrompt,
        reply: mpsc::SyncSender<ConflictDecision>,
    },
    Finished(JobCompletion),
}

pub struct JobHandle {
    pub id: u64,
    pub cancel_flag: Arc<AtomicBool>,
}

pub fn spawn_job(
    request: JobRequest,
    clear_cut_clipboard: bool,
    es: bool,
) -> (JobHandle, mpsc::Receiver<WorkerEvent>) {
    let id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_worker = cancel_flag.clone();
    let (tx, rx) = mpsc::channel::<WorkerEvent>();
    let title = request.title(es);

    std::thread::spawn(move || {
        let mut ctx = JobContext {
            id,
            title,
            cancel_flag: cancel_worker,
            tx: &tx,
            policy_all: None,
            done_items: 0,
            total_items: 0,
            done_bytes: 0,
            total_bytes: 0,
        };

        let result = execute_job(&mut ctx, request);
        let cancelled = ctx.cancel_flag.load(Ordering::Relaxed);
        let completion = match result {
            Ok(outcome) => JobCompletion {
                id,
                changed: outcome.changed,
                cancelled,
                error: None,
                undo_entry: outcome.undo_entry,
                select_name: outcome.select_name,
                clear_cut_clipboard: clear_cut_clipboard && outcome.changed,
            },
            Err(err) => JobCompletion {
                id,
                changed: false,
                cancelled,
                error: if cancelled { None } else { Some(err) },
                undo_entry: None,
                select_name: None,
                clear_cut_clipboard: false,
            },
        };
        let _ = tx.send(WorkerEvent::Finished(completion));
    });

    (JobHandle { id, cancel_flag }, rx)
}

struct JobOutcome {
    changed: bool,
    undo_entry: Option<UndoEntry>,
    select_name: Option<String>,
}

struct JobContext<'a> {
    id: u64,
    title: String,
    cancel_flag: Arc<AtomicBool>,
    tx: &'a mpsc::Sender<WorkerEvent>,
    policy_all: Option<ConflictChoice>,
    done_items: usize,
    total_items: usize,
    done_bytes: u64,
    total_bytes: u64,
}

impl<'a> JobContext<'a> {
    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    fn emit_progress(&self, current_item: &str) {
        let _ = self.tx.send(WorkerEvent::Progress(JobProgress {
            id: self.id,
            title: self.title.clone(),
            current_item: current_item.to_string(),
            done_items: self.done_items,
            total_items: self.total_items.max(1),
            done_bytes: self.done_bytes,
            total_bytes: self.total_bytes,
        }));
    }

    fn ask_conflict(&mut self, source: &Path, target: &Path, is_dir: bool) -> ConflictChoice {
        if let Some(choice) = self.policy_all {
            return choice;
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel::<ConflictDecision>(1);
        if self
            .tx
            .send(WorkerEvent::Conflict {
                job_id: self.id,
                prompt: ConflictPrompt {
                    source: source.to_path_buf(),
                    target: target.to_path_buf(),
                    is_dir,
                },
                reply: reply_tx,
            })
            .is_err()
        {
            return ConflictChoice::Skip;
        }
        match reply_rx.recv() {
            Ok(decision) => {
                if decision.apply_to_all {
                    self.policy_all = Some(decision.choice);
                }
                decision.choice
            }
            Err(_) => ConflictChoice::Skip,
        }
    }
}

fn count_tree(path: &Path) -> (usize, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if meta.is_dir() {
        let mut items = 1usize;
        let mut bytes = 0u64;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let (c, b) = count_tree(&entry.path());
                items += c;
                bytes += b;
            }
        }
        (items, bytes)
    } else {
        (1, meta.len())
    }
}

fn resolve_target_with_conflict(
    ctx: &mut JobContext<'_>,
    src: &Path,
    dest_folder: &Path,
) -> Result<Option<PathBuf>, String> {
    let file_name = src
        .file_name()
        .ok_or_else(|| "Nombre de archivo inválido".to_string())?;
    let target = dest_folder.join(file_name);
    if target.symlink_metadata().is_err() {
        return Ok(Some(target));
    }
    let is_dir = src
        .symlink_metadata()
        .map(|m| m.is_dir())
        .unwrap_or_else(|_| src.is_dir());

    match ctx.ask_conflict(src, &target, is_dir) {
        ConflictChoice::Skip => Ok(None),
        ConflictChoice::KeepBoth => Ok(Some(crate::operations::unique_destination_for_copy(
            src,
            dest_folder,
        )?)),
        ConflictChoice::Replace => {
            if target != src {
                remove_existing_path(&target)?;
            }
            Ok(Some(target))
        }
    }
}

fn remove_existing_path(path: &Path) -> Result<(), String> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|e| format!("No se pudo reemplazar carpeta: {}", e))
    } else {
        fs::remove_file(path).map_err(|e| format!("No se pudo reemplazar archivo: {}", e))
    }
}

fn copy_file_chunked(
    ctx: &mut JobContext<'_>,
    src: &Path,
    dest: &Path,
    label: &str,
) -> Result<(), String> {
    let mut reader =
        fs::File::open(src).map_err(|e| format!("No se pudo abrir '{}': {}", src.display(), e))?;
    let mut writer = fs::File::create(dest)
        .map_err(|e| format!("No se pudo crear '{}': {}", dest.display(), e))?;
    let mut buf = vec![0u8; COPY_BUF_SIZE];
    loop {
        if ctx.is_cancelled() {
            drop(writer);
            let _ = fs::remove_file(dest);
            return Err("Operación cancelada".to_string());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Error leyendo '{}': {}", src.display(), e))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("Error escribiendo '{}': {}", dest.display(), e))?;
        ctx.done_bytes = ctx.done_bytes.saturating_add(n as u64);
        ctx.emit_progress(label);
    }
    if let Ok(meta) = fs::metadata(src) {
        let _ = fs::set_permissions(dest, meta.permissions());
    }
    ctx.done_items += 1;
    ctx.emit_progress(label);
    Ok(())
}

fn copy_tree_chunked(ctx: &mut JobContext<'_>, src: &Path, dest: &Path) -> Result<(), String> {
    if ctx.is_cancelled() {
        return Err("Operación cancelada".to_string());
    }
    let meta = fs::symlink_metadata(src)
        .map_err(|e| format!("Error leyendo '{}': {}", src.display(), e))?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let link_target = fs::read_link(src)
                .map_err(|e| format!("No se pudo leer enlace '{}': {}", src.display(), e))?;
            std::os::unix::fs::symlink(&link_target, dest)
                .map_err(|e| format!("No se pudo crear enlace '{}': {}", dest.display(), e))?;
        }
        ctx.done_items += 1;
        ctx.emit_progress(&name);
        return Ok(());
    }

    if meta.is_dir() {
        fs::create_dir_all(dest)
            .map_err(|e| format!("No se pudo crear carpeta '{}': {}", dest.display(), e))?;
        ctx.done_items += 1;
        ctx.emit_progress(&name);
        let entries = fs::read_dir(src)
            .map_err(|e| format!("No se pudo leer carpeta '{}': {}", src.display(), e))?;
        for entry in entries.flatten() {
            copy_tree_chunked(ctx, &entry.path(), &dest.join(entry.file_name()))?;
        }
        return Ok(());
    }

    copy_file_chunked(ctx, src, dest, &name)
}

fn execute_job(ctx: &mut JobContext<'_>, request: JobRequest) -> Result<JobOutcome, String> {
    match request {
        JobRequest::Copy { sources, dest_dir } => execute_copy_job(ctx, &sources, &dest_dir),
        JobRequest::Move { sources, dest_dir } => execute_move_job(ctx, &sources, &dest_dir),
        JobRequest::Duplicate { sources } => execute_duplicate_job(ctx, &sources),
        JobRequest::Trash { sources } => execute_trash_job(ctx, &sources),
        JobRequest::Compress {
            sources,
            dest_dir,
            format,
        } => execute_compress_job(ctx, &sources, &dest_dir, format),
        JobRequest::Extract { archive, dest_dir } => execute_extract_job(ctx, &archive, &dest_dir),
    }
}

fn execute_copy_job(
    ctx: &mut JobContext<'_>,
    sources: &[PathBuf],
    dest_dir: &Path,
) -> Result<JobOutcome, String> {
    for src in sources {
        let (items, bytes) = count_tree(src);
        ctx.total_items += items;
        ctx.total_bytes += bytes;
    }
    ctx.emit_progress("");

    let mut created = Vec::new();
    let mut last_name = None;
    for src in sources {
        if ctx.is_cancelled() {
            break;
        }
        if src == dest_dir || (src.is_dir() && dest_dir.starts_with(src)) {
            continue;
        }
        let Some(target) = resolve_target_with_conflict(ctx, src, dest_dir)? else {
            let (skipped_items, skipped_bytes) = count_tree(src);
            ctx.done_items += skipped_items;
            ctx.done_bytes += skipped_bytes;
            continue;
        };
        copy_tree_chunked(ctx, src, &target)?;
        last_name = target.file_name().map(|n| n.to_string_lossy().to_string());
        created.push(target);
    }

    let changed = !created.is_empty();
    Ok(JobOutcome {
        changed,
        undo_entry: changed.then_some(UndoEntry::CreatedPaths { paths: created }),
        select_name: last_name,
    })
}

fn execute_move_job(
    ctx: &mut JobContext<'_>,
    sources: &[PathBuf],
    dest_dir: &Path,
) -> Result<JobOutcome, String> {
    ctx.total_items = sources.len().max(1);
    ctx.emit_progress("");

    let mut moved_pairs = Vec::new();
    let mut last_name = None;

    for src in sources {
        if ctx.is_cancelled() {
            break;
        }
        if src == dest_dir
            || src.parent() == Some(dest_dir)
            || (src.is_dir() && dest_dir.starts_with(src))
        {
            ctx.done_items += 1;
            continue;
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        ctx.emit_progress(&name);

        let Some(target) = resolve_target_with_conflict(ctx, src, dest_dir)? else {
            ctx.done_items += 1;
            continue;
        };

        let is_dir = src
            .symlink_metadata()
            .map(|m| m.is_dir())
            .unwrap_or_else(|_| src.is_dir());

        if fs::rename(src, &target).is_err() {
            let (t_items, t_bytes) = count_tree(src);
            ctx.total_items += t_items;
            ctx.total_bytes += t_bytes;
            copy_tree_chunked(ctx, src, &target)?;
            remove_existing_path(src)?;
        } else {
            ctx.done_items += 1;
        }

        if let Ok(mut conn) = crate::db::open_db() {
            let _ = crate::db::rename_item_metadata(&mut conn, src, &target, is_dir);
        }

        last_name = target.file_name().map(|n| n.to_string_lossy().to_string());
        moved_pairs.push((src.clone(), target));
        ctx.emit_progress(&name);
    }

    let changed = !moved_pairs.is_empty();
    Ok(JobOutcome {
        changed,
        undo_entry: changed.then_some(UndoEntry::Moves { pairs: moved_pairs }),
        select_name: last_name,
    })
}

fn execute_duplicate_job(
    ctx: &mut JobContext<'_>,
    sources: &[PathBuf],
) -> Result<JobOutcome, String> {
    for src in sources {
        let (items, bytes) = count_tree(src);
        ctx.total_items += items;
        ctx.total_bytes += bytes;
    }
    ctx.emit_progress("");

    let mut created = Vec::new();
    let mut last_name = None;
    for src in sources {
        if ctx.is_cancelled() {
            break;
        }
        let dest = crate::operations::next_duplicate_path(src)?;
        copy_tree_chunked(ctx, src, &dest)?;
        last_name = dest.file_name().map(|n| n.to_string_lossy().to_string());
        created.push(dest);
    }

    let changed = !created.is_empty();
    Ok(JobOutcome {
        changed,
        undo_entry: changed.then_some(UndoEntry::CreatedPaths { paths: created }),
        select_name: last_name,
    })
}

fn execute_trash_job(ctx: &mut JobContext<'_>, sources: &[PathBuf]) -> Result<JobOutcome, String> {
    ctx.total_items = sources.len().max(1);
    ctx.emit_progress("");

    let mut records = Vec::new();
    for src in sources {
        if ctx.is_cancelled() {
            break;
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        ctx.emit_progress(&name);
        if let Ok(record) = move_to_trash_with_record(src) {
            records.push(record);
        }
        ctx.done_items += 1;
        ctx.emit_progress(&name);
    }

    let changed = !records.is_empty();
    Ok(JobOutcome {
        changed,
        undo_entry: changed.then_some(UndoEntry::Trashed { records }),
        select_name: None,
    })
}

pub fn move_to_trash_with_record(src: &Path) -> Result<TrashedUndoRecord, String> {
    let trash_base = crate::operations::trash_base_dir();
    let files_dir = trash_base.join("files");
    let info_dir = trash_base.join("info");

    let before_names: std::collections::HashSet<String> = fs::read_dir(&files_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    crate::operations::trash_item(src)?;

    let orig_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Detectar qué archivo apareció en Trash/files tras gio::File::trash
    let matched_name = fs::read_dir(&files_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|name| !before_names.contains(name))
        .unwrap_or(orig_name);

    let trash_files_path = files_dir.join(&matched_name);
    let candidate_info = info_dir.join(format!("{}.trashinfo", matched_name));
    let trash_info_path = candidate_info.exists().then_some(candidate_info);

    Ok(TrashedUndoRecord {
        original_path: src.to_path_buf(),
        trash_files_path,
        trash_info_path,
    })
}

fn unique_archive_path(sources: &[PathBuf], dest_dir: &Path, format: ArchiveFormat) -> PathBuf {
    let base = if sources.len() == 1 {
        sources[0]
            .file_stem()
            .or_else(|| sources[0].file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Archivo".to_string())
    } else if malus_de_common::is_spanish_locale() {
        "Archivo".to_string()
    } else {
        "Archive".to_string()
    };
    let ext = format.extension();
    let mut candidate = dest_dir.join(format!("{}.{}", base, ext));
    let mut counter = 2usize;
    while candidate.exists() || candidate.symlink_metadata().is_ok() {
        candidate = dest_dir.join(format!("{} {}.{}", base, counter, ext));
        counter += 1;
    }
    candidate
}

fn run_command_cancellable(
    ctx: &JobContext<'_>,
    mut cmd: std::process::Command,
    current_label: &str,
) -> Result<(), String> {
    let mut child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("No se pudo iniciar comando: {}", e))?;

    loop {
        if ctx.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Operación cancelada".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let mut stderr_msg = String::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr_msg);
                }
                let trimmed = stderr_msg.trim();
                return Err(if trimmed.is_empty() {
                    format!("El comando terminó con código {}", status)
                } else {
                    trimmed.to_string()
                });
            }
            Ok(None) => {
                ctx.emit_progress(current_label);
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            Err(e) => return Err(format!("Error supervisando proceso: {}", e)),
        }
    }
}

fn execute_compress_job(
    ctx: &mut JobContext<'_>,
    sources: &[PathBuf],
    dest_dir: &Path,
    format: ArchiveFormat,
) -> Result<JobOutcome, String> {
    if sources.is_empty() {
        return Ok(JobOutcome {
            changed: false,
            undo_entry: None,
            select_name: None,
        });
    }
    let archive_path = unique_archive_path(sources, dest_dir, format);
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    ctx.total_items = sources.len().max(1);
    ctx.emit_progress(&archive_name);

    let parent_dir = sources[0].parent().unwrap_or(dest_dir);
    let mut rel_names = Vec::with_capacity(sources.len());
    for s in sources {
        if let Some(name) = s.file_name() {
            rel_names.push(name.to_os_string());
        }
    }
    if rel_names.is_empty() {
        return Err("No hay elementos válidos para comprimir".to_string());
    }

    let mut cmd = match format {
        ArchiveFormat::Zip => {
            let mut c = std::process::Command::new("zip");
            c.arg("-r").arg("-q").arg(&archive_path).arg("--");
            c
        }
        ArchiveFormat::TarGz => {
            let mut c = std::process::Command::new("tar");
            c.arg("--force-local").arg("-czf").arg(&archive_path).arg("--");
            c
        }
    };
    cmd.current_dir(parent_dir);
    for name in &rel_names {
        // Prefijar "./" además de "--" garantiza que nombres que empiecen por '-' nunca se interpreten como opciones.
        let mut safe_rel = std::ffi::OsString::from("./");
        safe_rel.push(name);
        cmd.arg(safe_rel);
    }

    if let Err(e) = run_command_cancellable(ctx, cmd, &archive_name) {
        let _ = fs::remove_file(&archive_path);
        return Err(e);
    }

    ctx.done_items = ctx.total_items;
    ctx.emit_progress(&archive_name);

    Ok(JobOutcome {
        changed: true,
        undo_entry: Some(UndoEntry::CreatedPaths {
            paths: vec![archive_path.clone()],
        }),
        select_name: Some(archive_name),
    })
}

pub fn is_supported_archive(path: &Path) -> bool {
    if path.is_dir() {
        return false;
    }
    let lower = path.to_string_lossy().to_lowercase();
    [
        ".zip", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tar", ".7z",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

fn is_safe_archive_entry_path(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return false;
    }
    let normalized = trimmed.replace('\\', "/");
    let p = Path::new(&normalized);
    if p.is_absolute() {
        return false;
    }
    for comp in p.components() {
        if matches!(
            comp,
            std::path::Component::RootDir
                | std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
        ) {
            return false;
        }
    }
    true
}

fn validate_archive_before_extract(archive: &Path, lower: &str) -> Result<(), String> {
    let entries: Vec<String> = if lower.ends_with(".zip") {
        let out = std::process::Command::new("unzip")
            .args(["-Z1", "--"])
            .arg(archive)
            .output()
            .map_err(|e| format!("No se pudo inspeccionar archivo zip: {e}"))?;
        if !out.status.success() {
            return Err("No se pudo validar el contenido del archivo zip".to_string());
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(ToString::to_string)
            .collect()
    } else if lower.ends_with(".7z") {
        let out = std::process::Command::new("7z")
            .args(["l", "-slt", "--"])
            .arg(archive)
            .output()
            .map_err(|e| format!("No se pudo inspeccionar archivo 7z: {e}"))?;
        if !out.status.success() {
            return Err("No se pudo validar el contenido del archivo 7z".to_string());
        }
        let mut parsed = Vec::new();
        let mut seen_header_sep = false;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.trim() == "----------" {
                seen_header_sep = true;
                continue;
            }
            if seen_header_sep {
                if let Some(path_val) = line.strip_prefix("Path = ") {
                    parsed.push(path_val.to_string());
                }
            }
        }
        parsed
    } else {
        let out = std::process::Command::new("tar")
            .args(["--force-local", "-tf"])
            .arg(archive)
            .output()
            .map_err(|e| format!("No se pudo inspeccionar archivo tar: {e}"))?;
        if !out.status.success() {
            return Err("No se pudo validar el contenido del archivo tar".to_string());
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(ToString::to_string)
            .collect()
    };

    for entry in entries {
        if !is_safe_archive_entry_path(&entry) {
            return Err(format!(
                "Archivo comprimido rechazado por ruta insegura (posible path traversal): {}",
                entry
            ));
        }
    }
    Ok(())
}

fn verify_extracted_tree_safe(root: &Path) -> Result<(), String> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|e| format!("No se pudo resolver directorio de extracción: {e}"))?;
    let mut stack = vec![canonical_root.clone()];
    while let Some(dir) = stack.pop() {
        let rd = fs::read_dir(&dir)
            .map_err(|e| format!("Error validando árbol extraído: {e}"))?;
        for entry in rd.flatten() {
            let p = entry.path();
            let meta = fs::symlink_metadata(&p)
                .map_err(|e| format!("Error inspeccionando metadatos extraídos: {e}"))?;
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&p)
                    .map_err(|e| format!("Error leyendo enlace simbólico extraído: {e}"))?;
                if target.is_absolute() {
                    return Err(format!(
                        "Enlace simbólico absoluto rechazado en archivo comprimido: {}",
                        p.display()
                    ));
                }
                let parent = p.parent().unwrap_or(&canonical_root);
                let resolved = fs::canonicalize(parent.join(&target)).unwrap_or_else(|_| {
                    let mut acc = parent.to_path_buf();
                    for comp in target.components() {
                        match comp {
                            std::path::Component::ParentDir => {
                                acc.pop();
                            }
                            std::path::Component::Normal(seg) => acc.push(seg),
                            _ => {}
                        }
                    }
                    acc
                });
                if !resolved.starts_with(&canonical_root) {
                    return Err(format!(
                        "Enlace simbólico fuera del directorio destino rechazado: {}",
                        p.display()
                    ));
                }
            } else if meta.is_dir() {
                stack.push(p);
            }
        }
    }
    Ok(())
}

fn execute_extract_job(
    ctx: &mut JobContext<'_>,
    archive: &Path,
    dest_dir: &Path,
) -> Result<JobOutcome, String> {
    if !archive.is_file() {
        return Err("Archivo comprimido no encontrado".to_string());
    }
    let archive_name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    ctx.total_items = 2;
    ctx.emit_progress(&archive_name);

    let lower = archive.to_string_lossy().to_lowercase();
    validate_archive_before_extract(archive, &lower)?;

    let tmp_dir = dest_dir.join(format!(".branches_extract_{}", ctx.id));
    fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("No se pudo preparar extracción temporal: {}", e))?;

    let cmd = if lower.ends_with(".zip") {
        let mut c = std::process::Command::new("unzip");
        c.arg("-q")
            .arg("-o")
            .arg("-d")
            .arg(&tmp_dir)
            .arg("--")
            .arg(archive);
        c
    } else if lower.ends_with(".7z") {
        let mut c = std::process::Command::new("7z");
        c.arg("x")
            .arg("-y")
            .arg(format!("-o{}", tmp_dir.display()))
            .arg("--")
            .arg(archive);
        c
    } else {
        let mut c = std::process::Command::new("tar");
        c.arg("--force-local")
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(&tmp_dir);
        c
    };

    if let Err(e) = run_command_cancellable(ctx, cmd, &archive_name) {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }

    if let Err(e) = verify_extracted_tree_safe(&tmp_dir) {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }

    let extracted_entries: Vec<PathBuf> = fs::read_dir(&tmp_dir)
        .map_err(|e| format!("Error leyendo contenido extraído: {}", e))?
        .flatten()
        .map(|e| e.path())
        .collect();

    ctx.done_items = 1;
    ctx.total_items = 1 + extracted_entries.len();

    let mut created = Vec::new();
    let mut last_name = None;
    for entry in extracted_entries {
        if ctx.is_cancelled() {
            break;
        }
        let Some(target) = resolve_target_with_conflict(ctx, &entry, dest_dir)? else {
            ctx.done_items += 1;
            continue;
        };
        if fs::rename(&entry, &target).is_err() {
            copy_tree_chunked(ctx, &entry, &target)?;
        } else {
            ctx.done_items += 1;
        }
        last_name = target.file_name().map(|n| n.to_string_lossy().to_string());
        created.push(target);
        ctx.emit_progress(last_name.as_deref().unwrap_or(&archive_name));
    }

    let _ = fs::remove_dir_all(&tmp_dir);

    let changed = !created.is_empty();
    Ok(JobOutcome {
        changed,
        undo_entry: changed.then_some(UndoEntry::CreatedPaths { paths: created }),
        select_name: last_name,
    })
}

/// Ejecuta la acción inversa de un `UndoEntry`.
pub fn apply_undo_entry(entry: UndoEntry) -> Result<Vec<PathBuf>, String> {
    let mut affected = Vec::new();
    match entry {
        UndoEntry::Rename { from, to } => {
            if !to.exists() && to.symlink_metadata().is_err() {
                return Err("El elemento a deshacer ya no existe".to_string());
            }
            if from.exists() || from.symlink_metadata().is_ok() {
                return Err("Ya existe un elemento en el nombre original".to_string());
            }
            let is_dir = to.is_dir();
            fs::rename(&to, &from).map_err(|e| format!("Error al deshacer renombrado: {}", e))?;
            if let Ok(mut conn) = crate::db::open_db() {
                let _ = crate::db::rename_item_metadata(&mut conn, &to, &from, is_dir);
            }
            affected.push(from);
        }
        UndoEntry::Moves { pairs } => {
            for (orig, dest) in pairs.into_iter().rev() {
                if !dest.exists() && dest.symlink_metadata().is_err() {
                    continue;
                }
                if orig.exists() || orig.symlink_metadata().is_ok() {
                    continue;
                }
                let is_dir = dest.is_dir();
                if let Some(parent) = orig.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let gio_dest = gio::File::for_path(&dest);
                let gio_orig = gio::File::for_path(&orig);
                if gio_dest
                    .move_(
                        &gio_orig,
                        gio::FileCopyFlags::NONE,
                        gio::Cancellable::NONE,
                        None,
                    )
                    .or_else(|_| fs::rename(&dest, &orig))
                    .is_ok()
                {
                    if let Ok(mut conn) = crate::db::open_db() {
                        let _ = crate::db::rename_item_metadata(&mut conn, &dest, &orig, is_dir);
                    }
                    affected.push(orig);
                }
            }
        }
        UndoEntry::CreatedPaths { paths } => {
            for p in paths {
                if p.exists() || p.symlink_metadata().is_ok() {
                    if crate::operations::trash_item(&p).is_ok() {
                        affected.push(p);
                    }
                }
            }
        }
        UndoEntry::Trashed { records } => {
            for rec in records {
                if !rec.trash_files_path.exists()
                    && rec.trash_files_path.symlink_metadata().is_err()
                {
                    continue;
                }
                if rec.original_path.exists() || rec.original_path.symlink_metadata().is_ok() {
                    continue;
                }
                if let Some(parent) = rec.original_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let src_f = gio::File::for_path(&rec.trash_files_path);
                let dst_f = gio::File::for_path(&rec.original_path);
                if src_f
                    .move_(
                        &dst_f,
                        gio::FileCopyFlags::NONE,
                        gio::Cancellable::NONE,
                        None,
                    )
                    .or_else(|_| fs::rename(&rec.trash_files_path, &rec.original_path))
                    .is_ok()
                {
                    if let Some(info_p) = rec.trash_info_path {
                        let _ = fs::remove_file(info_p);
                    }
                    affected.push(rec.original_path);
                }
            }
        }
    }
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_events(rx: &mpsc::Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let is_done = matches!(ev, WorkerEvent::Finished(_));
            out.push(ev);
            if is_done {
                break;
            }
        }
        out
    }

    #[test]
    fn async_copy_conflict_resolution_and_undo_roundtrip() {
        let root = std::env::temp_dir().join("malus_branches_jobs_test_copy");
        let _ = fs::remove_dir_all(&root);
        let src_dir = root.join("src");
        let dst_dir = root.join("dst");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        let src_file = src_dir.join("nota.txt");
        let existing_dst = dst_dir.join("nota.txt");
        fs::write(&src_file, b"contenido nuevo").unwrap();
        fs::write(&existing_dst, b"contenido viejo").unwrap();

        // Responder "KeepBoth" al conflicto
        let (_handle, rx) = spawn_job(
            JobRequest::Copy {
                sources: vec![src_file.clone()],
                dest_dir: dst_dir.clone(),
            },
            false,
            true,
        );

        let mut finished = None;
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            match ev {
                WorkerEvent::Conflict { reply, prompt, .. } => {
                    assert_eq!(prompt.target, existing_dst);
                    let _ = reply.send(ConflictDecision {
                        choice: ConflictChoice::KeepBoth,
                        apply_to_all: false,
                    });
                }
                WorkerEvent::Finished(comp) => {
                    finished = Some(comp);
                    break;
                }
                WorkerEvent::Progress(_) => {}
            }
        }

        let comp = finished.expect("Debe terminar el trabajo");
        assert!(comp.changed);
        assert!(comp.error.is_none());
        let copy_path = dst_dir.join("nota copia.txt");
        assert!(copy_path.exists());
        assert_eq!(fs::read(&existing_dst).unwrap(), b"contenido viejo");
        assert_eq!(fs::read(&copy_path).unwrap(), b"contenido nuevo");

        // Undo Rename
        let renamed = dst_dir.join("renombrado.txt");
        fs::rename(&copy_path, &renamed).unwrap();
        let _ = apply_undo_entry(UndoEntry::Rename {
            from: copy_path.clone(),
            to: renamed.clone(),
        })
        .unwrap();
        assert!(copy_path.exists());
        assert!(!renamed.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn async_compress_and_extract_zip_and_tar_gz() {
        let root = std::env::temp_dir().join("malus_branches_jobs_test_archive");
        let _ = fs::remove_dir_all(&root);
        let src_dir = root.join("src");
        let out_dir = root.join("extracted");
        fs::create_dir_all(src_dir.join("carpeta")).unwrap();
        fs::create_dir_all(&out_dir).unwrap();
        fs::write(src_dir.join("carpeta/hola.txt"), b"malus archive test").unwrap();

        // Comprimir en .zip
        let (_h1, rx1) = spawn_job(
            JobRequest::Compress {
                sources: vec![src_dir.join("carpeta")],
                dest_dir: root.clone(),
                format: ArchiveFormat::Zip,
            },
            false,
            true,
        );
        let events1 = drain_events(&rx1);
        let Some(WorkerEvent::Finished(comp1)) = events1.last() else {
            panic!("Se esperaba Finished");
        };
        assert!(comp1.changed, "Compress zip error: {:?}", comp1.error);
        let zip_path = root.join("carpeta.zip");
        assert!(zip_path.exists());
        assert!(is_supported_archive(&zip_path));

        // Extraer .zip
        let (_h2, rx2) = spawn_job(
            JobRequest::Extract {
                archive: zip_path,
                dest_dir: out_dir.clone(),
            },
            false,
            true,
        );
        let events2 = drain_events(&rx2);
        let Some(WorkerEvent::Finished(comp2)) = events2.last() else {
            panic!("Se esperaba Finished");
        };
        assert!(comp2.changed, "Extract zip error: {:?}", comp2.error);
        assert_eq!(
            fs::read(out_dir.join("carpeta/hola.txt")).unwrap(),
            b"malus archive test"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn async_job_supports_cancellation() {
        let root = std::env::temp_dir().join("malus_branches_jobs_test_cancel");
        let _ = fs::remove_dir_all(&root);
        let src_dir = root.join("src");
        let dst_dir = root.join("dst");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        let big = src_dir.join("grande.bin");
        fs::write(&big, vec![7u8; 2 * 1024 * 1024]).unwrap();

        let (handle, rx) = spawn_job(
            JobRequest::Copy {
                sources: vec![big],
                dest_dir: dst_dir.clone(),
            },
            false,
            true,
        );
        handle.cancel_flag.store(true, Ordering::Relaxed);
        let events = drain_events(&rx);
        let Some(WorkerEvent::Finished(comp)) = events.last() else {
            panic!("Se esperaba Finished");
        };
        assert!(comp.cancelled);
        assert!(!comp.changed);
        assert!(!dst_dir.join("grande.bin").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_security_rejects_traversal_and_escaping_symlinks() {
        assert!(is_safe_archive_entry_path("carpeta/archivo.txt"));
        assert!(!is_safe_archive_entry_path("/etc/passwd"));
        assert!(!is_safe_archive_entry_path("../escape.txt"));
        assert!(!is_safe_archive_entry_path("a/../../escape.txt"));
        assert!(!is_safe_archive_entry_path("..\\escape.txt"));

        let root = std::env::temp_dir().join(format!(
            "malus_branches_symlink_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ok.txt"), b"ok").unwrap();
        assert!(verify_extracted_tree_safe(&root).is_ok());

        let bad_link = root.join("escape_link");
        std::os::unix::fs::symlink("/etc/hosts", &bad_link).unwrap();
        assert!(verify_extracted_tree_safe(&root).is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
