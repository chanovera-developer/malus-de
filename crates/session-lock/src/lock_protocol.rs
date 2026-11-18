//! Cliente ligero del protocolo Wayland `ext-session-lock-v1`.
//!
//! Adquiere un bloqueo criptográfico/de protocolo en el compositor antes de mostrar
//! la interfaz GTK4 y solo envía `unlock_and_destroy` cuando PAM valida al usuario.
//! Si el proceso muere o se cierra sin autenticar, el socket se cierra sin enviar
//! `unlock_and_destroy`, por lo que el compositor mantiene la sesión bloqueada en negro.

use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_v1::{self, ExtSessionLockV1},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockRejection {
    AlreadyLocked,
    ConnectionFailed(String),
}

#[derive(Default)]
struct LockClientState {
    manager: Option<ExtSessionLockManagerV1>,
    locked: bool,
    finished: bool,
}

impl Dispatch<WlRegistry, ()> for LockClientState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "ext_session_lock_manager_v1" {
                let mgr = registry.bind::<ExtSessionLockManagerV1, _, _>(
                    name,
                    version.min(1),
                    qh,
                    (),
                );
                state.manager = Some(mgr);
            }
        }
    }
}

impl Dispatch<ExtSessionLockManagerV1, ()> for LockClientState {
    fn event(
        _: &mut Self,
        _: &ExtSessionLockManagerV1,
        _: <ExtSessionLockManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtSessionLockV1, ()> for LockClientState {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => {
                state.locked = true;
            }
            ext_session_lock_v1::Event::Finished => {
                state.finished = true;
            }
            _ => {}
        }
    }
}

pub struct SessionLockGuard {
    event_queue: EventQueue<LockClientState>,
    state: LockClientState,
    lock: Option<ExtSessionLockV1>,
    _manager: ExtSessionLockManagerV1,
}

impl SessionLockGuard {
    /// Intenta adquirir el bloqueo de sesión `ext-session-lock-v1`.
    /// Devuelve `Ok(None)` únicamente si el compositor padre no anuncia el protocolo.
    pub fn acquire() -> Result<Option<Self>, LockRejection> {
        let conn = Connection::connect_to_env()
            .map_err(|e| LockRejection::ConnectionFailed(e.to_string()))?;
        let display = conn.display();
        let mut event_queue: EventQueue<LockClientState> = conn.new_event_queue();
        let qh = event_queue.handle();
        let mut state = LockClientState::default();

        let _registry = display.get_registry(&qh, ());
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| LockRejection::ConnectionFailed(e.to_string()))?;

        let Some(manager) = state.manager.take() else {
            return Ok(None);
        };

        let lock = manager.lock(&qh, ());
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| LockRejection::ConnectionFailed(e.to_string()))?;

        if state.finished && !state.locked {
            return Err(LockRejection::AlreadyLocked);
        }

        Ok(Some(Self {
            event_queue,
            state,
            lock: Some(lock),
            _manager: manager,
        }))
    }

    /// Libera el bloqueo en el compositor tras autenticar al usuario con éxito.
    pub fn unlock_and_destroy(&mut self) {
        if let Some(lock) = self.lock.take() {
            lock.unlock_and_destroy();
            let _ = self.event_queue.roundtrip(&mut self.state);
        }
    }
}

