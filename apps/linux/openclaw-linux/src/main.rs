mod config;
mod logging;
mod node;
mod settings;

use openclaw_core::ui::{NodeStatusView, UiEvent, UiEventSink};

struct LinuxUiEventSink;

impl UiEventSink for LinuxUiEventSink {
    fn on_event(&self, event: UiEvent) {
        log::info!("ui event: {event:?}");
    }
}

#[tokio::main]
async fn main() {
    logging::init();

    let settings = std::sync::Arc::new(std::sync::Mutex::new(settings::AppSettings::default()));
    let ui_app = openclaw_ui_gtk::create_app();
    let ui_control = ui_app.control();
    ui_app.set_event_sink(Some(std::sync::Arc::new(LinuxUiEventSink)));

    let runtime = std::sync::Arc::new(node::runtime::NodeRuntime::new(ui_control.clone()));
    let controller = node::controller::NodeController::new(settings, runtime);
    let status = controller.status();

    let status_for_ui = status.clone();
    let ui_for_status = ui_control.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let snapshot = status_for_ui
                .lock()
                .ok()
                .map(|status| NodeStatusView {
                    connected: status.connected,
                    last_error: status.last_error.clone(),
                })
                .unwrap_or_else(|| NodeStatusView {
                    connected: false,
                    last_error: Some("Status lock poisoned".to_string()),
                });
            ui_for_status.set_node_status(snapshot);
        }
    });

    let start_controller = controller.clone();
    tokio::spawn(async move {
        start_controller.start().await;
    });

    ui_control.set_node_status(NodeStatusView::default());
    ui_app.run();
    controller.stop().await;
}
