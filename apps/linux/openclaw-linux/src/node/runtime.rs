use openclaw_core::node_runtime as core_runtime;
use openclaw_core::ui::UiControl;

pub type BridgeInvokeRequest = core_runtime::BridgeInvokeRequest;
pub type BridgeInvokeResponse = core_runtime::BridgeInvokeResponse;
pub type NodeError = core_runtime::NodeError;

pub struct NodeRuntime {
    inner: core_runtime::NodeRuntime,
}

impl NodeRuntime {
    pub fn new(ui: std::sync::Arc<dyn UiControl>) -> Self {
        let port = std::sync::Arc::new(UiCanvasPort { ui });
        Self {
            inner: core_runtime::NodeRuntime::new(port),
        }
    }

    pub fn handle_invoke(&self, req: BridgeInvokeRequest) -> BridgeInvokeResponse {
        self.inner.handle_invoke(req)
    }
}

struct UiCanvasPort {
    ui: std::sync::Arc<dyn UiControl>,
}

impl core_runtime::CanvasPort for UiCanvasPort {
    fn present(&self, url: Option<String>) {
        self.ui.present_canvas(url);
    }

    fn hide(&self) {
        self.ui.hide_canvas();
    }

    fn navigate(&self, url: String) {
        self.ui.navigate_canvas(url);
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgeInvokeRequest, NodeRuntime};
    use openclaw_core::ui::{NodeStatusView, OnboardingViewState, UiControl};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeUiControl {
        statuses: Mutex<Vec<NodeStatusView>>,
        present_calls: Mutex<Vec<Option<String>>>,
        hide_calls: Mutex<u32>,
        navigate_calls: Mutex<Vec<String>>,
    }

    impl UiControl for FakeUiControl {
        fn set_node_status(&self, status: NodeStatusView) {
            let mut statuses = self.statuses.lock().expect("status lock");
            statuses.push(status);
        }

        fn set_onboarding_state(&self, _state: OnboardingViewState) {}

        fn present_canvas(&self, url: Option<String>) {
            let mut calls = self.present_calls.lock().expect("present lock");
            calls.push(url);
        }

        fn hide_canvas(&self) {
            let mut calls = self.hide_calls.lock().expect("hide lock");
            *calls += 1;
        }

        fn navigate_canvas(&self, url: String) {
            let mut calls = self.navigate_calls.lock().expect("navigate lock");
            calls.push(url);
        }
    }

    #[test]
    fn runtime_maps_present_to_ui_control() {
        let ui = Arc::new(FakeUiControl::default());
        let runtime = NodeRuntime::new(ui.clone());

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "p".to_string(),
            command: "canvas.present".to_string(),
            params_json: Some(r#"{"url":"https://example.com"}"#.to_string()),
        });
        assert!(res.ok);

        let present_calls = ui.present_calls.lock().expect("present lock");
        assert_eq!(
            present_calls.as_slice(),
            &[Some("https://example.com".to_string())]
        );
    }

    #[test]
    fn runtime_maps_hide_to_ui_control() {
        let ui = Arc::new(FakeUiControl::default());
        let runtime = NodeRuntime::new(ui.clone());

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "h".to_string(),
            command: "canvas.hide".to_string(),
            params_json: None,
        });
        assert!(res.ok);

        let hide_calls = ui.hide_calls.lock().expect("hide lock");
        assert_eq!(*hide_calls, 1);
    }

    #[test]
    fn runtime_maps_navigate_to_ui_control() {
        let ui = Arc::new(FakeUiControl::default());
        let runtime = NodeRuntime::new(ui.clone());

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "n".to_string(),
            command: "canvas.navigate".to_string(),
            params_json: Some(r#"{"url":"https://example.com/nav"}"#.to_string()),
        });
        assert!(res.ok);

        let navigate_calls = ui.navigate_calls.lock().expect("navigate lock");
        assert_eq!(
            navigate_calls.as_slice(),
            &["https://example.com/nav".to_string()]
        );
    }
}
