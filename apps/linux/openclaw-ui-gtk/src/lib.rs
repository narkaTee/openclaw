use gtk4::prelude::*;
use openclaw_core::ui::{
    ConnectionMode, NodeStatusView, OnboardingPage, OnboardingViewState, OnboardingWizardStepType,
    OnboardingWizardStepView, UiApp, UiControl, UiEvent, UiEventSink,
};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use webkit6::prelude::*;

const DEFAULT_CANVAS_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OpenClaw Canvas</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      font-family: sans-serif;
      color: #f5f5f5;
      background: radial-gradient(circle at 20% 20%, #1f2937 0%, #111827 55%, #030712 100%);
      display: grid;
      place-items: center;
    }
    main {
      max-width: 34rem;
      text-align: center;
      padding: 1.5rem;
    }
    h1 { margin: 0 0 0.5rem 0; font-size: 1.4rem; }
    p { margin: 0; opacity: 0.85; line-height: 1.45; }
  </style>
</head>
<body>
  <main>
    <h1>OpenClaw Canvas Ready</h1>
    <p>Waiting for a canvas URL from the gateway node command.</p>
  </main>
</body>
</html>
"#;

#[derive(Clone, Debug)]
enum CanvasCommand {
    Present { url: Option<String> },
    Hide,
    Navigate { url: String },
}

#[derive(Clone, Debug)]
enum UiCommand {
    SetNodeStatus(NodeStatusView),
    SetOnboardingState(OnboardingViewState),
    Canvas(CanvasCommand),
}

#[derive(Clone)]
struct GtkUiControl {
    sender: Sender<UiCommand>,
}

impl UiControl for GtkUiControl {
    fn set_node_status(&self, status: NodeStatusView) {
        let _ = self.sender.send(UiCommand::SetNodeStatus(status));
    }

    fn set_onboarding_state(&self, state: OnboardingViewState) {
        let _ = self.sender.send(UiCommand::SetOnboardingState(state));
    }

    fn present_canvas(&self, url: Option<String>) {
        let _ = self
            .sender
            .send(UiCommand::Canvas(CanvasCommand::Present { url }));
    }

    fn hide_canvas(&self) {
        let _ = self.sender.send(UiCommand::Canvas(CanvasCommand::Hide));
    }

    fn navigate_canvas(&self, url: String) {
        let _ = self
            .sender
            .send(UiCommand::Canvas(CanvasCommand::Navigate { url }));
    }
}

pub struct GtkUiApp {
    control: Arc<GtkUiControl>,
    command_receiver: Mutex<Option<Receiver<UiCommand>>>,
    event_sink: Arc<Mutex<Option<Arc<dyn UiEventSink>>>>,
}

impl GtkUiApp {
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            control: Arc::new(GtkUiControl { sender }),
            command_receiver: Mutex::new(Some(receiver)),
            event_sink: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for GtkUiApp {
    fn default() -> Self {
        Self::new()
    }
}

impl UiApp for GtkUiApp {
    fn control(&self) -> Arc<dyn UiControl> {
        self.control.clone()
    }

    fn set_event_sink(&self, sink: Option<Arc<dyn UiEventSink>>) {
        if let Ok(mut guard) = self.event_sink.lock() {
            *guard = sink;
        }
    }

    fn run(self: Box<Self>) {
        let app = *self;
        let command_receiver = match app.command_receiver.into_inner() {
            Ok(receiver) => receiver,
            Err(poison) => poison.into_inner(),
        }
        .expect("gtk app command receiver missing");
        run_gtk_app(command_receiver, app.event_sink);
    }
}

pub fn create_app() -> Box<dyn UiApp> {
    Box::new(GtkUiApp::new())
}

fn run_gtk_app(
    command_receiver: Receiver<UiCommand>,
    event_sink: Arc<Mutex<Option<Arc<dyn UiEventSink>>>>,
) {
    let app = gtk4::Application::new(
        Some("ai.openclaw.linux"),
        gtk4::gio::ApplicationFlags::empty(),
    );
    let command_receiver = RefCell::new(Some(command_receiver));
    let shutdown_sink = event_sink.clone();
    let activate_sink = event_sink.clone();

    app.connect_shutdown(move |_| {
        emit_event(&shutdown_sink, UiEvent::AppClosed);
    });

    app.connect_activate(move |app| {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("OpenClaw"));
        window.set_default_size(920, 700);

        let root_stack = gtk4::Stack::new();
        root_stack.set_hexpand(true);
        root_stack.set_vexpand(true);

        let runtime_container = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        runtime_container.set_margin_top(16);
        runtime_container.set_margin_bottom(16);
        runtime_container.set_margin_start(16);
        runtime_container.set_margin_end(16);

        let status_label = gtk4::Label::new(None);
        status_label.set_xalign(0.0);
        let error_label = gtk4::Label::new(None);
        error_label.set_xalign(0.0);
        error_label.set_wrap(true);
        runtime_container.append(&status_label);
        runtime_container.append(&error_label);

        let onboarding_widgets = OnboardingWidgets::new(activate_sink.clone());
        root_stack.add_named(&onboarding_widgets.root, Some("onboarding"));
        root_stack.add_named(&runtime_container, Some("runtime"));
        root_stack.set_visible_child_name("runtime");
        window.set_child(Some(&root_stack));

        if let Some(receiver) = command_receiver.borrow_mut().take() {
            let app = app.clone();
            let status_label = status_label.clone();
            let error_label = error_label.clone();
            let root_stack = root_stack.clone();
            let onboarding_widgets = onboarding_widgets.clone();
            let mut canvas_widgets: Option<CanvasWidgets> = None;
            update_status_labels(&status_label, &error_label, &NodeStatusView::default());

            gtk4::glib::timeout_add_local(Duration::from_millis(100), move || {
                loop {
                    let command = match receiver.try_recv() {
                        Ok(command) => command,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
                    };

                    match command {
                        UiCommand::SetNodeStatus(next) => {
                            update_status_labels(&status_label, &error_label, &next);
                        }
                        UiCommand::SetOnboardingState(next) => {
                            onboarding_widgets.update(&next);
                            root_stack.set_visible_child_name(if next.visible {
                                "onboarding"
                            } else {
                                "runtime"
                            });
                        }
                        UiCommand::Canvas(canvas_command) => {
                            for action in plan_canvas_actions(canvas_command) {
                                apply_canvas_action(&app, &mut canvas_widgets, action);
                            }
                        }
                    }
                }
                gtk4::glib::ControlFlow::Continue
            });
        }

        window.present();
    });

    app.run();
}

#[derive(Clone)]
struct OnboardingWidgets {
    root: gtk4::Box,
    title_label: gtk4::Label,
    subtitle_label: gtk4::Label,
    page_body: gtk4::Box,
    back_button: gtk4::Button,
    next_button: gtk4::Button,
    step_input: RcRef<WizardInputState>,
    step_model: RcRef<Option<OnboardingWizardStepView>>,
    event_sink: Arc<Mutex<Option<Arc<dyn UiEventSink>>>>,
}

impl OnboardingWidgets {
    fn new(event_sink: Arc<Mutex<Option<Arc<dyn UiEventSink>>>>) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        root.set_margin_start(18);
        root.set_margin_end(18);

        let title_label = gtk4::Label::new(None);
        title_label.set_xalign(0.0);
        title_label.add_css_class("title-2");
        let subtitle_label = gtk4::Label::new(None);
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_wrap(true);
        subtitle_label.add_css_class("dim-label");

        let page_body = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        page_body.set_hexpand(true);
        page_body.set_vexpand(true);

        let nav = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let back_button = gtk4::Button::with_label("Back");
        let next_button = gtk4::Button::with_label("Next");
        nav.append(&back_button);
        nav.append(&next_button);

        root.append(&title_label);
        root.append(&subtitle_label);
        root.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        root.append(&page_body);
        root.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        root.append(&nav);

        let widgets = Self {
            root,
            title_label,
            subtitle_label,
            page_body,
            back_button,
            next_button,
            step_input: RcRef::default(),
            step_model: RcRef::default(),
            event_sink,
        };

        let back_sink = widgets.event_sink.clone();
        widgets.back_button.connect_clicked(move |_| {
            emit_event(&back_sink, UiEvent::OnboardingBack);
        });

        let next_sink = widgets.event_sink.clone();
        widgets.next_button.connect_clicked(move |_| {
            emit_event(&next_sink, UiEvent::OnboardingNext);
        });

        widgets
    }

    fn update(&self, state: &OnboardingViewState) {
        self.back_button.set_sensitive(state.can_go_back);
        self.next_button.set_sensitive(state.can_go_next);
        self.next_button.set_label(&state.next_label);

        let (title, subtitle) = page_heading(state.page.clone());
        self.title_label.set_text(title);
        self.subtitle_label.set_text(subtitle);

        clear_box(&self.page_body);
        *self.step_input.borrow_mut() = WizardInputState::None;
        *self.step_model.borrow_mut() = None;

        match state.page {
            OnboardingPage::Welcome => self.render_welcome(),
            OnboardingPage::Connection => self.render_connection(state),
            OnboardingPage::Wizard => self.render_wizard(state),
            OnboardingPage::Permissions => self.render_permissions(),
            OnboardingPage::Ready => self.render_ready(state),
        }
    }

    fn render_welcome(&self) {
        let message = gtk4::Label::new(Some(
            "OpenClaw connects AI agents to your chat channels.\n\nSecurity notice: the connected model can run commands and manipulate files based on the permissions and tools you enable.",
        ));
        message.set_wrap(true);
        message.set_xalign(0.0);
        self.page_body.append(&message);
    }

    fn render_connection(&self, state: &OnboardingViewState) {
        let local = gtk4::CheckButton::with_label("This machine (Local Gateway)");
        let remote = gtk4::CheckButton::with_label("Remote Gateway");
        remote.set_group(Some(&local));
        let later = gtk4::CheckButton::with_label("Configure later");
        later.set_group(Some(&local));

        local.set_active(state.mode == ConnectionMode::Local);
        remote.set_active(state.mode == ConnectionMode::Remote);
        later.set_active(state.mode == ConnectionMode::Unconfigured);

        let local_sink = self.event_sink.clone();
        local.connect_toggled(move |button| {
            if button.is_active() {
                emit_event(
                    &local_sink,
                    UiEvent::OnboardingSelectMode(ConnectionMode::Local),
                );
            }
        });
        let remote_sink = self.event_sink.clone();
        remote.connect_toggled(move |button| {
            if button.is_active() {
                emit_event(
                    &remote_sink,
                    UiEvent::OnboardingSelectMode(ConnectionMode::Remote),
                );
            }
        });
        let later_sink = self.event_sink.clone();
        later.connect_toggled(move |button| {
            if button.is_active() {
                emit_event(
                    &later_sink,
                    UiEvent::OnboardingSelectMode(ConnectionMode::Unconfigured),
                );
            }
        });

        self.page_body.append(&local);
        self.page_body.append(&remote);
        self.page_body.append(&later);

        if state.mode == ConnectionMode::Remote {
            let grid = gtk4::Grid::new();
            grid.set_column_spacing(8);
            grid.set_row_spacing(8);

            let url_label = gtk4::Label::new(Some("Gateway URL"));
            url_label.set_xalign(0.0);
            let url_entry = gtk4::Entry::new();
            url_entry.set_placeholder_text(Some("wss://gateway.example.ts.net"));
            url_entry.set_text(&state.remote_url);
            let url_sink = self.event_sink.clone();
            url_entry.connect_changed(move |entry| {
                emit_event(
                    &url_sink,
                    UiEvent::OnboardingRemoteUrlChanged(entry.text().to_string()),
                );
            });

            let target_label = gtk4::Label::new(Some("SSH target (optional)"));
            target_label.set_xalign(0.0);
            let target_entry = gtk4::Entry::new();
            target_entry.set_placeholder_text(Some("user@host[:port]"));
            target_entry.set_text(&state.remote_target);
            let target_sink = self.event_sink.clone();
            target_entry.connect_changed(move |entry| {
                emit_event(
                    &target_sink,
                    UiEvent::OnboardingRemoteTargetChanged(entry.text().to_string()),
                );
            });

            let identity_label = gtk4::Label::new(Some("SSH identity (optional)"));
            identity_label.set_xalign(0.0);
            let identity_entry = gtk4::Entry::new();
            identity_entry.set_placeholder_text(Some("~/.ssh/id_ed25519"));
            identity_entry.set_text(&state.remote_identity);
            let identity_sink = self.event_sink.clone();
            identity_entry.connect_changed(move |entry| {
                emit_event(
                    &identity_sink,
                    UiEvent::OnboardingRemoteIdentityChanged(entry.text().to_string()),
                );
            });

            grid.attach(&url_label, 0, 0, 1, 1);
            grid.attach(&url_entry, 1, 0, 1, 1);
            grid.attach(&target_label, 0, 1, 1, 1);
            grid.attach(&target_entry, 1, 1, 1, 1);
            grid.attach(&identity_label, 0, 2, 1, 1);
            grid.attach(&identity_entry, 1, 2, 1, 1);
            self.page_body.append(&grid);
        }

        if let Some(status) = state.status_message.as_deref() {
            let label = gtk4::Label::new(Some(status));
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.add_css_class("dim-label");
            self.page_body.append(&label);
        }
    }

    fn render_wizard(&self, state: &OnboardingViewState) {
        if let Some(error) = state.wizard.error_message.as_deref() {
            let error_label = gtk4::Label::new(Some(error));
            error_label.set_wrap(true);
            error_label.set_xalign(0.0);
            error_label.add_css_class("error");
            self.page_body.append(&error_label);
            let retry = gtk4::Button::with_label("Retry wizard");
            let retry_sink = self.event_sink.clone();
            retry.connect_clicked(move |_| {
                emit_event(&retry_sink, UiEvent::OnboardingWizardRetry);
            });
            self.page_body.append(&retry);
            return;
        }

        if state.wizard.is_starting {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            row.append(&gtk4::Spinner::new());
            row.append(&gtk4::Label::new(Some("Starting setup wizard…")));
            self.page_body.append(&row);
            return;
        }

        if state.wizard.is_complete {
            let complete = gtk4::Label::new(Some("Wizard complete. Continue to the next step."));
            complete.set_xalign(0.0);
            self.page_body.append(&complete);
            return;
        }

        if let Some(step) = state.wizard.step.as_ref() {
            *self.step_model.borrow_mut() = Some(step.clone());

            if let Some(title) = step.title.as_deref() {
                let title_label = gtk4::Label::new(Some(title));
                title_label.set_xalign(0.0);
                title_label.add_css_class("title-4");
                self.page_body.append(&title_label);
            }
            if let Some(message) = step.message.as_deref() {
                let message_label = gtk4::Label::new(Some(message));
                message_label.set_wrap(true);
                message_label.set_xalign(0.0);
                self.page_body.append(&message_label);
            }

            self.render_wizard_step_input(step);

            let submit =
                gtk4::Button::with_label(if step.step_type == OnboardingWizardStepType::Action {
                    "Run"
                } else {
                    "Continue"
                });
            submit.set_sensitive(!state.wizard.is_submitting);
            let submit_sink = self.event_sink.clone();
            let step_input = self.step_input.clone();
            let step_model = self.step_model.clone();
            submit.connect_clicked(move |_| {
                let step = step_model.borrow();
                let input = step_input.borrow();
                let value = wizard_submit_value(step.as_ref(), &input);
                emit_event(&submit_sink, UiEvent::OnboardingWizardSubmit(value));
            });
            self.page_body.append(&submit);
            return;
        }

        let waiting = gtk4::Label::new(Some("Waiting for wizard step…"));
        waiting.set_xalign(0.0);
        self.page_body.append(&waiting);
    }

    fn render_wizard_step_input(&self, step: &OnboardingWizardStepView) {
        match step.step_type {
            OnboardingWizardStepType::Text => {
                let entry = gtk4::Entry::new();
                if let Some(placeholder) = step.placeholder.as_deref() {
                    entry.set_placeholder_text(Some(placeholder));
                }
                if let Some(initial) = step.initial_value.as_ref().and_then(|value| value.as_str())
                {
                    entry.set_text(initial);
                }
                entry.set_visibility(!step.sensitive);
                self.page_body.append(&entry);
                *self.step_input.borrow_mut() = WizardInputState::Text(entry);
            }
            OnboardingWizardStepType::Confirm => {
                let check = gtk4::CheckButton::with_label("Confirmed");
                let initial = step
                    .initial_value
                    .as_ref()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                check.set_active(initial);
                self.page_body.append(&check);
                *self.step_input.borrow_mut() = WizardInputState::Confirm(check);
            }
            OnboardingWizardStepType::Select => {
                let combo = gtk4::ComboBoxText::new();
                let mut values = Vec::new();
                let mut initial = None;
                for (index, option) in step.options.iter().enumerate() {
                    combo.append_text(&option.label);
                    values.push(option.value.clone());
                    if initial.is_none() && step.initial_value.as_ref() == Some(&option.value) {
                        initial = Some(index);
                    }
                }
                combo.set_active(
                    initial
                        .or(if values.is_empty() { None } else { Some(0) })
                        .map(|idx| idx as u32),
                );
                self.page_body.append(&combo);
                *self.step_input.borrow_mut() = WizardInputState::Select { combo, values };
            }
            OnboardingWizardStepType::Multiselect => {
                let mut rows = Vec::new();
                for option in &step.options {
                    let row = gtk4::CheckButton::with_label(&option.label);
                    let selected = step
                        .initial_value
                        .as_ref()
                        .and_then(|value| value.as_array())
                        .is_some_and(|values| values.iter().any(|value| value == &option.value));
                    row.set_active(selected);
                    self.page_body.append(&row);
                    rows.push((row, option.value.clone()));
                }
                *self.step_input.borrow_mut() = WizardInputState::Multiselect(rows);
            }
            OnboardingWizardStepType::Progress => {
                self.page_body.append(&gtk4::Spinner::new());
                *self.step_input.borrow_mut() = WizardInputState::None;
            }
            OnboardingWizardStepType::Note
            | OnboardingWizardStepType::Action
            | OnboardingWizardStepType::Unsupported(_) => {
                *self.step_input.borrow_mut() = WizardInputState::None;
            }
        }
    }

    fn render_permissions(&self) {
        let message = gtk4::Label::new(Some(
            "Linux permission checks vary by desktop environment.\nOpenClaw will request capabilities when needed (notifications, screen capture, microphone, and related portal permissions).",
        ));
        message.set_wrap(true);
        message.set_xalign(0.0);
        self.page_body.append(&message);
    }

    fn render_ready(&self, state: &OnboardingViewState) {
        let summary = match state.mode {
            ConnectionMode::Local => {
                "Local mode selected. The Linux app will use the local Gateway."
            }
            ConnectionMode::Remote => {
                "Remote mode selected. The Linux app will connect to the configured remote Gateway."
            }
            ConnectionMode::Unconfigured => {
                "Gateway setup skipped. You can configure Local or Remote mode later."
            }
        };
        let summary_label = gtk4::Label::new(Some(summary));
        summary_label.set_wrap(true);
        summary_label.set_xalign(0.0);
        self.page_body.append(&summary_label);
    }
}

fn wizard_submit_value(
    step: Option<&OnboardingWizardStepView>,
    input: &WizardInputState,
) -> Option<Value> {
    let step_type = step.map(|step| step.step_type.clone());
    match step_type.unwrap_or_default() {
        OnboardingWizardStepType::Text => match input {
            WizardInputState::Text(entry) => Some(Value::String(entry.text().to_string())),
            _ => None,
        },
        OnboardingWizardStepType::Confirm => match input {
            WizardInputState::Confirm(check) => Some(Value::Bool(check.is_active())),
            _ => None,
        },
        OnboardingWizardStepType::Select => match input {
            WizardInputState::Select { combo, values } => combo
                .active()
                .and_then(|idx| values.get(idx as usize).cloned()),
            _ => None,
        },
        OnboardingWizardStepType::Multiselect => match input {
            WizardInputState::Multiselect(rows) => Some(Value::Array(
                rows.iter()
                    .filter(|(toggle, _)| toggle.is_active())
                    .map(|(_, value)| value.clone())
                    .collect(),
            )),
            _ => Some(Value::Array(Vec::new())),
        },
        OnboardingWizardStepType::Action => Some(Value::Bool(true)),
        OnboardingWizardStepType::Note
        | OnboardingWizardStepType::Progress
        | OnboardingWizardStepType::Unsupported(_) => None,
    }
}

fn page_heading(page: OnboardingPage) -> (&'static str, &'static str) {
    match page {
        OnboardingPage::Welcome => ("Welcome to OpenClaw", "Set up your Linux app and Gateway."),
        OnboardingPage::Connection => (
            "Choose Gateway mode",
            "Pick Local, Remote, or configure later.",
        ),
        OnboardingPage::Wizard => ("Setup Wizard", "Guided setup from the Gateway."),
        OnboardingPage::Permissions => (
            "Permissions",
            "Review desktop permissions required by OpenClaw.",
        ),
        OnboardingPage::Ready => ("All set", "Finish onboarding and start using OpenClaw."),
    }
}

type RcRef<T> = Rc<RefCell<T>>;

#[derive(Clone)]
enum WizardInputState {
    None,
    Text(gtk4::Entry),
    Confirm(gtk4::CheckButton),
    Select {
        combo: gtk4::ComboBoxText,
        values: Vec<Value>,
    },
    Multiselect(Vec<(gtk4::CheckButton, Value)>),
}

impl Default for WizardInputState {
    fn default() -> Self {
        Self::None
    }
}

fn emit_event(event_sink: &Arc<Mutex<Option<Arc<dyn UiEventSink>>>>, event: UiEvent) {
    if let Ok(guard) = event_sink.lock() {
        if let Some(sink) = guard.as_ref() {
            sink.on_event(event);
        }
    }
}

fn clear_box(container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn update_status_labels(
    status_label: &gtk4::Label,
    error_label: &gtk4::Label,
    status: &NodeStatusView,
) {
    let status_text = if status.connected {
        "Node mode: Connected"
    } else {
        "Node mode: Disconnected"
    };
    status_label.set_text(status_text);

    let error_text = status.last_error.as_deref().unwrap_or("");
    if error_text.is_empty() {
        error_label.set_text("");
    } else {
        error_label.set_text(&format!("Last error: {error_text}"));
    }
}

struct CanvasWidgets {
    window: gtk4::ApplicationWindow,
    webview: webkit6::WebView,
}

fn ensure_canvas_window<'a>(
    app: &gtk4::Application,
    canvas_slot: &'a mut Option<CanvasWidgets>,
) -> &'a CanvasWidgets {
    if canvas_slot.is_none() {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("OpenClaw Canvas"));
        window.set_default_size(1024, 700);
        window.set_hide_on_close(true);

        let webview = webkit6::WebView::new();
        webview.set_hexpand(true);
        webview.set_vexpand(true);
        load_canvas_target(&webview, None);

        window.set_child(Some(&webview));
        *canvas_slot = Some(CanvasWidgets { window, webview });
    }

    canvas_slot
        .as_ref()
        .expect("canvas window should be initialized")
}

fn load_canvas_target(webview: &webkit6::WebView, url: Option<&str>) {
    if let Some(target) = canonical_canvas_target(url) {
        webview.load_uri(&target);
    } else {
        webview.load_html(DEFAULT_CANVAS_HTML, None);
    }
}

fn canonical_canvas_target(raw: Option<&str>) -> Option<String> {
    let trimmed = raw.map(|value| value.trim()).unwrap_or_default();
    if trimmed.is_empty() || trimmed == "/" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CanvasUiAction {
    Load(Option<String>),
    Show,
    Hide,
}

fn plan_canvas_actions(command: CanvasCommand) -> Vec<CanvasUiAction> {
    match command {
        CanvasCommand::Present { url } => {
            if url.is_some() {
                vec![CanvasUiAction::Load(url), CanvasUiAction::Show]
            } else {
                vec![CanvasUiAction::Show]
            }
        }
        CanvasCommand::Hide => vec![CanvasUiAction::Hide],
        CanvasCommand::Navigate { url } => {
            vec![CanvasUiAction::Load(Some(url)), CanvasUiAction::Show]
        }
    }
}

fn apply_canvas_action(
    app: &gtk4::Application,
    canvas_widgets: &mut Option<CanvasWidgets>,
    action: CanvasUiAction,
) {
    match action {
        CanvasUiAction::Load(url) => {
            let canvas = ensure_canvas_window(app, canvas_widgets);
            load_canvas_target(&canvas.webview, url.as_deref());
        }
        CanvasUiAction::Show => {
            let canvas = ensure_canvas_window(app, canvas_widgets);
            canvas.window.present();
        }
        CanvasUiAction::Hide => {
            if let Some(canvas) = canvas_widgets.as_ref() {
                canvas.window.hide();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CanvasCommand, CanvasUiAction, plan_canvas_actions};

    #[test]
    fn present_without_url_only_shows_canvas() {
        let actions = plan_canvas_actions(CanvasCommand::Present { url: None });
        assert_eq!(actions, vec![CanvasUiAction::Show]);
    }

    #[test]
    fn present_with_url_loads_then_shows_canvas() {
        let actions = plan_canvas_actions(CanvasCommand::Present {
            url: Some("https://example.com".to_string()),
        });
        assert_eq!(
            actions,
            vec![
                CanvasUiAction::Load(Some("https://example.com".to_string())),
                CanvasUiAction::Show,
            ]
        );
    }

    #[test]
    fn navigate_always_loads_then_shows_canvas() {
        let actions = plan_canvas_actions(CanvasCommand::Navigate {
            url: "https://example.com".to_string(),
        });
        assert_eq!(
            actions,
            vec![
                CanvasUiAction::Load(Some("https://example.com".to_string())),
                CanvasUiAction::Show,
            ]
        );
    }
}
