use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            gles::{GlesMapping, GlesRenderer, GlesTarget},
            ExportMem,
        },
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle},
};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        mpsc::{sync_channel, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
        OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tracing::{error, info, warn};

/// Perfil de codificación seleccionado para una sesión de grabación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoEncoderBackend {
    /// H.264 acelerado por GPU vía VA-API con CQP 16 (calidad casi indistinguible y CPU mínima).
    VaapiH264 { render_node: PathBuf },
    /// H.264 por CPU (fallback si no hay VA-API disponible) con CRF 15.
    SoftwareH264,
    /// H.264 RGB 4:4:4 sin pérdidas (CRF 0, píxel a píxel exacto 1:1).
    LosslessRgb,
    /// Fallback VP9/VP8 si el destino es explícitamente `.webm`.
    WebmVp9,
}

static CACHED_VAAPI_NODE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Detecta y valida (una sola vez por proceso) un nodo DRM render con soporte real de `h264_vaapi`.
fn detect_vaapi_h264_device() -> Option<PathBuf> {
    CACHED_VAAPI_NODE
        .get_or_init(|| {
            let mut candidates: Vec<PathBuf> = (128..136)
                .map(|n| PathBuf::from(format!("/dev/dri/renderD{n}")))
                .filter(|p| p.exists())
                .collect();
            if candidates.is_empty() {
                return None;
            }
            candidates.sort();
            for node in candidates {
                let Some(node_str) = node.to_str() else {
                    continue;
                };
                let ok = Command::new("ffmpeg")
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-init_hw_device",
                        &format!("vaapi=va:{node_str}"),
                        "-filter_hw_device",
                        "va",
                        "-f",
                        "lavfi",
                        "-i",
                        "color=c=black:s=128x128:d=0.05:r=30",
                        "-vf",
                        "format=rgb0,hwupload,scale_vaapi=format=nv12:out_color_matrix=bt709:out_range=tv",
                        "-c:v",
                        "h264_vaapi",
                        "-rc_mode",
                        "CQP",
                        "-qp",
                        "16",
                        "-bf",
                        "0",
                        "-f",
                        "null",
                        "-",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if ok {
                    info!(
                        "Codificación por hardware VA-API H.264 disponible en {}",
                        node.display()
                    );
                    return Some(node);
                }
            }
            warn!("VA-API H.264 no disponible; se usará libx264 por CPU");
            None
        })
        .clone()
}

/// Valida que la ruta destino de una grabación sea un archivo de video local
/// (`.mp4`, `.mkv` o `.webm`), absoluto, sin traversal (`..`), no existente previamente,
/// y dentro de un directorio del usuario permitido (`$HOME`, `$XDG_VIDEOS_DIR` o `$XDG_RUNTIME_DIR`).
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
        .is_some_and(|e| {
            e.eq_ignore_ascii_case("mp4")
                || e.eq_ignore_ascii_case("mkv")
                || e.eq_ignore_ascii_case("webm")
        });
    if !ext_ok {
        bail!("La grabación de pantalla solo permite archivos .mp4, .mkv o .webm");
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

fn select_encoder_backend(destination: &Path, lossless: bool) -> VideoEncoderBackend {
    let is_webm = destination
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    if is_webm && !lossless {
        return VideoEncoderBackend::WebmVp9;
    }
    if lossless {
        return VideoEncoderBackend::LosslessRgb;
    }
    if let Some(render_node) = detect_vaapi_h264_device() {
        VideoEncoderBackend::VaapiH264 { render_node }
    } else {
        VideoEncoderBackend::SoftwareH264
    }
}

/// Construye la lista completa de argumentos para `ffmpeg` según resolución, FPS, audio y backend.
pub fn build_ffmpeg_args(
    destination: &Path,
    width: u32,
    height: u32,
    fps: u32,
    use_audio: bool,
    backend: &VideoEncoderBackend,
) -> Result<Vec<String>> {
    let dest_str = destination
        .to_str()
        .context("La ruta de grabación no es UTF-8 válida")?;
    let is_mp4 = destination
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    let is_webm = destination
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    let gop = (fps * 2).to_string();
    let mut args = vec![
        "-n".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ];

    if let VideoEncoderBackend::VaapiH264 { render_node } = backend {
        let node_str = render_node
            .to_str()
            .context("La ruta del nodo VA-API no es UTF-8 válida")?;
        args.extend([
            "-init_hw_device".to_string(),
            format!("vaapi=va:{node_str}"),
            "-filter_hw_device".to_string(),
            "va".to_string(),
        ]);
    }

    // Entrada de video crudo desde el compositor (Fourcc::Abgr8888 en LE = bytes [R,G,B,A] == rgb0).
    // El hilo dedicado mantiene un reloj CFR exacto a `fps`, por lo que `ffmpeg` recibe cadencia
    // estable sincronizada con el reloj real sin deriva ni saltos.
    args.extend([
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "rgb0".to_string(),
        "-video_size".to_string(),
        format!("{width}x{height}"),
        "-framerate".to_string(),
        fps.to_string(),
        "-i".to_string(),
        "-".to_string(),
    ]);

    if use_audio {
        args.extend([
            "-thread_queue_size".to_string(),
            "1024".to_string(),
            "-f".to_string(),
            "pulse".to_string(),
            "-i".to_string(),
            "default".to_string(),
        ]);
    }

    match backend {
        VideoEncoderBackend::VaapiH264 { .. } => {
            // Conversión de color BT.709 directamente en la GPU (cero CPU) + CQP 16 (casi indistinguible)
            args.extend([
                "-vf".to_string(),
                "hwupload,scale_vaapi=format=nv12:out_color_matrix=bt709:out_range=tv".to_string(),
                "-c:v".to_string(),
                "h264_vaapi".to_string(),
                "-rc_mode".to_string(),
                "CQP".to_string(),
                "-qp".to_string(),
                "16".to_string(),
                "-bf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                gop,
                "-colorspace".to_string(),
                "bt709".to_string(),
                "-color_primaries".to_string(),
                "bt709".to_string(),
                "-color_trc".to_string(),
                "bt709".to_string(),
                "-color_range".to_string(),
                "tv".to_string(),
            ]);
        }
        VideoEncoderBackend::SoftwareH264 => {
            args.extend([
                "-vf".to_string(),
                "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p".to_string(),
                "-c:v".to_string(),
                "libx264".to_string(),
                "-preset".to_string(),
                "veryfast".to_string(),
                "-crf".to_string(),
                "15".to_string(),
                "-g".to_string(),
                gop,
                "-colorspace".to_string(),
                "bt709".to_string(),
                "-color_primaries".to_string(),
                "bt709".to_string(),
                "-color_trc".to_string(),
                "bt709".to_string(),
                "-color_range".to_string(),
                "tv".to_string(),
            ]);
        }
        VideoEncoderBackend::LosslessRgb => {
            // Modo sin pérdidas real: RGB 4:4:4 (gbrp) CRF 0 sin submuestreo de croma ni cuantización.
            args.extend([
                "-c:v".to_string(),
                "libx264rgb".to_string(),
                "-preset".to_string(),
                "ultrafast".to_string(),
                "-crf".to_string(),
                "0".to_string(),
                "-g".to_string(),
                gop,
            ]);
        }
        VideoEncoderBackend::WebmVp9 => {
            args.extend([
                "-c:v".to_string(),
                "libvpx-vp9".to_string(),
                "-deadline".to_string(),
                "realtime".to_string(),
                "-cpu-used".to_string(),
                "8".to_string(),
                "-crf".to_string(),
                "18".to_string(),
                "-b:v".to_string(),
                "0".to_string(),
                "-row-mt".to_string(),
                "1".to_string(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
            ]);
        }
    }

    if use_audio {
        if is_webm {
            args.extend([
                "-c:a".to_string(),
                "libopus".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else {
            args.extend([
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        }
    }

    if is_mp4 {
        args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    }

    args.push(format!("file:{dest_str}"));
    Ok(args)
}

pub struct ScreenRecordingSession {
    pub destination: PathBuf,
    pub output_name: Option<String>,
    pub rect: Option<Rectangle<i32, Physical>>,
    pub target_fps: u32,
    pub lossless: bool,
    frame_sender: Option<SyncSender<Vec<u8>>>,
    recycle_receiver: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    worker_handle: Option<JoinHandle<()>>,
    /// PBO asíncrono del fotograma anterior (permite leer GPU→CPU sin bloquear el renderizado).
    pending_mapping: Option<GlesMapping>,
    pending_mapping_at: Option<Instant>,
    pub first_frame_captured: bool,
    pub last_frame: Instant,
    pub last_status_broadcast: Instant,
    pub start_time: Instant,
    pub frame_width: u32,
    pub frame_height: u32,
}

fn pulse_audio_available() -> bool {
    Command::new("pactl")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl ScreenRecordingSession {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        destination: PathBuf,
        output_name: Option<String>,
        rect: Option<Rectangle<i32, Physical>>,
        width: u32,
        height: u32,
        include_audio: bool,
        fps: u32,
        lossless: bool,
        broadcaster: malus_de_common::ipc::IpcBroadcaster,
        wayland_socket: String,
    ) -> Result<Self> {
        let destination = validate_recording_destination(&destination)?;
        let w = (width & !1).max(2);
        let h = (height & !1).max(2);
        let fps = fps.clamp(10, 120);

        if let Some(parent) = destination.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let use_audio = if include_audio {
            if pulse_audio_available() {
                true
            } else {
                warn!("PulseAudio/PipeWire-Pulse no disponible; grabando solo video");
                false
            }
        } else {
            false
        };

        let backend = select_encoder_backend(&destination, lossless);
        let ffmpeg_args = build_ffmpeg_args(&destination, w, h, fps, use_audio, &backend)?;

        let mut cmd = Command::new("ffmpeg");
        cmd.args(&ffmpeg_args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("No se pudo iniciar ffmpeg para grabación de pantalla")?;
        let mut stdin = child
            .stdin
            .take()
            .context("No se pudo abrir stdin de ffmpeg")?;
        let stderr = child.stderr.take();

        let (sender, receiver) = sync_channel::<Vec<u8>>(8);
        let (recycle_tx, recycle_rx) = sync_channel::<Vec<u8>>(8);

        let dest_clone = destination.clone();
        let worker_handle = thread::spawn(move || {
            let frame_interval = Duration::from_nanos(1_000_000_000u64 / fps.max(1) as u64);
            let mut total_frames = 0u64;
            let mut current_frame: Option<Vec<u8>> = None;

            // Esperar al primer fotograma capturado antes de iniciar el reloj CFR
            match receiver.recv() {
                Ok(first) => {
                    current_frame = Some(first);
                }
                Err(_) => {
                    // Se detuvo antes de recibir el primer fotograma
                }
            }

            if let Some(mut active_frame) = current_frame.take() {
                let clock_origin = Instant::now();
                let mut stream_closed = false;

                // Escribir el primer fotograma inmediatamente (t = 0)
                if let Err(e) = stdin.write_all(&active_frame) {
                    error!("Fallo al escribir el primer fotograma en ffmpeg: {e}");
                    stream_closed = true;
                } else {
                    total_frames = 1;
                }

                // Bucle de cadencia CFR exacta:
                // - Si llegan nuevos fotogramas, siempre promovemos el más reciente reciclando el anterior.
                // - Si el escritorio está en reposo (Idle Sleeping), duplicamos el último fotograma a
                //   intervalos exactos de `1 / fps` sin deriva acumulada.
                while !stream_closed {
                    let target_time = clock_origin
                        + Duration::from_nanos(
                            total_frames.saturating_mul(frame_interval.as_nanos() as u64),
                        );

                    loop {
                        let now = Instant::now();
                        if now >= target_time {
                            // Drenar cualquier fotograma ya listo sin bloquear
                            loop {
                                match receiver.try_recv() {
                                    Ok(newer) => {
                                        let old = std::mem::replace(&mut active_frame, newer);
                                        let _ = recycle_tx.try_send(old);
                                    }
                                    Err(TryRecvError::Empty) => break,
                                    Err(TryRecvError::Disconnected) => {
                                        stream_closed = true;
                                        break;
                                    }
                                }
                            }
                            break;
                        }

                        let wait = target_time.saturating_duration_since(now);
                        match receiver.recv_timeout(wait) {
                            Ok(newer) => {
                                let old = std::mem::replace(&mut active_frame, newer);
                                let _ = recycle_tx.try_send(old);
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => {
                                stream_closed = true;
                                break;
                            }
                        }
                    }

                    if stream_closed {
                        // Vaciar el último fotograma pendiente antes de cerrar EOF
                        let _ = stdin.write_all(&active_frame);
                        total_frames += 1;
                        break;
                    }

                    if let Err(e) = stdin.write_all(&active_frame) {
                        error!("Fallo al escribir fotograma en ffmpeg: {e}");
                        break;
                    }
                    total_frames += 1;

                    // Si el codificador se retrasó momentáneamente respecto al reloj real (p. ej.
                    // lossless en alta resolución), emitir a lo sumo 2 fotogramas de alcance seguidos
                    // para mantener sincronía A/V sin generar ráfagas interminables.
                    let behind = Instant::now().saturating_duration_since(target_time);
                    if behind > frame_interval * 4 {
                        // Resincronizar suavemente si hubo un bloqueo extremo del disco/CPU
                        let skipped = (behind.as_nanos() / frame_interval.as_nanos().max(1)) as u64;
                        let dup_cap = skipped.min(2);
                        for _ in 0..dup_cap {
                            if stdin.write_all(&active_frame).is_err() {
                                stream_closed = true;
                                break;
                            }
                            total_frames += 1;
                        }
                    }
                }
                let _ = recycle_tx.try_send(active_frame);
            }

            drop(stdin);

            let mut err_text = String::new();
            if let Some(mut err_pipe) = stderr {
                use std::io::Read;
                let _ = err_pipe.read_to_string(&mut err_text);
            }

            let status = child.wait();
            let success = match status {
                Ok(s) => s.success() && dest_clone.is_file(),
                Err(e) => {
                    error!("Error al esperar que ffmpeg finalice: {e}");
                    false
                }
            };
            if !success && !err_text.trim().is_empty() {
                error!("ffmpeg stderr: {}", err_text.trim());
            }
            info!(
                "Grabación de pantalla finalizada: {} fotogramas @ {} fps, éxito: {}",
                total_frames, fps, success
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

                // Mostrar miniatura flotante en la esquina inferior derecha con acciones rápidas
                let screenshot_bin = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("malus-de-screenshot")))
                    .filter(|p| p.is_file())
                    .unwrap_or_else(|| PathBuf::from("malus-de-screenshot"));
                let mut ui_cmd = Command::new(screenshot_bin);
                ui_cmd
                    .args(["--show-recording", &path_str])
                    .env("WAYLAND_DISPLAY", &wayland_socket)
                    .env("GDK_BACKEND", "wayland");
                malus_de_common::config::set_cursor_environment(&mut ui_cmd);
                let _ = ui_cmd.spawn();
            }

            broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ScreenRecordingStatus {
                is_recording: false,
                elapsed_secs: 0,
            });
            broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ScreenRecordingFinished {
                destination: dest_clone.to_string_lossy().to_string(),
                success,
                error: if success {
                    None
                } else {
                    let msg = err_text.trim();
                    Some(if msg.is_empty() {
                        "ffmpeg finalizó con error".to_string()
                    } else {
                        format!("ffmpeg finalizó con error: {msg}")
                    })
                },
            });
        });

        let quality_label = match &backend {
            VideoEncoderBackend::VaapiH264 { render_node } => {
                format!("VA-API H.264 CQP 16 ({})", render_node.display())
            }
            VideoEncoderBackend::SoftwareH264 => "libx264 CRF 15".to_string(),
            VideoEncoderBackend::LosslessRgb => "Sin pérdidas RGB 1:1 (libx264rgb CRF 0)".to_string(),
            VideoEncoderBackend::WebmVp9 => "VP9 CRF 18".to_string(),
        };

        info!(
            "Grabación iniciada [{}]: {}x{} @ {} fps en {}",
            quality_label,
            w,
            h,
            fps,
            destination.display()
        );

        let now = Instant::now();
        Ok(Self {
            destination,
            output_name,
            rect,
            target_fps: fps,
            lossless,
            frame_sender: Some(sender),
            recycle_receiver: Some(recycle_rx),
            worker_handle: Some(worker_handle),
            pending_mapping: None,
            pending_mapping_at: None,
            first_frame_captured: false,
            last_frame: now - Duration::from_secs(1),
            last_status_broadcast: now,
            start_time: now,
            frame_width: w,
            frame_height: h,
        })
    }

    pub fn matches_output(&self, name: &str) -> bool {
        self.output_name
            .as_deref()
            .is_none_or(|target| target == name)
    }

    /// Intervalo mínimo entre capturas en el compositor. Usa el 85% del periodo teórico para
    /// absorber el jitter natural del VBlank (p. ej. 16.3 ms en 60 Hz) y nunca saltar fotogramas.
    fn capture_min_interval(&self) -> Duration {
        Duration::from_nanos(850_000_000u64 / self.target_fps.max(1) as u64)
    }

    /// Indica si debemos capturar el fotograma que acaba de renderizar el compositor.
    pub fn frame_due(&self, now: Instant) -> bool {
        !self.first_frame_captured
            || now.duration_since(self.last_frame) >= self.capture_min_interval()
    }

    /// Indica si el bucle del compositor debe despertar incluso en reposo (Idle Sleeping):
    /// solo para capturar el primer fotograma al inicio, o para vaciar un PBO pendiente que lleva
    /// listo más de un periodo de fotograma.
    pub fn needs_render_wakeup(&self, now: Instant) -> bool {
        if !self.first_frame_captured {
            return true;
        }
        self.pending_mapping_ready(now)
    }

    /// Devuelve `true` cuando existe un PBO del fotograma anterior listo para ser leído en CPU
    /// después de haber transcurrido el margen de transferencia DMA.
    pub fn pending_mapping_ready(&self, now: Instant) -> bool {
        if self.pending_mapping.is_none() {
            return false;
        }
        let flush_delay = Duration::from_nanos(1_000_000_000u64 / self.target_fps.max(1) as u64);
        self.pending_mapping_at
            .is_some_and(|t| now.duration_since(t) >= flush_delay)
    }

    pub fn expected_frame_bytes(&self) -> usize {
        self.frame_width as usize * self.frame_height as usize * 4
    }

    /// Mapea y entrega al hilo de codificación cualquier PBO asíncrono pendiente del fotograma anterior.
    pub fn flush_pending_mapping(&mut self, renderer: &mut GlesRenderer) {
        if let Some(prev_mapping) = self.pending_mapping.take() {
            self.pending_mapping_at = None;
            if let Ok(pixels) = renderer.map_texture(&prev_mapping) {
                if pixels.len() == self.expected_frame_bytes() {
                    self.push_frame_slice(pixels);
                }
            }
        }
    }

    /// Captura el fotograma actual mediante doble búfer PBO asíncrono:
    /// 1. Lee el PBO del fotograma N-1 (ya transferido por DMA sin frenar la GPU) y lo envía a ffmpeg.
    /// 2. Lanza `copy_framebuffer` asíncrono para el fotograma N (que se leerá en N+1).
    pub fn capture_frame_async(
        &mut self,
        renderer: &mut GlesRenderer,
        target: &GlesTarget<'_>,
        region: Rectangle<i32, BufferCoord>,
        now: Instant,
    ) {
        let is_first = !self.first_frame_captured;
        self.first_frame_captured = true;
        self.last_frame = now;

        // 1. Leer fotograma anterior (ya completado por la GPU en el periodo previo)
        self.flush_pending_mapping(renderer);

        // 2. Iniciar transferencia DMA asíncrona del fotograma actual hacia un PBO
        if let Ok(mapping) = renderer.copy_framebuffer(target, region, Fourcc::Abgr8888) {
            if is_first {
                // El primer fotograma se entrega inmediatamente para arrancar el reloj CFR en t=0
                if let Ok(pixels) = renderer.map_texture(&mapping) {
                    if pixels.len() == self.expected_frame_bytes() {
                        self.push_frame_slice(pixels);
                    }
                }
            } else {
                self.pending_mapping = Some(mapping);
                self.pending_mapping_at = Some(now);
            }
        }
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
        // del compositor para evitar congelar el escritorio mientras ffmpeg vacía su búfer y
        // escribe el átomo `moov` (+faststart).
        self.pending_mapping.take();
        drop(self.frame_sender.take());
        drop(self.recycle_receiver.take());
        let _ = self.worker_handle.take();
    }
}

#[cfg(test)]
mod tests {
    use super::{build_ffmpeg_args, validate_recording_destination, VideoEncoderBackend};
    use std::path::PathBuf;

    #[test]
    fn validates_recording_destination_paths() {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME requerido en pruebas"));
        let valid_mp4 = home.join("malus-test-recording-nonexistent-987654.mp4");
        let valid_mkv = home.join("malus-test-recording-nonexistent-987654.mkv");
        let valid_webm = home.join("malus-test-recording-nonexistent-987654.webm");
        assert!(validate_recording_destination(&valid_mp4).is_ok());
        assert!(validate_recording_destination(&valid_mkv).is_ok());
        assert!(validate_recording_destination(&valid_webm).is_ok());

        // Rechaza rutas relativas
        assert!(validate_recording_destination(std::path::Path::new("video.mp4")).is_err());
        // Rechaza path traversal
        assert!(validate_recording_destination(&home.join("../etc/out.mp4")).is_err());
        // Rechaza extensiones no soportadas
        assert!(validate_recording_destination(&home.join(".bashrc")).is_err());
        assert!(validate_recording_destination(&home.join("video.avi")).is_err());
        // Rechaza nombres que empiezan por '-' (inyección de argumentos)
        assert!(validate_recording_destination(&home.join("-flag.mp4")).is_err());
        // Rechaza rutas fuera del directorio del usuario
        assert!(validate_recording_destination(std::path::Path::new("/etc/evil.mp4")).is_err());
    }

    #[test]
    fn builds_expected_ffmpeg_args_for_vaapi_software_and_lossless() {
        let dest = PathBuf::from("/home/user/video.mp4");

        let vaapi_args = build_ffmpeg_args(
            &dest,
            1920,
            1080,
            60,
            true,
            &VideoEncoderBackend::VaapiH264 {
                render_node: PathBuf::from("/dev/dri/renderD128"),
            },
        )
        .expect("VA-API args");
        assert!(vaapi_args.contains(&"h264_vaapi".to_string()));
        assert!(vaapi_args.contains(&"16".to_string()));
        assert!(vaapi_args.contains(&"aac".to_string()));
        assert!(vaapi_args.contains(&"+faststart".to_string()));
        assert!(vaapi_args.contains(&"60".to_string()));

        let lossless_args = build_ffmpeg_args(
            &dest,
            1920,
            1080,
            60,
            false,
            &VideoEncoderBackend::LosslessRgb,
        )
        .expect("Lossless args");
        assert!(lossless_args.contains(&"libx264rgb".to_string()));
        assert!(lossless_args.contains(&"0".to_string()));
        assert!(lossless_args.contains(&"ultrafast".to_string()));
        assert!(lossless_args.contains(&"+faststart".to_string()));

        let sw_args = build_ffmpeg_args(
            &dest,
            1280,
            720,
            60,
            false,
            &VideoEncoderBackend::SoftwareH264,
        )
        .expect("Software args");
        assert!(sw_args.contains(&"libx264".to_string()));
        assert!(sw_args.contains(&"15".to_string()));
    }
}
