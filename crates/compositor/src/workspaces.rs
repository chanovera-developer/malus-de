//! Gestor de Espacios de Trabajo Virtuales (*Workspaces*) para Malus DE.
//!
//! Mantiene una lista ordenada de escritorios virtuales (`Escritorio 1`, `Escritorio 2`, ...),
//! conservando en memoria las ventanas de los espacios inactivos con sus ubicaciones
//! para conmutación instantánea o reubicación vía Drag & Drop desde Exposé.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Point},
    wayland::seat::WaylandFocus,
};

pub const MAX_WORKSPACES: usize = 8;

#[derive(Debug, Clone)]
pub struct WorkspaceWindow {
    pub window: Window,
    pub location: Point<i32, Logical>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceEntry {
    pub id: usize,
    pub name: String,
    pub hidden_windows: Vec<WorkspaceWindow>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    pub active_index: usize,
    pub workspaces: Vec<WorkspaceEntry>,
    next_id: usize,
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self {
            active_index: 0,
            workspaces: vec![
                WorkspaceEntry {
                    id: 1,
                    name: "Escritorio 1".to_string(),
                    hidden_windows: Vec::new(),
                },
                WorkspaceEntry {
                    id: 2,
                    name: "Escritorio 2".to_string(),
                    hidden_windows: Vec::new(),
                },
            ],
            next_id: 3,
        }
    }
}

impl WorkspaceManager {
    pub fn count(&self) -> usize {
        self.workspaces.len()
    }

    pub fn add_workspace(&mut self) -> Option<usize> {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.workspaces.push(WorkspaceEntry {
            id,
            name: format!("Escritorio {}", self.workspaces.len() + 1),
            hidden_windows: Vec::new(),
        });
        Some(self.workspaces.len() - 1)
    }

    pub fn remove_workspace(&mut self, index: usize) -> Option<Vec<WorkspaceWindow>> {
        if self.workspaces.len() <= 1 || index >= self.workspaces.len() || index == self.active_index {
            return None;
        }
        let removed = self.workspaces.remove(index);
        if index < self.active_index {
            self.active_index -= 1;
        }
        for (i, ws) in self.workspaces.iter_mut().enumerate() {
            ws.name = format!("Escritorio {}", i + 1);
        }
        Some(removed.hidden_windows)
    }

    pub fn retain_alive(&mut self) {
        for ws in &mut self.workspaces {
            ws.hidden_windows.retain(|entry| entry.window.alive());
        }
    }

    pub fn remove_surface(&mut self, surf: &WlSurface) -> Option<Window> {
        for ws in &mut self.workspaces {
            if let Some(pos) = ws
                .hidden_windows
                .iter()
                .position(|entry| entry.window.wl_surface().as_deref() == Some(surf))
            {
                return Some(ws.hidden_windows.remove(pos).window);
            }
        }
        None
    }

    pub fn window_for_surface(&self, surf: &WlSurface) -> Option<Window> {
        for ws in &self.workspaces {
            for entry in &ws.hidden_windows {
                if entry.window.wl_surface().as_deref() == Some(surf) {
                    return Some(entry.window.clone());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_manager_starts_with_two_spaces_and_respects_max() {
        let mut mgr = WorkspaceManager::default();
        assert_eq!(mgr.count(), 2);
        assert_eq!(mgr.active_index, 0);
        assert_eq!(mgr.workspaces[0].name, "Escritorio 1");
        assert_eq!(mgr.workspaces[1].name, "Escritorio 2");

        while mgr.count() < MAX_WORKSPACES {
            assert!(mgr.add_workspace().is_some());
        }
        assert!(mgr.add_workspace().is_none());
        assert_eq!(mgr.count(), MAX_WORKSPACES);
    }

    #[test]
    fn workspace_manager_remove_adjusts_active_index_and_prevents_empty() {
        let mut mgr = WorkspaceManager::default();
        let third = mgr.add_workspace().unwrap();
        assert_eq!(third, 2);
        mgr.active_index = 2;

        // No permite borrar el espacio activo
        assert!(mgr.remove_workspace(2).is_none());
        // Al borrar el índice 0, el activo baja de 2 a 1
        assert!(mgr.remove_workspace(0).is_some());
        assert_eq!(mgr.active_index, 1);
        assert_eq!(mgr.count(), 2);

        assert!(mgr.remove_workspace(0).is_some());
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.active_index, 0);
        // No permite quedarse sin espacios
        assert!(mgr.remove_workspace(0).is_none());
    }
}

