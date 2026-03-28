#[derive(Clone, Debug, Default)]
pub struct NodeStatusView {
    pub connected: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionMode {
    Local,
    Remote,
    #[default]
    Unconfigured,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OnboardingPage {
    #[default]
    Welcome,
    Connection,
    Wizard,
    Permissions,
    Ready,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnboardingWizardOptionView {
    pub value: serde_json::Value,
    pub label: String,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OnboardingWizardStepType {
    #[default]
    Note,
    Text,
    Confirm,
    Select,
    Multiselect,
    Progress,
    Action,
    Unsupported(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnboardingWizardStepView {
    pub id: String,
    pub step_type: OnboardingWizardStepType,
    pub title: Option<String>,
    pub message: Option<String>,
    pub options: Vec<OnboardingWizardOptionView>,
    pub initial_value: Option<serde_json::Value>,
    pub placeholder: Option<String>,
    pub sensitive: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnboardingWizardViewState {
    pub session_id: Option<String>,
    pub is_starting: bool,
    pub is_submitting: bool,
    pub is_complete: bool,
    pub error_message: Option<String>,
    pub step: Option<OnboardingWizardStepView>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnboardingViewState {
    pub visible: bool,
    pub page: OnboardingPage,
    pub mode: ConnectionMode,
    pub remote_url: String,
    pub remote_target: String,
    pub remote_identity: String,
    pub status_message: Option<String>,
    pub can_go_back: bool,
    pub can_go_next: bool,
    pub next_label: String,
    pub wizard: OnboardingWizardViewState,
}

#[derive(Clone, Debug)]
pub enum UiEvent {
    AppClosed,
    OnboardingBack,
    OnboardingNext,
    OnboardingSelectMode(ConnectionMode),
    OnboardingRemoteUrlChanged(String),
    OnboardingRemoteTargetChanged(String),
    OnboardingRemoteIdentityChanged(String),
    OnboardingWizardSubmit(Option<serde_json::Value>),
    OnboardingWizardRetry,
}

pub trait UiEventSink: Send + Sync {
    fn on_event(&self, event: UiEvent);
}

pub trait UiControl: Send + Sync {
    fn set_node_status(&self, status: NodeStatusView);
    fn set_onboarding_state(&self, state: OnboardingViewState);
    fn present_canvas(&self, url: Option<String>);
    fn hide_canvas(&self);
    fn navigate_canvas(&self, url: String);
}

pub trait UiApp {
    fn control(&self) -> std::sync::Arc<dyn UiControl>;
    fn set_event_sink(&self, sink: Option<std::sync::Arc<dyn UiEventSink>>);
    fn run(self: Box<Self>);
}
