use log::{info, warn};

#[derive(Clone, Debug)]
pub struct BridgeInvokeRequest {
    pub id: String,
    pub command: String,
    pub params_json: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BridgeInvokeResponse {
    pub id: String,
    pub ok: bool,
    pub payload_json: Option<String>,
    pub error: Option<NodeError>,
}

#[derive(Clone, Debug)]
pub struct NodeError {
    pub code: String,
    pub message: String,
}

pub trait CanvasPort: Send + Sync {
    fn present(&self, url: Option<String>);
    fn hide(&self);
    fn navigate(&self, url: String);
}

pub struct NodeRuntime {
    canvas: std::sync::Arc<dyn CanvasPort>,
}

impl NodeRuntime {
    pub fn new(canvas: std::sync::Arc<dyn CanvasPort>) -> Self {
        Self { canvas }
    }

    pub fn handle_invoke(&self, req: BridgeInvokeRequest) -> BridgeInvokeResponse {
        match req.command.as_str() {
            "canvas.present" => {
                let url = parse_canvas_url(req.params_json.as_deref(), false);
                info!(
                    "canvas.present target={}",
                    describe_canvas_target(url.as_deref())
                );
                self.canvas.present(url);
                BridgeInvokeResponse {
                    id: req.id,
                    ok: true,
                    payload_json: None,
                    error: None,
                }
            }
            "canvas.hide" => {
                info!("canvas.hide");
                self.canvas.hide();
                BridgeInvokeResponse {
                    id: req.id,
                    ok: true,
                    payload_json: None,
                    error: None,
                }
            }
            "canvas.navigate" => {
                let url = parse_canvas_url(req.params_json.as_deref(), true);
                match url {
                    Some(url) => {
                        info!(
                            "canvas.navigate target={}",
                            describe_canvas_target(Some(&url))
                        );
                        self.canvas.navigate(url);
                        BridgeInvokeResponse {
                            id: req.id,
                            ok: true,
                            payload_json: None,
                            error: None,
                        }
                    }
                    None => BridgeInvokeResponse {
                        id: req.id,
                        ok: false,
                        payload_json: None,
                        error: Some(NodeError {
                            code: "invalid_request".to_string(),
                            message: "INVALID_REQUEST: url required".to_string(),
                        }),
                    },
                }
            }
            "canvas.eval"
            | "canvas.snapshot"
            | "canvas.a2ui.reset"
            | "canvas.a2ui.push"
            | "canvas.a2ui.pushJSONL" => BridgeInvokeResponse {
                id: req.id,
                ok: false,
                payload_json: None,
                error: Some(NodeError {
                    code: "unavailable".to_string(),
                    message: "CANVAS_UNAVAILABLE: canvas not implemented yet".to_string(),
                }),
            },
            _ => BridgeInvokeResponse {
                id: req.id,
                ok: false,
                payload_json: None,
                error: Some(NodeError {
                    code: "invalid_request".to_string(),
                    message: "INVALID_REQUEST: unknown command".to_string(),
                }),
            },
        }
    }
}

fn parse_canvas_url(raw: Option<&str>, required: bool) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        if required {
            warn!("canvas url missing in paramsJSON: empty");
        }
        return None;
    }

    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            if required {
                warn!("canvas url missing in paramsJSON: {}", preview_params(raw));
            }
            return None;
        }
    };

    let url = value
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if required && url.is_none() {
        warn!("canvas url missing in paramsJSON: {}", preview_params(raw));
    }
    url
}

fn preview_params(raw: &str) -> String {
    let compact = raw.replace('\n', " ");
    const MAX: usize = 180;
    if compact.len() > MAX {
        format!("{}... (len={})", &compact[..MAX], compact.len())
    } else {
        compact
    }
}

fn describe_canvas_target(target: Option<&str>) -> String {
    let Some(target) = target.map(str::trim) else {
        return "(none)".to_string();
    };
    if target.is_empty() {
        return "(empty)".to_string();
    }
    if target.starts_with('<') {
        return format!("<inline-html len={}>", target.len());
    }

    const MAX: usize = 160;
    if target.len() > MAX {
        format!("{}... (len={})", &target[..MAX], target.len())
    } else {
        target.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{BridgeInvokeRequest, CanvasPort, NodeRuntime, parse_canvas_url};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeCanvasPort {
        present_calls: Mutex<Vec<Option<String>>>,
        hide_calls: Mutex<u32>,
        navigate_calls: Mutex<Vec<String>>,
    }

    impl CanvasPort for FakeCanvasPort {
        fn present(&self, url: Option<String>) {
            let mut calls = self.present_calls.lock().expect("present calls lock");
            calls.push(url);
        }

        fn hide(&self) {
            let mut calls = self.hide_calls.lock().expect("hide calls lock");
            *calls += 1;
        }

        fn navigate(&self, url: String) {
            let mut calls = self.navigate_calls.lock().expect("navigate calls lock");
            calls.push(url);
        }
    }

    #[test]
    fn parse_canvas_url_from_object_url() {
        let raw = r#"{"url":"https://example.com"}"#;
        assert_eq!(
            parse_canvas_url(Some(raw), true),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn parse_canvas_url_from_object_url_with_inline_html_string() {
        let raw = r#"{"url":"<html><body>hello</body></html>"}"#;
        assert_eq!(
            parse_canvas_url(Some(raw), true),
            Some("<html><body>hello</body></html>".to_string())
        );
    }

    #[test]
    fn parse_canvas_url_rejects_target_alias() {
        let raw = r#"{"target":"https://example.com/a"}"#;
        assert_eq!(parse_canvas_url(Some(raw), true), None);
    }

    #[test]
    fn parse_canvas_url_rejects_nested_params() {
        let raw = r#"{"params":{"url":"https://example.com/nested"}}"#;
        assert_eq!(parse_canvas_url(Some(raw), true), None);
    }

    #[test]
    fn parse_canvas_url_rejects_json_string_payload() {
        let raw = r#""https://example.com/raw""#;
        assert_eq!(parse_canvas_url(Some(raw), true), None);
    }

    #[test]
    fn parse_canvas_url_rejects_non_json_literal() {
        let raw = "<html><body>hello</body></html>";
        assert_eq!(parse_canvas_url(Some(raw), false), None);
    }

    #[test]
    fn runtime_routes_commands_to_canvas_port() {
        let canvas = Arc::new(FakeCanvasPort::default());
        let runtime = NodeRuntime::new(canvas.clone());

        let present = runtime.handle_invoke(BridgeInvokeRequest {
            id: "1".to_string(),
            command: "canvas.present".to_string(),
            params_json: Some(r#"{"url":"https://example.com"}"#.to_string()),
        });
        assert!(present.ok);

        let hide = runtime.handle_invoke(BridgeInvokeRequest {
            id: "2".to_string(),
            command: "canvas.hide".to_string(),
            params_json: None,
        });
        assert!(hide.ok);

        let navigate = runtime.handle_invoke(BridgeInvokeRequest {
            id: "3".to_string(),
            command: "canvas.navigate".to_string(),
            params_json: Some(r#"{"url":"https://example.com/nav"}"#.to_string()),
        });
        assert!(navigate.ok);

        let present_calls = canvas.present_calls.lock().expect("present lock");
        assert_eq!(
            present_calls.as_slice(),
            &[Some("https://example.com".to_string())]
        );
        drop(present_calls);

        let hide_calls = canvas.hide_calls.lock().expect("hide lock");
        assert_eq!(*hide_calls, 1);
        drop(hide_calls);

        let navigate_calls = canvas.navigate_calls.lock().expect("navigate lock");
        assert_eq!(
            navigate_calls.as_slice(),
            &["https://example.com/nav".to_string()]
        );
    }

    #[test]
    fn present_without_url_still_calls_present_with_none() {
        let canvas = Arc::new(FakeCanvasPort::default());
        let runtime = NodeRuntime::new(canvas.clone());

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "p-none".to_string(),
            command: "canvas.present".to_string(),
            params_json: None,
        });
        assert!(res.ok);

        let present_calls = canvas.present_calls.lock().expect("present lock");
        assert_eq!(present_calls.as_slice(), &[None]);
    }

    #[test]
    fn navigate_missing_url_returns_invalid_request_and_does_not_navigate() {
        let canvas = Arc::new(FakeCanvasPort::default());
        let runtime = NodeRuntime::new(canvas.clone());

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "n-missing".to_string(),
            command: "canvas.navigate".to_string(),
            params_json: Some("{}".to_string()),
        });
        assert!(!res.ok);
        let err = res.error.expect("navigate should return error");
        assert_eq!(err.code, "invalid_request");
        assert!(err.message.contains("url required"));

        let navigate_calls = canvas.navigate_calls.lock().expect("navigate lock");
        assert!(navigate_calls.is_empty());
    }

    #[test]
    fn unknown_command_returns_invalid_request() {
        let canvas = Arc::new(FakeCanvasPort::default());
        let runtime = NodeRuntime::new(canvas);

        let res = runtime.handle_invoke(BridgeInvokeRequest {
            id: "unknown".to_string(),
            command: "canvas.nonexistent".to_string(),
            params_json: None,
        });
        assert!(!res.ok);
        let err = res.error.expect("unknown command should return error");
        assert_eq!(err.code, "invalid_request");
        assert!(err.message.contains("unknown command"));
    }
}
