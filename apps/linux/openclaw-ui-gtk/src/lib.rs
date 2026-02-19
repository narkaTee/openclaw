use gtk4::prelude::*;
use openclaw_core::ui::{NodeStatusView, UiApp, UiControl, UiEvent, UiEventSink};
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
    let command_receiver = std::cell::RefCell::new(Some(command_receiver));

    app.connect_shutdown(move |_| {
        if let Ok(guard) = event_sink.lock() {
            if let Some(sink) = guard.as_ref() {
                sink.on_event(UiEvent::AppClosed);
            }
        }
    });

    app.connect_activate(move |app| {
        let window = gtk4::ApplicationWindow::new(app);
        window.set_title(Some("OpenClaw"));
        window.set_default_size(900, 640);

        let status_label = gtk4::Label::new(None);
        status_label.set_xalign(0.0);

        let error_label = gtk4::Label::new(None);
        error_label.set_xalign(0.0);
        error_label.set_wrap(true);

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        vbox.set_margin_top(16);
        vbox.set_margin_bottom(16);
        vbox.set_margin_start(16);
        vbox.set_margin_end(16);
        vbox.append(&status_label);
        vbox.append(&error_label);
        window.set_child(Some(&vbox));

        if let Some(receiver) = command_receiver.borrow_mut().take() {
            let app = app.clone();
            let status_label = status_label.clone();
            let error_label = error_label.clone();
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
            // Parity with macOS: present-without-target should only reveal the
            // existing canvas window and keep current page state.
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
