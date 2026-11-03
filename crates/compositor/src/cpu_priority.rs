//! Gestión de prioridad de CPU de procesos en Malus DE.
//!
//! Todas las aplicaciones mantienen prioridad de CPU uniforme (sin estrangulamiento
//! ni penalización a procesos en segundo plano) para garantizar respuesta inmediata
//! y consistencia de rendimiento en el programador del kernel.

#[derive(Default)]
pub struct FocusCpuPriority {
    focused_pid: Option<i32>,
}

impl FocusCpuPriority {
    pub fn set_focused_pid(&mut self, pid: Option<i32>) {
        self.focused_pid = pid;
    }
}

#[cfg(test)]
mod tests {
    use super::FocusCpuPriority;

    #[test]
    fn focus_cpu_priority_maintains_uniform_priority() {
        let mut priorities = FocusCpuPriority::default();
        priorities.set_focused_pid(Some(42));
        assert_eq!(priorities.focused_pid, Some(42));
        priorities.set_focused_pid(None);
        assert_eq!(priorities.focused_pid, None);
    }
}
