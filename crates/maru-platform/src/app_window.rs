//! A visible application's client area, in physical desktop pixels.
use maru_core::{AppInfo, Rect};

#[derive(Debug, Clone, PartialEq)]
pub struct AppWindow {
    pub app: AppInfo,
    pub rect: Rect,
    pub identity: u64,
}
