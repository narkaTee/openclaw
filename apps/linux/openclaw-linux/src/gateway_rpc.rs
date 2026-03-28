use crate::config::GatewayConfig;
use futures_util::{SinkExt, StreamExt};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use url::Url;

#[derive(Debug, Deserialize)]
struct EventFrame {
    #[serde(rename = "type")]
    frame_type: String,
    event: String,
    payload: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ResponseFrame {
    #[serde(rename = "type")]
    _frame_type: String,
    id: String,
    ok: bool,
    payload: Option<Value>,
    error: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RequestFrame {
    #[serde(rename = "type")]
    frame_type: String,
    id: String,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

pub async fn call(
    config: &GatewayConfig,
    method: &str,
    params: Option<Value>,
) -> Result<Value, String> {
    let url = config
        .url
        .as_deref()
        .ok_or_else(|| "Gateway URL not configured".to_string())?;
    let url = Url::parse(url).map_err(|err| err.to_string())?;
    debug!("gateway rpc call start method={} url={}", method, url);
    let (mut socket, _) = connect_async(url.as_str()).await.map_err(|err| {
        let message = err.to_string();
        warn!(
            "gateway rpc websocket connect failed method={} url={} error={}",
            method, url, message
        );
        message
    })?;

    let nonce = wait_for_connect_challenge(&mut socket).await;
    if let Err(error) = send_connect(&mut socket, config, nonce.as_deref()).await {
        warn!(
            "gateway rpc connect failed method={} url={} error={}",
            method, url, error
        );
        return Err(error);
    }

    let req_id = request_id("req");
    let frame = RequestFrame {
        frame_type: "req".to_string(),
        id: req_id.clone(),
        method: method.to_string(),
        params,
    };
    let payload = serde_json::to_string(&frame).map_err(|err| err.to_string())?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| err.to_string())?;

    let response = wait_for_response(&mut socket, &req_id).await?;
    if !response.ok {
        let message = response
            .error
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(|text| text.to_string())
            })
            .unwrap_or_else(|| "gateway request failed".to_string());
        warn!(
            "gateway rpc response not ok method={} req_id={} error={}",
            method, req_id, message
        );
        return Err(message);
    }

    Ok(response.payload.unwrap_or(Value::Null))
}

pub async fn health_ok(config: &GatewayConfig) -> Result<(), String> {
    let payload = call(config, "health", None).await?;
    if payload
        .get("ok")
        .and_then(|value| value.as_bool())
        .is_some_and(|ok| !ok)
    {
        return Err("Gateway health check failed".to_string());
    }
    Ok(())
}

async fn wait_for_connect_challenge(
    socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> Option<String> {
    let fut = async {
        loop {
            let message = socket.next().await?;
            let message = message.ok()?;
            let text = message_to_text(message)?;
            let event = serde_json::from_str::<EventFrame>(&text).ok()?;
            if event.frame_type == "event" && event.event == "connect.challenge" {
                let nonce = event
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.get("nonce"))
                    .and_then(|value| value.as_str())
                    .map(|text| text.to_string());
                return nonce;
            }
        }
    };
    timeout(Duration::from_millis(750), fut)
        .await
        .ok()
        .flatten()
}

async fn send_connect(
    socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    config: &GatewayConfig,
    nonce: Option<&str>,
) -> Result<(), String> {
    let req_id = request_id("connect");
    let mut auth_mode = "none";
    let mut params = json!({
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": {
            "id": "openclaw-linux-onboarding",
            "displayName": "OpenClaw Linux",
            "version": "dev",
            "platform": "linux",
            "mode": "ui"
        },
        "role": "operator",
        "scopes": ["operator.admin", "operator.approvals", "operator.pairing"],
        "caps": [],
        "locale": "en-US",
        "userAgent": "openclaw-linux/dev"
    });

    if let Some(token) = config.token.as_deref() {
        params["auth"] = json!({ "token": token });
        auth_mode = "token";
    } else if let Some(password) = config.password.as_deref() {
        params["auth"] = json!({ "password": password });
        auth_mode = "password";
    }
    debug!("gateway rpc send connect req_id={} auth_mode={}", req_id, auth_mode);

    let _ = nonce;

    let frame = RequestFrame {
        frame_type: "req".to_string(),
        id: req_id.clone(),
        method: "connect".to_string(),
        params: Some(params),
    };
    let payload = serde_json::to_string(&frame).map_err(|err| err.to_string())?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| err.to_string())?;

    let response = wait_for_response(socket, &req_id).await?;
    if !response.ok {
        let message = response
            .error
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(|text| text.to_string())
            })
            .unwrap_or_else(|| "gateway connect failed".to_string());
        warn!(
            "gateway connect response not ok req_id={} auth_mode={} error={}",
            req_id, auth_mode, message
        );
        return Err(message);
    }
    Ok(())
}

async fn wait_for_response(
    socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    id: &str,
) -> Result<ResponseFrame, String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|err| err.to_string())?;
        let text = match message_to_text(message) {
            Some(text) => text,
            None => continue,
        };
        let value: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
        let frame_type = value
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if frame_type != "res" {
            continue;
        }
        let response =
            serde_json::from_value::<ResponseFrame>(value).map_err(|err| err.to_string())?;
        if response.id == id {
            return Ok(response);
        }
    }
    Err("gateway response timed out".to_string())
}

fn message_to_text(message: Message) -> Option<String> {
    match message {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        _ => None,
    }
}

fn request_id(prefix: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{prefix}-{}", now.as_nanos())
}
