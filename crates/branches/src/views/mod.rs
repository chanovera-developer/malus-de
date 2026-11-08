pub mod icon_view;
pub mod list_view;

use std::path::PathBuf;

pub type NavigateCallback = std::rc::Rc<dyn Fn(PathBuf)>;
pub type SelectionCallback = std::rc::Rc<dyn Fn(usize)>;
