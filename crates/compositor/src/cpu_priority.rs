use std::{
    collections::{HashMap, VecDeque},
    fs,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};
use zbus::{zvariant::Value, Connection, Proxy};

const BACKGROUND_CPU_WEIGHT: u64 = 20;
const FOCUSED_CPU_WEIGHT: u64 = 1_000;
const DEFAULT_CPU_WEIGHT: u64 = 100;

#[derive(Default)]
pub struct FocusCpuPriority {
    focused_pid: Option<i32>,
    requests: Option<Sender<Option<i32>>>,
}

#[derive(Clone, Copy)]
struct ProcessInfo {
    parent_pid: i32,
    start_time: u64,
}

impl FocusCpuPriority {
    pub fn set_focused_pid(&mut self, pid: Option<i32>) {
        if self.focused_pid == pid {
            return;
        }

        let result = self
            .requests
            .get_or_insert_with(start_priority_worker)
            .send(pid);
        match result {
            Ok(()) => self.focused_pid = pid,
            Err(error) => {
                self.requests = None;
                self.focused_pid = None;
                tracing::error!(
                    "No se pudo actualizar la prioridad de CPU de la aplicación enfocada: {error}"
                );
            }
        }
    }
}

fn start_priority_worker() -> Sender<Option<i32>> {
    let (sender, receiver) = mpsc::channel();
    if let Err(error) = thread::Builder::new()
        .name("malus-cpu-priority".to_string())
        .spawn(move || priority_worker(receiver))
    {
        tracing::error!("No se pudo iniciar el gestor de prioridad de CPU: {error}");
    }
    sender
}

fn priority_worker(receiver: Receiver<Option<i32>>) {
    let connection = match zbus::block_on(Connection::session()) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!("No se pudo conectar al gestor systemd del usuario: {error}");
            return;
        }
    };
    let manager = match zbus::block_on(Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )) {
        Ok(manager) => manager,
        Err(error) => {
            tracing::error!("No se pudo acceder al gestor systemd del usuario: {error}");
            return;
        }
    };

    let mut scopes = HashMap::<(i32, u64), String>::new();
    let mut focused_scope: Option<String> = None;
    while let Ok(mut request) = receiver.recv() {
        loop {
            match receiver.try_recv() {
                Ok(latest) => request = latest,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        if let Some(scope) = focused_scope.take() {
            set_scope_weight(&manager, &scope, BACKGROUND_CPU_WEIGHT);
        }

        let Some(pid) = request else {
            continue;
        };
        let Some(processes) = same_user_processes() else {
            tracing::warn!(
                "No se pudo leer la tabla de procesos para crear el scope de la aplicación."
            );
            continue;
        };
        let Some(root) = processes.get(&pid) else {
            continue;
        };
        let key = (pid, root.start_time);
        let unit_name = scopes
            .entry(key)
            .or_insert_with(|| format!("malus-app-{}-{}.scope", pid, root.start_time))
            .clone();
        let process_ids = descendant_pids(pid, &processes)
            .into_iter()
            .filter_map(|process_id| u32::try_from(process_id).ok())
            .collect::<Vec<_>>();
        if process_ids.is_empty() {
            continue;
        }

        match zbus::block_on(create_or_update_scope(
            &manager,
            &unit_name,
            process_ids,
            FOCUSED_CPU_WEIGHT,
        )) {
            Ok(()) => focused_scope = Some(unit_name),
            Err(error) => {
                tracing::warn!(
                    "No se pudo priorizar la aplicación enfocada en el scope {unit_name}: {error}"
                );
            }
        }
    }

    for scope in scopes.values() {
        set_scope_weight(&manager, scope, DEFAULT_CPU_WEIGHT);
    }
}

async fn create_or_update_scope(
    manager: &Proxy<'_>,
    unit_name: &str,
    process_ids: Vec<u32>,
    cpu_weight: u64,
) -> zbus::Result<()> {
    let properties = vec![
        ("PIDs", Value::from(process_ids)),
        ("CPUWeight", Value::from(cpu_weight)),
        ("Slice", Value::from("app.slice")),
        (
            "Description",
            Value::from("Malus DE focus-aware application scheduling"),
        ),
    ];
    let auxiliary = Vec::<(&str, Vec<(&str, Value<'_>)>)>::new();
    let start_result: zbus::Result<zbus::zvariant::OwnedObjectPath> = manager
        .call(
            "StartTransientUnit",
            &(unit_name, "fail", properties, auxiliary),
        )
        .await;
    match start_result {
        Ok(_) => Ok(()),
        Err(start_error) => set_scope_weight_async(manager, unit_name, cpu_weight)
            .await
            .map_err(|_| start_error),
    }
}

fn set_scope_weight(manager: &Proxy<'_>, unit_name: &str, cpu_weight: u64) {
    if let Err(error) = zbus::block_on(set_scope_weight_async(manager, unit_name, cpu_weight)) {
        tracing::warn!("No se pudo ajustar el peso de CPU del scope {unit_name}: {error}");
    }
}

async fn set_scope_weight_async(
    manager: &Proxy<'_>,
    unit_name: &str,
    cpu_weight: u64,
) -> zbus::Result<()> {
    let properties = vec![("CPUWeight", Value::from(cpu_weight))];
    manager
        .call("SetUnitProperties", &(unit_name, true, properties))
        .await
}

fn same_user_processes() -> Option<HashMap<i32, ProcessInfo>> {
    let uid = rustix::process::getuid().as_raw();
    let entries = fs::read_dir("/proc").ok()?;
    let mut processes = HashMap::new();

    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let path = entry.path();
        let Some(process_uid) = process_uid(&path) else {
            continue;
        };
        if process_uid != uid {
            continue;
        }
        if let Some((parent_pid, start_time)) = process_stat(&path.join("stat")) {
            processes.insert(
                pid,
                ProcessInfo {
                    parent_pid,
                    start_time,
                },
            );
        }
    }
    Some(processes)
}

fn process_uid(path: &std::path::Path) -> Option<u32> {
    let status = fs::read_to_string(path.join("status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn process_stat(path: &std::path::Path) -> Option<(i32, u64)> {
    let stat = fs::read_to_string(path).ok()?;
    let fields = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let parent_pid = fields.get(1)?.parse().ok()?;
    let start_time = fields.get(19)?.parse().ok()?;
    Some((parent_pid, start_time))
}

fn descendant_pids(root_pid: i32, processes: &HashMap<i32, ProcessInfo>) -> Vec<i32> {
    let mut children = HashMap::<i32, Vec<i32>>::new();
    for (pid, process) in processes {
        children.entry(process.parent_pid).or_default().push(*pid);
    }

    let mut descendants = Vec::new();
    let mut pending = VecDeque::from([root_pid]);
    while let Some(pid) = pending.pop_front() {
        if processes.contains_key(&pid) {
            descendants.push(pid);
            if let Some(child_pids) = children.get(&pid) {
                pending.extend(child_pids);
            }
        }
    }
    descendants
}

#[cfg(test)]
mod tests {
    use super::{descendant_pids, process_stat, ProcessInfo};
    use std::{collections::HashMap, process::Command, thread, time::Duration};

    #[test]
    fn parses_process_stat_with_parenthesized_command_name() {
        let stat = "42 (app (worker)) S 7 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 12345";
        let path = std::env::temp_dir().join(format!("malus-stat-test-{}", std::process::id()));
        std::fs::write(&path, stat).expect("write fixture");

        assert_eq!(process_stat(&path), Some((7, 12345)));
        std::fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn finds_only_descendants_of_the_focused_process() {
        let processes = HashMap::from([
            (
                10,
                ProcessInfo {
                    parent_pid: 1,
                    start_time: 1,
                },
            ),
            (
                11,
                ProcessInfo {
                    parent_pid: 10,
                    start_time: 1,
                },
            ),
            (
                12,
                ProcessInfo {
                    parent_pid: 11,
                    start_time: 1,
                },
            ),
            (
                20,
                ProcessInfo {
                    parent_pid: 1,
                    start_time: 1,
                },
            ),
        ]);

        assert_eq!(descendant_pids(10, &processes), vec![10, 11, 12]);
    }

    #[test]
    fn assigns_and_restores_systemd_cpu_weight_for_focused_application() {
        let manager_available = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "app.slice"])
            .status()
            .is_ok_and(|status| status.success());
        if !manager_available {
            eprintln!("Skipping systemd scope integration test: user app.slice is unavailable.");
            return;
        }

        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start test process");
        let pid = child.id() as i32;
        let mut priorities = super::FocusCpuPriority::default();
        priorities.set_focused_pid(Some(pid));
        let focused = wait_for_cpu_weight(pid, 1_000);
        priorities.set_focused_pid(None);
        let background = wait_for_cpu_weight(pid, 20);
        drop(priorities);
        let restored = wait_for_cpu_weight(pid, 100);

        child.kill().expect("stop test process");
        child.wait().expect("reap test process");

        assert!(focused, "focused app should receive CPUWeight=1000");
        assert!(background, "background app should receive CPUWeight=20");
        assert!(
            restored,
            "scope weight should return to the default on shutdown"
        );
    }

    fn wait_for_cpu_weight(pid: i32, expected: u64) -> bool {
        for _ in 0..100 {
            if cpu_weight(pid) == Some(expected) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn cpu_weight(pid: i32) -> Option<u64> {
        let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
        let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
        std::fs::read_to_string(
            std::path::Path::new("/sys/fs/cgroup")
                .join(path.trim_start_matches('/'))
                .join("cpu.weight"),
        )
        .ok()?
        .trim()
        .parse()
        .ok()
    }
}
