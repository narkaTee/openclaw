use crate::config::{GatewayConfig, load_gateway_config};
use crate::node::caps::{current_caps, current_commands};
use crate::node::permissions::current_permissions;
use crate::node::runtime::NodeRuntime;
use crate::node::session::{ConnectOptions, GatewayNodeSession};
use crate::settings::AppSettings;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct NodeStatus {
    pub connected: bool,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct NodeController {
    settings: Arc<Mutex<AppSettings>>,
    status: Arc<Mutex<NodeStatus>>,
    session: Arc<GatewayNodeSession>,
    runtime: Arc<NodeRuntime>,
}

impl NodeController {
    pub fn new(settings: Arc<Mutex<AppSettings>>, runtime: Arc<NodeRuntime>) -> Self {
        Self {
            settings,
            status: Arc::new(Mutex::new(NodeStatus::default())),
            session: Arc::new(GatewayNodeSession::new()),
            runtime,
        }
    }

    pub fn status(&self) -> Arc<Mutex<NodeStatus>> {
        self.status.clone()
    }

    pub async fn start(&self) {
        let config = load_gateway_config();
        self.connect_once(config).await;
    }

    pub async fn stop(&self) {
        self.session.disconnect().await;
        if let Ok(mut status) = self.status.lock() {
            status.connected = false;
        }
    }

    async fn connect_once(&self, config: GatewayConfig) {
        let url = match config.url.as_deref() {
            Some(url) if !url.trim().is_empty() => url,
            _ => {
                if let Ok(mut status) = self.status.lock() {
                    status.connected = false;
                    status.last_error = Some("Gateway URL not configured".to_string());
                }
                return;
            }
        };

        let settings = self.settings.lock().map(|s| s.clone()).unwrap_or_default();
        let caps = current_caps(settings.canvas_enabled);
        let commands = current_commands(settings.canvas_enabled);
        let permissions = current_permissions();
        log::info!(
            "node connect caps={} commands={} permissions={}",
            caps.len(),
            commands.len(),
            permissions.len()
        );
        log::info!("node connect commands={}", commands.join(", "));

        let options = ConnectOptions {
            role: "node".to_string(),
            caps,
            commands,
            permissions,
            client_id: "node-host".to_string(),
            client_mode: "node".to_string(),
            client_display_name: "OpenClaw Linux".to_string(),
        };

        let runtime = self.runtime.clone();
        let status = self.status.clone();
        let result = self
            .session
            .connect(
                url,
                config.token.as_deref(),
                config.password.as_deref(),
                options,
                move |reason| {
                    if let Ok(mut status) = status.lock() {
                        status.connected = false;
                        status.last_error = Some(reason);
                    }
                },
                move |req| runtime.handle_invoke(req),
            )
            .await;

        if let Ok(mut status) = self.status.lock() {
            match result {
                Ok(()) => {
                    status.connected = true;
                    status.last_error = None;
                }
                Err(err) => {
                    status.connected = false;
                    status.last_error = Some(err);
                }
            }
        }
    }
}
