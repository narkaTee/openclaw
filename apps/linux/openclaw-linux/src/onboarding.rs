use crate::config::{self, GatewayConnectionUpdate, GatewayMode};
use crate::gateway_process::GatewayProcessManager;
use crate::gateway_rpc;
use crate::node::controller::NodeController;
use crate::settings::{AppSettings, CURRENT_ONBOARDING_VERSION};
use openclaw_core::ui::{
    ConnectionMode, OnboardingPage, OnboardingViewState, OnboardingWizardOptionView,
    OnboardingWizardStepType, OnboardingWizardStepView, UiControl, UiEvent,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct OnboardingCoordinator {
    settings: Arc<Mutex<AppSettings>>,
    ui: Arc<dyn UiControl>,
    node_controller: NodeController,
    gateway_manager: Arc<GatewayProcessManager>,
    state: OnboardingViewState,
    runtime_started: bool,
}

impl OnboardingCoordinator {
    pub fn new(
        settings: Arc<Mutex<AppSettings>>,
        ui: Arc<dyn UiControl>,
        node_controller: NodeController,
        gateway_manager: Arc<GatewayProcessManager>,
    ) -> Self {
        let state = OnboardingViewState {
            next_label: "Next".to_string(),
            ..OnboardingViewState::default()
        };
        Self {
            settings,
            ui,
            node_controller,
            gateway_manager,
            state,
            runtime_started: false,
        }
    }

    pub async fn bootstrap(&mut self) {
        let gateway = config::load_gateway_config();
        self.state.mode = match gateway.mode {
            GatewayMode::Local => ConnectionMode::Local,
            GatewayMode::Remote => ConnectionMode::Remote,
            GatewayMode::Unconfigured => ConnectionMode::Unconfigured,
        };
        self.state.remote_url = gateway.remote_url.unwrap_or_default();
        self.state.remote_target = gateway.remote_target.unwrap_or_default();
        self.state.remote_identity = gateway.remote_identity.unwrap_or_default();

        let settings = self
            .settings
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();

        let should_show =
            !settings.onboarding_seen || settings.onboarding_version < CURRENT_ONBOARDING_VERSION;
        self.state.visible = should_show;
        self.state.page = OnboardingPage::Welcome;
        self.refresh_connection_message();
        self.refresh_navigation();
        self.push_state();

        if !should_show {
            self.start_runtime_if_needed().await;
        }
    }

    pub async fn handle_event(&mut self, event: UiEvent) {
        if !self.state.visible {
            return;
        }

        match event {
            UiEvent::OnboardingSelectMode(mode) => {
                self.state.mode = mode;
                self.refresh_connection_message();
                self.refresh_navigation();
                self.push_state();
            }
            UiEvent::OnboardingRemoteUrlChanged(url) => {
                self.state.remote_url = url;
            }
            UiEvent::OnboardingRemoteTargetChanged(target) => {
                self.state.remote_target = target;
            }
            UiEvent::OnboardingRemoteIdentityChanged(identity) => {
                self.state.remote_identity = identity;
            }
            UiEvent::OnboardingBack => {
                self.handle_back().await;
            }
            UiEvent::OnboardingNext => {
                self.handle_next().await;
            }
            UiEvent::OnboardingWizardSubmit(value) => {
                self.handle_wizard_submit(value).await;
            }
            UiEvent::OnboardingWizardRetry => {
                self.start_wizard().await;
            }
            UiEvent::AppClosed => {}
        }
    }

    fn push_state(&self) {
        self.ui.set_onboarding_state(self.state.clone());
    }

    fn refresh_connection_message(&mut self) {
        self.state.status_message = match self.state.mode {
            ConnectionMode::Local => {
                Some("This machine will run or attach to a local OpenClaw Gateway.".to_string())
            }
            ConnectionMode::Remote => Some(
                "Use a reachable remote Gateway URL (for example wss://gateway.example.ts.net)."
                    .to_string(),
            ),
            ConnectionMode::Unconfigured => Some(
                "Skip Gateway setup for now. You can configure Local or Remote later.".to_string(),
            ),
        };
    }

    fn refresh_navigation(&mut self) {
        self.state.can_go_back = !matches!(self.state.page, OnboardingPage::Welcome);
        self.state.next_label = if matches!(self.state.page, OnboardingPage::Ready) {
            "Finish".to_string()
        } else {
            "Next".to_string()
        };
        self.state.can_go_next = match self.state.page {
            OnboardingPage::Welcome => true,
            OnboardingPage::Connection => match self.state.mode {
                ConnectionMode::Remote => true,
                ConnectionMode::Local | ConnectionMode::Unconfigured => true,
            },
            OnboardingPage::Wizard => self.state.wizard.is_complete,
            OnboardingPage::Permissions => true,
            OnboardingPage::Ready => true,
        };
    }

    async fn handle_back(&mut self) {
        match self.state.page {
            OnboardingPage::Welcome => {}
            OnboardingPage::Connection => {
                self.state.page = OnboardingPage::Welcome;
                self.state.status_message = None;
            }
            OnboardingPage::Wizard => {
                self.cancel_wizard_if_running().await;
                self.reset_wizard_state();
                self.state.page = OnboardingPage::Connection;
                self.refresh_connection_message();
            }
            OnboardingPage::Permissions => {
                self.state.page = if self.state.mode == ConnectionMode::Local {
                    OnboardingPage::Wizard
                } else {
                    OnboardingPage::Connection
                };
                if self.state.page == OnboardingPage::Connection {
                    self.refresh_connection_message();
                } else {
                    self.state.status_message = None;
                }
            }
            OnboardingPage::Ready => {
                self.state.page = OnboardingPage::Permissions;
                self.state.status_message = None;
            }
        }
        self.refresh_navigation();
        self.push_state();
    }

    async fn handle_next(&mut self) {
        if !self.state.can_go_next {
            return;
        }

        match self.state.page {
            OnboardingPage::Welcome => {
                self.state.page = OnboardingPage::Connection;
            }
            OnboardingPage::Connection => {
                if self.state.mode == ConnectionMode::Remote
                    && self.state.remote_url.trim().is_empty()
                {
                    self.state.status_message =
                        Some("Remote mode requires a Gateway URL.".to_string());
                    self.refresh_navigation();
                    self.push_state();
                    return;
                }
                if let Err(error) = self.persist_connection_mode() {
                    log::warn!("failed to persist onboarding gateway connection mode: {error}");
                    self.state.status_message = Some(error);
                    self.refresh_navigation();
                    self.push_state();
                    return;
                }
                if self.state.mode == ConnectionMode::Local {
                    self.state.page = OnboardingPage::Wizard;
                    self.state.status_message = None;
                    self.reset_wizard_state();
                    self.start_wizard().await;
                    return;
                }
                self.state.page = OnboardingPage::Permissions;
                self.state.status_message = None;
            }
            OnboardingPage::Wizard => {
                self.state.page = OnboardingPage::Permissions;
                self.state.status_message = None;
            }
            OnboardingPage::Permissions => {
                self.state.page = OnboardingPage::Ready;
                self.state.status_message = None;
            }
            OnboardingPage::Ready => {
                self.finish_onboarding().await;
                return;
            }
        }

        self.refresh_navigation();
        self.push_state();
    }

    fn persist_connection_mode(&self) -> Result<(), String> {
        let mode = match self.state.mode {
            ConnectionMode::Local => GatewayMode::Local,
            ConnectionMode::Remote => GatewayMode::Remote,
            ConnectionMode::Unconfigured => GatewayMode::Unconfigured,
        };
        let update = GatewayConnectionUpdate {
            mode,
            remote_url: if self.state.remote_url.trim().is_empty() {
                None
            } else {
                Some(self.state.remote_url.trim().to_string())
            },
            remote_target: if self.state.remote_target.trim().is_empty() {
                None
            } else {
                Some(self.state.remote_target.trim().to_string())
            },
            remote_identity: if self.state.remote_identity.trim().is_empty() {
                None
            } else {
                Some(self.state.remote_identity.trim().to_string())
            },
        };
        config::save_gateway_connection(&update)
    }

    async fn start_wizard(&mut self) {
        self.state.wizard.error_message = None;
        self.state.wizard.is_starting = true;
        self.state.wizard.is_submitting = false;
        self.state.wizard.is_complete = false;
        self.state.wizard.step = None;
        self.state.wizard.session_id = None;
        self.refresh_navigation();
        self.push_state();

        if let Err(error) = self.persist_connection_mode() {
            log::warn!("failed to persist onboarding connection mode before wizard start: {error}");
            self.state.wizard.is_starting = false;
            self.state.wizard.error_message = Some(error);
            self.refresh_navigation();
            self.push_state();
            return;
        }

        let gateway_config = config::load_gateway_config();
        if let Err(error) = self
            .gateway_manager
            .ensure_local_gateway(&gateway_config)
            .await
        {
            log::warn!("failed to ensure local gateway before wizard.start: {error}");
            self.state.wizard.is_starting = false;
            self.state.wizard.error_message = Some(error);
            self.refresh_navigation();
            self.push_state();
            return;
        }
        let gateway_config = config::load_gateway_config();

        let payload = gateway_rpc::call(
            &gateway_config,
            "wizard.start",
            Some(json!({"mode":"local"})),
        )
        .await;
        match payload {
            Ok(payload) => self.apply_wizard_start(payload),
            Err(error) => {
                log::warn!("wizard.start failed during onboarding: {error}");
                self.state.wizard.is_starting = false;
                self.state.wizard.error_message = Some(error);
            }
        }

        if self.state.wizard.is_complete {
            self.state.page = OnboardingPage::Permissions;
        }
        self.refresh_navigation();
        self.push_state();
    }

    async fn handle_wizard_submit(&mut self, value: Option<Value>) {
        let session_id = match self.state.wizard.session_id.clone() {
            Some(session_id) => session_id,
            None => return,
        };
        let step_id = match self.state.wizard.step.as_ref() {
            Some(step) => step.id.clone(),
            None => return,
        };
        let session_id_for_log = session_id.clone();
        let step_id_for_log = step_id.clone();

        self.state.wizard.is_submitting = true;
        self.state.wizard.error_message = None;
        self.refresh_navigation();
        self.push_state();

        let params = if let Some(value) = value {
            json!({
                "sessionId": session_id,
                "answer": {
                    "stepId": step_id,
                    "value": value
                }
            })
        } else {
            json!({
                "sessionId": session_id,
                "answer": {
                    "stepId": step_id
                }
            })
        };

        let gateway_config = config::load_gateway_config();
        let payload = gateway_rpc::call(&gateway_config, "wizard.next", Some(params)).await;
        match payload {
            Ok(payload) => self.apply_wizard_next(payload),
            Err(error) => {
                log::warn!(
                    "wizard.next failed during onboarding session_id={} step_id={} error={}",
                    session_id_for_log,
                    step_id_for_log,
                    error
                );
                self.state.wizard.is_submitting = false;
                if is_wizard_session_lost(&error) {
                    log::warn!(
                        "wizard session lost during onboarding; restarting session_id={}",
                        session_id_for_log
                    );
                    self.state.wizard.error_message =
                        Some("Wizard session lost. Restarting…".to_string());
                    self.push_state();
                    self.start_wizard().await;
                    return;
                }
                self.state.wizard.error_message = Some(error);
            }
        }

        if self.state.wizard.is_complete {
            self.state.page = OnboardingPage::Permissions;
        }
        self.refresh_navigation();
        self.push_state();
    }

    async fn cancel_wizard_if_running(&mut self) {
        let Some(session_id) = self.state.wizard.session_id.clone() else {
            return;
        };
        let gateway_config = config::load_gateway_config();
        if let Err(error) = gateway_rpc::call(
            &gateway_config,
            "wizard.cancel",
            Some(json!({ "sessionId": session_id })),
        )
        .await
        {
            log::warn!(
                "wizard.cancel failed during onboarding session_id={} error={}",
                session_id,
                error
            );
        }
    }

    fn reset_wizard_state(&mut self) {
        self.state.wizard = Default::default();
    }

    fn apply_wizard_start(&mut self, payload: Value) {
        let parsed = match serde_json::from_value::<WizardStartResult>(payload) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::warn!("failed to decode wizard.start response: {error}");
                self.state.wizard.is_starting = false;
                self.state.wizard.error_message =
                    Some(format!("Wizard start decode failed: {error}"));
                return;
            }
        };
        self.state.wizard.is_starting = false;
        self.state.wizard.is_submitting = false;
        self.state.wizard.session_id = Some(parsed.session_id.clone());
        self.state.wizard.error_message = parsed.error.clone();
        self.state.wizard.step = parsed.step.as_ref().map(map_wizard_step);
        self.state.wizard.is_complete =
            parsed.done || matches!(parsed.status.as_deref(), Some("done"));
    }

    fn apply_wizard_next(&mut self, payload: Value) {
        let parsed = match serde_json::from_value::<WizardNextResult>(payload) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::warn!("failed to decode wizard.next response: {error}");
                self.state.wizard.is_submitting = false;
                self.state.wizard.error_message =
                    Some(format!("Wizard step decode failed: {error}"));
                return;
            }
        };
        self.state.wizard.is_submitting = false;
        self.state.wizard.error_message = parsed.error.clone();
        self.state.wizard.step = parsed.step.as_ref().map(map_wizard_step);
        self.state.wizard.is_complete =
            parsed.done || matches!(parsed.status.as_deref(), Some("done"));
        if self.state.wizard.is_complete
            || matches!(parsed.status.as_deref(), Some("cancelled" | "error"))
        {
            self.state.wizard.session_id = None;
        }
    }

    async fn finish_onboarding(&mut self) {
        if let Ok(mut settings) = self.settings.lock() {
            settings.onboarding_seen = true;
            settings.onboarding_version = CURRENT_ONBOARDING_VERSION;
            if let Err(error) = settings.save() {
                log::warn!("failed to save linux app settings: {error}");
            }
        }

        self.state.visible = false;
        self.refresh_navigation();
        self.push_state();
        self.start_runtime_if_needed().await;
    }

    async fn start_runtime_if_needed(&mut self) {
        if self.runtime_started {
            return;
        }

        let gateway = config::load_gateway_config();
        match gateway.mode {
            GatewayMode::Unconfigured => {
                self.runtime_started = true;
            }
            GatewayMode::Local => {
                if let Err(error) = self.gateway_manager.ensure_local_gateway(&gateway).await {
                    log::warn!("failed to ensure local gateway before node startup: {error}");
                }
                self.node_controller.start().await;
                self.runtime_started = true;
            }
            GatewayMode::Remote => {
                self.node_controller.start().await;
                self.runtime_started = true;
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct WizardStepOption {
    value: Value,
    label: String,
    hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WizardStep {
    id: String,
    #[serde(rename = "type")]
    step_type: String,
    title: Option<String>,
    message: Option<String>,
    options: Option<Vec<WizardStepOption>>,
    #[serde(rename = "initialValue")]
    initial_value: Option<Value>,
    placeholder: Option<String>,
    sensitive: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WizardStartResult {
    #[serde(rename = "sessionId")]
    session_id: String,
    done: bool,
    step: Option<WizardStep>,
    status: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WizardNextResult {
    done: bool,
    step: Option<WizardStep>,
    status: Option<String>,
    error: Option<String>,
}

fn map_wizard_step(step: &WizardStep) -> OnboardingWizardStepView {
    let step_type = match step.step_type.as_str() {
        "note" => OnboardingWizardStepType::Note,
        "text" => OnboardingWizardStepType::Text,
        "confirm" => OnboardingWizardStepType::Confirm,
        "select" => OnboardingWizardStepType::Select,
        "multiselect" => OnboardingWizardStepType::Multiselect,
        "progress" => OnboardingWizardStepType::Progress,
        "action" => OnboardingWizardStepType::Action,
        other => OnboardingWizardStepType::Unsupported(other.to_string()),
    };
    OnboardingWizardStepView {
        id: step.id.clone(),
        step_type,
        title: step.title.clone(),
        message: step.message.clone(),
        options: step
            .options
            .as_ref()
            .map(|options| {
                options
                    .iter()
                    .map(|option| OnboardingWizardOptionView {
                        value: option.value.clone(),
                        label: option.label.clone(),
                        hint: option.hint.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        initial_value: step.initial_value.clone(),
        placeholder: step.placeholder.clone(),
        sensitive: step.sensitive.unwrap_or(false),
    }
}

fn is_wizard_session_lost(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("wizard not found") || lower.contains("wizard not running")
}
