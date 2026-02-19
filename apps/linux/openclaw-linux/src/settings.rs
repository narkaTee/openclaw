#[derive(Clone, Debug)]
pub struct AppSettings {
    pub paused: bool,
    pub canvas_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            paused: false,
            canvas_enabled: true,
        }
    }
}
