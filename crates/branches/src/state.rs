use malus_de_common::config::{BranchesConfig, IconGridMetrics};
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Icons,
    List,
    Columns,
    Gallery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    DateModified,
    Size,
    Kind,
}

impl SortBy {
    pub fn as_str(self) -> &'static str {
        match self {
            SortBy::Name => "name",
            SortBy::DateModified => "date",
            SortBy::Size => "size",
            SortBy::Kind => "kind",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "date" => SortBy::DateModified,
            "size" => SortBy::Size,
            "kind" => SortBy::Kind,
            _ => SortBy::Name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Ascending => "asc",
            SortOrder::Descending => "desc",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "desc" => SortOrder::Descending,
            _ => SortOrder::Ascending,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AppState {
    pub current_path: PathBuf,
    pub history_back: Vec<PathBuf>,
    pub history_forward: Vec<PathBuf>,
    pub show_hidden: bool,
    pub view_mode: ViewMode,
    pub sort_by: SortBy,
    pub sort_order: SortOrder,
    pub split_view_active: bool,
    pub search_query: String,
    pub icon_size: i32,
    pub grid_spacing: i32,
}

impl AppState {
    pub fn new(initial_path: PathBuf) -> Self {
        let cfg = BranchesConfig::load();
        Self {
            current_path: initial_path,
            history_back: Vec::new(),
            history_forward: Vec::new(),
            show_hidden: cfg.show_hidden,
            view_mode: ViewMode::Icons,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            split_view_active: false,
            search_query: String::new(),
            icon_size: cfg.clamped_icon_size(),
            grid_spacing: cfg.clamped_grid_spacing(),
        }
    }

    #[allow(dead_code)]
    pub fn icon_grid_metrics(&self) -> IconGridMetrics {
        IconGridMetrics::from_icon_size(self.icon_size, self.grid_spacing)
    }

    pub fn navigate_to(&mut self, target: PathBuf) -> bool {
        if target == self.current_path {
            return false;
        }
        self.history_back.push(self.current_path.clone());
        self.history_forward.clear();
        self.current_path = target;
        true
    }

    pub fn can_go_back(&self) -> bool {
        !self.history_back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.history_forward.is_empty()
    }

    pub fn go_back(&mut self) -> Option<PathBuf> {
        if let Some(prev) = self.history_back.pop() {
            self.history_forward.push(self.current_path.clone());
            self.current_path = prev.clone();
            Some(prev)
        } else {
            None
        }
    }

    pub fn go_forward(&mut self) -> Option<PathBuf> {
        if let Some(next) = self.history_forward.pop() {
            self.history_back.push(self.current_path.clone());
            self.current_path = next.clone();
            Some(next)
        } else {
            None
        }
    }

    pub fn go_up(&mut self) -> Option<PathBuf> {
        if let Some(parent) = self.current_path.parent().map(|p| p.to_path_buf()) {
            if parent != self.current_path {
                self.navigate_to(parent.clone());
                return Some(parent);
            }
        }
        None
    }

    pub fn go_back_steps(&mut self, steps: usize) -> Option<PathBuf> {
        if steps == 0 || steps > self.history_back.len() {
            return None;
        }
        let mut last = None;
        for _ in 0..steps {
            last = self.go_back();
        }
        last
    }

    pub fn go_forward_steps(&mut self, steps: usize) -> Option<PathBuf> {
        if steps == 0 || steps > self.history_forward.len() {
            return None;
        }
        let mut last = None;
        for _ in 0..steps {
            last = self.go_forward();
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_steps_and_sort_conversions_work() {
        assert_eq!(SortBy::from_str("date"), SortBy::DateModified);
        assert_eq!(SortBy::from_str("size"), SortBy::Size);
        assert_eq!(SortBy::from_str("kind"), SortBy::Kind);
        assert_eq!(SortBy::from_str("other"), SortBy::Name);
        assert_eq!(SortOrder::from_str("desc"), SortOrder::Descending);
        assert_eq!(SortOrder::from_str("asc"), SortOrder::Ascending);

        let mut st = AppState::new(PathBuf::from("/a"));
        st.navigate_to(PathBuf::from("/b"));
        st.navigate_to(PathBuf::from("/c"));
        st.navigate_to(PathBuf::from("/d"));

        assert_eq!(st.go_back_steps(2), Some(PathBuf::from("/b")));
        assert_eq!(st.current_path, PathBuf::from("/b"));
        assert_eq!(st.history_forward, vec![PathBuf::from("/d"), PathBuf::from("/c")]);
        assert_eq!(st.go_forward_steps(2), Some(PathBuf::from("/d")));
        assert_eq!(st.current_path, PathBuf::from("/d"));
    }
}
