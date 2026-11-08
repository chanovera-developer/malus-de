use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Icons,
    List,
    Columns,
    Gallery,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    DateModified,
    Size,
    Kind,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
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
}

impl AppState {
    pub fn new(initial_path: PathBuf) -> Self {
        Self {
            current_path: initial_path,
            history_back: Vec::new(),
            history_forward: Vec::new(),
            show_hidden: false,
            view_mode: ViewMode::Icons,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Ascending,
            split_view_active: false,
            search_query: String::new(),
        }
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
}
