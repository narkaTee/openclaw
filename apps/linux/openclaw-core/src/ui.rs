#[derive(Clone, Debug, Default)]
pub struct NodeStatusView {
    pub connected: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum UiEvent {
    AppClosed,
}

pub trait UiEventSink: Send + Sync {
    fn on_event(&self, event: UiEvent);
}

pub trait UiControl: Send + Sync {
    fn set_node_status(&self, status: NodeStatusView);
    fn present_canvas(&self, url: Option<String>);
    fn hide_canvas(&self);
    fn navigate_canvas(&self, url: String);
}

pub trait UiApp {
    fn control(&self) -> std::sync::Arc<dyn UiControl>;
    fn set_event_sink(&self, sink: Option<std::sync::Arc<dyn UiEventSink>>);
    fn run(self: Box<Self>);
}
