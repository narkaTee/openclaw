mod config;
mod gateway_process;
mod gateway_rpc;
mod logging;
mod node;
mod onboarding;
mod settings;

use openclaw_core::ui::{NodeStatusView, UiEvent, UiEventSink};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    logging::init();

    let settings = Arc::new(Mutex::new(settings::AppSettings::load()));
    let ui_app = openclaw_ui_gtk::create_app();
    let ui_control = ui_app.control();
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiEvent>();
    ui_app.set_event_sink(Some(Arc::new(ChannelUiEventSink { sender: ui_tx })));

    let runtime = Arc::new(node::runtime::NodeRuntime::new(ui_control.clone()));
    let controller = node::controller::NodeController::new(settings.clone(), runtime);
    let gateway_manager = Arc::new(gateway_process::GatewayProcessManager::default());
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

    let onboarding_ui = ui_control.clone();
    let onboarding_controller = controller.clone();
    let onboarding_settings = settings.clone();
    let onboarding_gateway_manager = gateway_manager.clone();
    let onboarding_task = tokio::spawn(async move {
        let mut coordinator = onboarding::OnboardingCoordinator::new(
            onboarding_settings,
            onboarding_ui,
            onboarding_controller,
            onboarding_gateway_manager,
        );
        coordinator.bootstrap().await;
        while let Some(event) = ui_rx.recv().await {
            if matches!(event, UiEvent::AppClosed) {
                break;
            }
            coordinator.handle_event(event).await;
        }
    });

    ui_control.set_node_status(NodeStatusView::default());
    ui_app.run();
    let _ = onboarding_task.await;
    gateway_manager.stop().await;
    controller.stop().await;
}

struct ChannelUiEventSink {
    sender: mpsc::UnboundedSender<UiEvent>,
}

impl UiEventSink for ChannelUiEventSink {
    fn on_event(&self, event: UiEvent) {
        if self.sender.send(event.clone()).is_err() {
            log::warn!("failed to deliver ui event: {event:?}");
        }
    }
}
