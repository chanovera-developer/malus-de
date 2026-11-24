use anyhow::{bail, Context, Result};
use smithay::utils::{Physical, Rectangle};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{sync_channel, SyncSender, TrySendError},
    thread::{self, JoinHandle},
    time::Instant,
};
use tracing::{error, info, warn};

/// Valida que la ruta destino de una grabación sea un archivo `.webm` local,
/// absoluto, sin traversal (`..`), no existente previamente, y dentro de un
/// directorio del usuario permitido (`$HOME`, `$XDG_VIDEOS_DIR` o `$XDG_RUNTIME_DIR`).
pub fn validate_recording_destination(destination: &Path) -> Result<PathBuf> {
    if !destination.is_absolute() {
        bail!("La ruta de grabación debe ser absoluta");
    }
    if destination
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("La ruta de grabación no puede contener componentes '..'");
    }

    let file_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .context("La ruta de grabación no tiene un nombre de archivo UTF-8 válido")?;
    if file_name.is_empty() || file_name.starts_with('-') || file_name.starts_with('.') {
        bail!("Nombre de archivo de grabación inválido");
    }

    let ext_ok = destination
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    if !ext_ok {
        bail!("La grabación de pantalla solo permite archivos .webm");
    }

    let allowed_roots: Vec<PathBuf> = ["HOME", "XDG_VIDEOS_DIR", "XDG_RUNTIME_DIR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .filter_map(|p| std::fs::canonicalize(&p).ok().or(Some(p)))
        .collect();

    if allowed_roots.is_empty() {
        bail!("No se pudo determinar un directorio base permitido para grabaciones");
    }

    // Verificar ancestro existente más cercano para evitar escapes por enlaces simbólicos
    let mut existing_ancestor = destination.parent().unwrap_or(destination);
    while !existing_ancestor.exists() {
        match existing_ancestor.parent() {
            Some(p) => existing_ancestor = p,
            None => break,
        }
    }
    let canonical_ancestor = std::fs::canonicalize(existing_ancestor)
        .with_context(|| format!("No se pudo resolver {}", existing_ancestor.display()))?;
    if !allowed_roots
        .iter()
        .any(|root| canonical_ancestor.starts_with(root))
    {
        bail!("La ruta de grabación debe residir dentro del directorio del usuario");
    }

    if std::fs::symlink_metadata(destination).is_ok() {
        bail!("La ruta de grabación ya existe; se rechaza sobrescribir archivos existentes");
    }

    let relative_tail = destination
        .strip_prefix(existing_ancestor)
        .unwrap_or(Path::new(file_name));
    Ok(canonical_ancestor.join(relative_tail))
}

pub struct ScreenRecordingSession {
    pub destination: PathBuf,
    pub rect: Option<Rectangle<i32, Physical>>,
    pub target_fps: u32,
    frame_sender: Option<SyncSender<Vec<u8>>>,
    recycle_receiver: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    worker_handle: Option<JoinHandle<()>>,
    pub last_frame: Instant,
    pub start_time: Instant,
    pub frame_width: u32,
    pub frame_height: u32,
}

impl ScreenRecordingSession {
    pub fn start(
        destination: PathBuf,
        rect: Option<Rectangle<i32, Physical>>,
        width: u32,
        height: u32,
        include_audio: bool,
        fps: u32,
        broadcaster: malus_de_common::ipc::IpcBroadcaster,
    ) -> Result<Self> {
        let destination = validate_recording_destination(&destination)?;
        let w = (width & !1).max(2);
        let h = (height & !1).max(2);
        let fps = fps.clamp(10, 60);

        if let Some(parent) = destination.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-n",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &format!("{w}x{h}"),
            "-framerate",
            &fps.to_string(),
            "-i",
            "-",
        ]);

        if include_audio {
            cmd.args(["-f", "pulse", "-i", "default"]);
        }

        // Formato abierto estándar WebM con códec VP8/VP9 y Opus (100% libre de patentes)
        cmd.args([
            "-c:v",
            "libvpx",
            "-deadline",
            "realtime",
            "-quality",
            "realtime",
            "-cpu-used",
            "6",
            "-b:v",
            "3M",
            "-pix_fmt",
            "yuv420p",
        ]);

        if include_audio {
            cmd.args(["-c:a", "libopus", "-b:a", "128k"]);
        }

        let dest_str = destination
            .to_str()
            .context("La ruta de grabación no es UTF-8 válida")?;
        cmd.arg(format!("file:{dest_str}"));

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let mut child = cmd
            .spawn()
            .context("No se pudo iniciar ffmpeg para grabación de pantalla")?;
        let mut stdin = child
            .stdin
            .take()
            .context("No se pudo abrir stdin de ffmpeg")?;

        let (sender, receiver) = sync_channel::<Vec<u8>>(4);
        let (recycle_tx, recycle_rx) = sync_channel::<Vec<u8>>(4);

        let dest_clone = destination.clone();
        let worker_handle = thread::spawn(move || {
            let mut total_frames = 0usize;
            while let Ok(frame) = receiver.recv() {
                if let Err(e) = stdin.write_all(&frame) {
                    error!("Fallo al escribir fotograma en ffmpeg: {e}");
                    break;
                }
                total_frames += 1;
                let _ = recycle_tx.try_send(frame);
            }
            drop(stdin);

            let status = child.wait();
            let success = match status {
                Ok(s) => s.success(),
                Err(e) => {
                    error!("Error al esperar que ffmpeg finalice: {e}");
                    false
                }
            };
            info!(
                "Grabación de pantalla finalizada: {} fotogramas, éxito: {}",
                total_frames, success
            );

            if success {
                let path_str = dest_clone.to_string_lossy().to_string();
                let _ = Command::new("notify-send")
                    .args([
                        "-a",
                        "Malus DE",
                        "-i",
                        "video-x-generic",
                        "Grabación de pantalla completada",
                        &format!("Guardada en {}", path_str),
                    ])
                    .spawn();
            }

            broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ScreenRecordingFinished {
                destination: dest_clone.to_string_lossy().to_string(),
                success,
                error: if success {
                    None
                } else {
                    Some("ffmpeg finalizó con error".to_string())
                },
            });
        });

        info!(
            "Grabación iniciada en formato abierto WebM: {}x{} @ {} fps en {}",
            w,
            h,
            fps,
            destination.display()
        );

        Ok(Self {
            destination,
            rect,
            target_fps: fps,
            frame_sender: Some(sender),
            recycle_receiver: Some(recycle_rx),
            worker_handle: Some(worker_handle),
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_width: w,
            frame_height: h,
        })
    }

    pub fn push_frame(&mut self, frame: Vec<u8>) {
        if let Some(ref sender) = self.frame_sender {
            match sender.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    warn!("El canal de grabación de pantalla se desconectó");
                }
            }
        }
    }

    /// Copia `pixels` en un búfer reciclado del pool de grabación para evitar asignaciones
    /// de varios megabytes en el heap en cada fotograma.
    pub fn push_frame_slice(&mut self, pixels: &[u8]) {
        let mut buf = self
            .recycle_receiver
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
            .unwrap_or_else(|| Vec::with_capacity(pixels.len()));
        buf.clear();
        buf.extend_from_slice(pixels);
        self.push_frame(buf);
    }
}

impl Drop for ScreenRecordingSession {
    fn drop(&mut self) {
        // Cerrar el canal señala EOF al hilo de ffmpeg; NO hacemos `join()` en el hilo principal
        // del compositor para evitar congelar el escritorio mientras ffmpeg vacía su búfer.
        drop(self.frame_sender.take());
        drop(self.recycle_receiver.take());
        let _ = self.worker_handle.take();
    }
}

#[cfg(test)]
mod tests {
    use super::validate_recording_destination;
    use std::path::PathBuf;

    #[test]
    fn validates_recording_destination_paths() {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME requerido en pruebas"));
        let valid = home.join("malus-test-recording-nonexistent-987654.webm");
        assert!(validate_recording_destination(&valid).is_ok());

        // Rechaza rutas relativas
        assert!(validate_recording_destination(std::path::Path::new("video.webm")).is_err());
        // Rechaza path traversal
        assert!(validate_recording_destination(&home.join("../etc/out.webm")).is_err());
        // Rechaza extensiones no webm
        assert!(validate_recording_destination(&home.join(".bashrc")).is_err());
        // Rechaza nombres que empiezan por '-' (inyección de argumentos)
        assert!(validate_recording_destination(&home.join("-flag.webm")).is_err());
        // Rechaza rutas fuera del directorio del usuario
        assert!(validate_recording_destination(std::path::Path::new("/etc/evil.webm")).is_err());
    }
}
