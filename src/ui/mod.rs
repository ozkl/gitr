pub mod changes;
pub mod dialogs;
pub mod file_history;
pub mod diff_view;
pub mod history;
pub mod image_view;
pub mod sidebar;
pub mod theme;

use std::path::PathBuf;

use crate::app::Settings;
use dialogs::Dialog;

/// Shared UI state handed to views.
pub struct Ctx<'a> {
    pub dialog: &'a mut Option<Dialog>,
    pub settings: &'a mut Settings,
    pub focus_search: bool,
    pub focus_commit: bool,
    pub open_repo: Option<PathBuf>,
}
