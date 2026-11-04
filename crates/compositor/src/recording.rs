use anyhow::{Context, Result};
use smithay::utils::{Physical, Rectangle};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{sync_channel, SyncSender, TrySendError},
    thread::{self, JoinHandle},
    time::Instant,
};
use tracing::{error, info, warn};

pub struct ScreenRecordingSession {
    pub destination: PathBuf,
    pub rect: Option<Rectangle<i32, Physical>>,
    pub target_fps: u32,
    frame_sender: Option<SyncSender<Vec<u8>>>,
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
        let w = (width & !1).max(2);
        let h = (height & !1).max(2);
        let fps = fps.clamp(10, 60);

        if let Some(parent) = destination.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-y",
            "-f", "rawvideo",
            "-pixel_format", "rgba",
            "-video_size", &format!("{w}x{h}"),
            "-framerate", &fps.to_string(),
            "-i", "-",
        ]);

        if include_audio {
            cmd.args(["-f", "pulse", "-i", "default"]);
        }

        // Formato abierto estándar WebM con códec VP8/VP9 y Opus (100% libre de patentes)
        cmd.args([
            "-c:v", "libvpx",
            "-deadline", "realtime",
            "-quality", "realtime",
            "-cpu-used", "6",
            "-b:v", "3M",
            "-pix_fmt", "yuv420p",
        ]);

        if include_audio {
            cmd.args(["-c:a", "libopus", "-b:a", "128k"]);
        }

        cmd.arg(destination.to_str().unwrap_or("recording.webm"));

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let mut child = cmd.spawn().context("No se pudo iniciar ffmpeg para grabación de pantalla")?;
        let mut stdin = child.stdin.take().context("No se pudo abrir stdin de ffmpeg")?;

        let (sender, receiver) = sync_channel::<Vec<u8>>(4);

        let dest_clone = destination.clone();
        let worker_handle = thread::spawn(move || {
            let mut total_frames = 0usize;
            while let Ok(frame) = receiver.recv() {
                if let Err(e) = stdin.write_all(&frame) {
                    error!("Fallo al escribir fotograma en ffmpeg: {e}");
                    break;
                }
                total_frames += 1;
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
            info!("Grabación de pantalla finalizada: {} fotogramas, éxito: {}", total_frames, success);

            if success {
                let path_str = dest_clone.to_string_lossy().to_string();
                let _ = Command::new("notify-send")
                    .args([
                        "-a", "Malus DE",
                        "-i", "video-x-generic",
                        "Grabación de pantalla completada",
                        &format!("Guardada en {}", path_str),
                    ])
                    .spawn();
            }

            broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ScreenRecordingFinished {
                destination: dest_clone.to_string_lossy().to_string(),
                success,
                error: if success { None } else { Some("ffmpeg finalizó con error".to_string()) },
            });
        });

        info!(
            "Grabación iniciada en formato abierto WebM: {}x{} @ {} fps en {}",
            w, h, fps, destination.display()
        );

        Ok(Self {
            destination,
            rect,
            target_fps: fps,
            frame_sender: Some(sender),
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
}

impl Drop for ScreenRecordingSession {
    fn drop(&mut self) {
        drop(self.frame_sender.take());
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
