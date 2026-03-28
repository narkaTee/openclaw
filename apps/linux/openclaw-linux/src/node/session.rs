use crate::node::runtime::{BridgeInvokeRequest, BridgeInvokeResponse, NodeError};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::Signer;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const LINUX_APP_DEVICE_IDENTITY_FILE: &str = "linux-app-device.json";
const LINUX_APP_DEVICE_AUTH_FILE: &str = "linux-app-device-auth.json";

#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub role: String,
    pub caps: Vec<String>,
    pub commands: Vec<String>,
    pub permissions: HashMap<String, bool>,
    pub client_id: String,
    pub client_mode: String,
    pub client_display_name: String,
}

#[derive(Default)]
pub struct GatewayNodeSession {
    disconnect_tx: std::sync::Mutex<Option<mpsc::Sender<()>>>,
}

#[derive(Debug, Deserialize)]
struct EventFrame {
    #[serde(rename = "type")]
    frame_type: String,
    event: String,
    payload: Option<Value>,
    seq: Option<i64>,
    #[serde(rename = "stateVersion")]
    state_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ResponseFrame {
    #[serde(rename = "type")]
    frame_type: String,
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

#[derive(Debug, Deserialize)]
struct NodeInvokeRequestPayload {
    id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    command: String,
    #[serde(default, rename = "paramsJSON")]
    params_json: Option<String>,
    #[serde(default, rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DeviceIdentityFile {
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "publicKeyPem")]
    public_key_pem: String,
    #[serde(rename = "privateKeyPem")]
    private_key_pem: String,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthFile {
    #[serde(rename = "deviceId")]
    device_id: String,
    tokens: HashMap<String, DeviceAuthToken>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthToken {
    token: String,
    role: String,
}

impl GatewayNodeSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn connect(
        &self,
        url: &str,
        token: Option<&str>,
        password: Option<&str>,
        options: ConnectOptions,
        on_disconnect: impl Fn(String) + Send + Sync + 'static,
        on_invoke: impl Fn(BridgeInvokeRequest) -> BridgeInvokeResponse + Send + Sync + 'static,
    ) -> Result<(), String> {
        self.disconnect().await;
        info!("node session connect start url={}", url);
        let url = Url::parse(url).map_err(|err| err.to_string())?;
        let (mut stream, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|err| err.to_string())?;

        let connect_nonce = wait_for_connect_challenge(&mut stream).await;
        if let Some(nonce) = connect_nonce.as_deref() {
            info!(
                "gateway connect challenge received nonce_len={}",
                nonce.len()
            );
        } else {
            warn!("gateway connect challenge missing or timed out");
        }
        let device_identity = load_device_identity();
        let device_token = load_device_token(device_identity.as_ref());
        let auth_token = device_token.as_deref().or(token);
        if device_token.is_some() {
            info!("gateway auth using device token");
        } else if auth_token.is_some() {
            info!("gateway auth using shared token");
        } else if password.is_some() {
            info!("gateway auth using password");
        } else {
            warn!("gateway auth missing token/password");
        }

        let connect_id = uuid();
        let params = build_connect_params(
            &options,
            auth_token,
            password,
            connect_nonce.as_deref(),
            device_identity.as_ref(),
        )?;
        let frame = RequestFrame {
            frame_type: "req".to_string(),
            id: connect_id.clone(),
            method: "connect".to_string(),
            params: Some(params),
        };
        let payload = serde_json::to_string(&frame).map_err(|err| err.to_string())?;
        debug!("sending gateway connect request id={}", connect_id);
        stream
            .send(Message::Text(payload.into()))
            .await
            .map_err(|err| err.to_string())?;

        let response = wait_for_connect_response(&mut stream, &connect_id).await?;
        if !response.ok {
            let message = response
                .error
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "gateway connect failed".to_string());
            error!("gateway connect failed: {}", message);
            return Err(message);
        }
        info!("gateway connect ok id={}", connect_id);

        let (mut write, mut read) = stream.split();
        let (disconnect_tx, disconnect_rx) = mpsc::channel::<()>();
        {
            if let Ok(mut guard) = self.disconnect_tx.lock() {
                *guard = Some(disconnect_tx);
            }
        }
        tokio::spawn(async move {
            let disconnect_rx = disconnect_rx;
            loop {
                if disconnect_rx.try_recv().is_ok() {
                    info!("gateway disconnect requested");
                    let _ = write.send(Message::Close(None)).await;
                    break;
                }

                let message = match timeout(Duration::from_millis(250), read.next()).await {
                    Ok(message) => message,
                    Err(_) => continue,
                };
                let Some(message) = message else {
                    break;
                };
                match message {
                    Ok(Message::Text(text)) => {
                        if let Ok(event) = serde_json::from_str::<EventFrame>(&text) {
                            if event.frame_type == "event" && event.event == "node.invoke.request" {
                                if let Some(payload) = event.payload {
                                    if let Ok(request) =
                                        serde_json::from_value::<NodeInvokeRequestPayload>(payload)
                                    {
                                        info!(
                                            "node invoke request id={} command={}",
                                            request.id, request.command
                                        );
                                        let invoke = BridgeInvokeRequest {
                                            id: request.id.clone(),
                                            command: request.command.clone(),
                                            params_json: request.params_json.clone(),
                                        };
                                        let timeout_ms = request.timeout_ms.unwrap_or(30_000);
                                        let response =
                                            timeout(Duration::from_millis(timeout_ms), async {
                                                on_invoke(invoke)
                                            })
                                            .await
                                            .unwrap_or_else(|_| BridgeInvokeResponse {
                                                id: request.id.clone(),
                                                ok: false,
                                                payload_json: None,
                                                error: Some(NodeError {
                                                    code: "unavailable".to_string(),
                                                    message: "node invoke timed out".to_string(),
                                                }),
                                            });
                                        info!(
                                            "node invoke response id={} ok={}",
                                            response.id, response.ok
                                        );
                                        let result = build_invoke_result(&request, &response);
                                        let result_text =
                                            serde_json::to_string(&result).unwrap_or_default();
                                        let _ = write.send(Message::Text(result_text.into())).await;
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            warn!("gateway binary message: {}", text);
                            on_disconnect(text);
                        }
                        break;
                    }
                    Ok(Message::Close(_)) => {
                        warn!("gateway closed connection");
                        on_disconnect("gateway closed connection".to_string());
                        break;
                    }
                    Err(err) => {
                        error!("gateway socket error: {}", err);
                        on_disconnect(err.to_string());
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub async fn disconnect(&self) {
        let tx = {
            self.disconnect_tx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
        };
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
    }
}

async fn wait_for_connect_challenge(
    stream: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
) -> Option<String> {
    let fut = async {
        loop {
            let message = stream.next().await?;
            let message = message.ok()?;
            let text = message_to_text(message)?;
            let event = serde_json::from_str::<EventFrame>(&text).ok()?;
            if event.frame_type == "event" && event.event == "connect.challenge" {
                let nonce = event
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.get("nonce"))
                    .and_then(|value| value.as_str())
                    .map(|s| s.to_string());
                return nonce;
            }
        }
    };
    timeout(Duration::from_secs(6), fut).await.ok().flatten()
}

async fn wait_for_connect_response(
    stream: &mut tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    id: &str,
) -> Result<ResponseFrame, String> {
    while let Some(message) = stream.next().await {
        let message = message.map_err(|err| err.to_string())?;
        let text = match message_to_text(message) {
            Some(text) => text,
            None => continue,
        };
        let value: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
        let frame_type = value
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if frame_type != "res" {
            continue;
        }
        let response =
            serde_json::from_value::<ResponseFrame>(value).map_err(|err| err.to_string())?;
        if response.id == id {
            return Ok(response);
        }
    }
    Err("connect failed (no response)".to_string())
}

fn build_connect_params(
    options: &ConnectOptions,
    auth_token: Option<&str>,
    password: Option<&str>,
    nonce: Option<&str>,
    identity: Option<&DeviceIdentityFile>,
) -> Result<Value, String> {
    debug!(
        "build connect params caps={} commands={} permissions={}",
        options.caps.len(),
        options.commands.len(),
        options.permissions.len()
    );
    let locale = std::env::var("LANG")
        .ok()
        .and_then(|value| value.split('.').next().map(|s| s.to_string()))
        .unwrap_or_else(|| "en-US".to_string());

    let client = json!({
        "id": options.client_id,
        "displayName": options.client_display_name,
        "version": "dev",
        "platform": "linux",
        "mode": options.client_mode,
    });

    let mut params = json!({
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": client,
        "role": options.role,
        "scopes": [],
        "caps": options.caps,
        "commands": options.commands,
        "permissions": options.permissions,
        "locale": locale,
        "userAgent": "openclaw-linux/dev",
    });

    if let Some(token) = auth_token {
        params["auth"] = json!({ "token": token });
    } else if let Some(password) = password {
        params["auth"] = json!({ "password": password });
    }

    if let Some(identity) = identity {
        let signed_at_ms = current_time_ms();
        let scopes = "";
        let auth_value = auth_token.unwrap_or("");
        let payload = build_device_payload(
            identity,
            &options.client_id,
            &options.client_mode,
            &options.role,
            scopes,
            signed_at_ms,
            auth_value,
            nonce,
        )?;
        params["device"] = json!({
            "id": identity.device_id,
            "publicKey": payload.public_key,
            "signature": payload.signature,
            "signedAt": signed_at_ms,
            "nonce": nonce,
        });
    }

    Ok(params)
}

struct DeviceSignaturePayload {
    public_key: String,
    signature: String,
}

fn build_device_payload(
    identity: &DeviceIdentityFile,
    client_id: &str,
    client_mode: &str,
    role: &str,
    scopes: &str,
    signed_at_ms: i64,
    auth_token: &str,
    nonce: Option<&str>,
) -> Result<DeviceSignaturePayload, String> {
    let pem = pem::parse(&identity.private_key_pem)
        .map_err(|err| format!("device key parse failed: {err}"))?;
    let signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(&pem.contents())
        .map_err(|err| format!("device key parse failed: {err}"))?;
    let public_key = signing_key.verifying_key().to_bytes();
    let public_key = URL_SAFE_NO_PAD.encode(public_key);

    let version = if nonce.is_some() { "v2" } else { "v1" };
    let mut parts = vec![
        version.to_string(),
        identity.device_id.clone(),
        client_id.to_string(),
        client_mode.to_string(),
        role.to_string(),
        scopes.to_string(),
        signed_at_ms.to_string(),
        auth_token.to_string(),
    ];
    if let Some(nonce) = nonce {
        parts.push(nonce.to_string());
    }
    let payload = parts.join("|");
    let signature = signing_key.sign(payload.as_bytes()).to_bytes();
    let signature = URL_SAFE_NO_PAD.encode(signature);

    Ok(DeviceSignaturePayload {
        public_key,
        signature,
    })
}

fn load_device_identity() -> Option<DeviceIdentityFile> {
    match load_or_create_device_identity() {
        Ok(identity) => Some(identity),
        Err(error) => {
            warn!("failed to load linux app device identity: {}", error);
            None
        }
    }
}

fn load_device_token(identity: Option<&DeviceIdentityFile>) -> Option<String> {
    let identity = identity?;
    let path = resolve_identity_path(LINUX_APP_DEVICE_AUTH_FILE)?;
    let raw = fs::read_to_string(path).ok()?;
    let auth: DeviceAuthFile = serde_json::from_str(&raw).ok()?;
    if auth.device_id != identity.device_id {
        return None;
    }
    auth.tokens
        .get("node")
        .map(|token| token.token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn resolve_identity_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".openclaw")
            .join("identity")
            .join(name),
    )
}

fn load_or_create_device_identity() -> Result<DeviceIdentityFile, String> {
    let path = resolve_identity_path(LINUX_APP_DEVICE_IDENTITY_FILE)
        .ok_or_else(|| "HOME is not set".to_string())?;
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(existing) = serde_json::from_str::<DeviceIdentityFile>(&raw) {
            return Ok(existing);
        }
    }

    let generated = generate_device_identity()?;
    save_device_identity_file(&path, &generated)?;
    Ok(generated)
}

fn save_device_identity_file(path: &PathBuf, identity: &DeviceIdentityFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut bytes = serde_json::to_vec_pretty(identity).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
    Ok(())
}

fn generate_device_identity() -> Result<DeviceIdentityFile, String> {
    let mut seed = [0_u8; 32];
    let mut random = fs::File::open("/dev/urandom").map_err(|err| err.to_string())?;
    random
        .read_exact(&mut seed)
        .map_err(|err| err.to_string())?;

    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let public_key = signing_key.verifying_key().to_bytes();
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    let device_id = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let private_der = signing_key.to_pkcs8_der().map_err(|err| err.to_string())?;
    let private_key_pem = pem::encode_config(
        &pem::Pem::new("PRIVATE KEY", private_der.as_bytes()),
        pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF),
    );
    let public_der = signing_key
        .verifying_key()
        .to_public_key_der()
        .map_err(|err| err.to_string())?;
    let public_key_pem = pem::encode_config(
        &pem::Pem::new("PUBLIC KEY", public_der.as_bytes()),
        pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF),
    );

    Ok(DeviceIdentityFile {
        device_id,
        public_key_pem,
        private_key_pem,
    })
}

fn build_invoke_result(
    request: &NodeInvokeRequestPayload,
    response: &BridgeInvokeResponse,
) -> RequestFrame {
    let mut params = json!({
        "id": request.id,
        "nodeId": request.node_id,
        "ok": response.ok,
    });
    if let Some(payload) = &response.payload_json {
        params["payloadJSON"] = json!(payload);
    }
    if let Some(error) = &response.error {
        params["error"] = json!({
            "code": error.code,
            "message": error.message,
        });
    }
    RequestFrame {
        frame_type: "req".to_string(),
        id: uuid(),
        method: "node.invoke.result".to_string(),
        params: Some(params),
    }
}

fn message_to_text(message: Message) -> Option<String> {
    match message {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        _ => None,
    }
}

fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("node-{}", now.as_nanos())
}
