use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::path::PathBuf;

const LINUX_APP_CONFIG_OVERRIDE: &str = "openclaw.linux-app.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GatewayMode {
    Local,
    Remote,
    #[default]
    Unconfigured,
}

#[derive(Clone, Debug, Default)]
pub struct GatewayConfig {
    pub mode: GatewayMode,
    pub port: u16,
    pub url: Option<String>,
    pub token: Option<String>,
    pub password: Option<String>,
    pub remote_url: Option<String>,
    pub remote_target: Option<String>,
    pub remote_identity: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GatewayConnectionUpdate {
    pub mode: GatewayMode,
    pub remote_url: Option<String>,
    pub remote_target: Option<String>,
    pub remote_identity: Option<String>,
}

pub fn load_gateway_config() -> GatewayConfig {
    let env_url = read_env_trimmed("OPENCLAW_GATEWAY_URL");
    let env_token = read_env_trimmed("OPENCLAW_GATEWAY_TOKEN");
    let env_password = read_env_trimmed("OPENCLAW_GATEWAY_PASSWORD");

    let config_value = load_config_value();
    let mode = parse_gateway_mode(config_value.as_ref());
    let remote_url = resolve_remote_field(config_value.as_ref(), "url");
    let remote_target = resolve_remote_field(config_value.as_ref(), "sshTarget");
    let remote_identity = resolve_remote_field(config_value.as_ref(), "sshIdentity");
    let port = resolve_local_port(config_value.as_ref());

    let url = if env_url.is_some() {
        env_url
    } else {
        resolve_gateway_url(config_value.as_ref(), &mode, port, remote_url.clone())
    };

    let token = if env_token.is_some() {
        env_token
    } else {
        resolve_gateway_token(config_value.as_ref(), &mode)
    };

    let password = if env_password.is_some() {
        env_password
    } else {
        resolve_gateway_password(config_value.as_ref(), &mode)
    };

    GatewayConfig {
        mode,
        port,
        url,
        token,
        password,
        remote_url,
        remote_target,
        remote_identity,
    }
}

pub fn save_gateway_connection(update: &GatewayConnectionUpdate) -> Result<(), String> {
    let mut root = load_config_value().unwrap_or_else(|| Value::Object(Map::new()));
    let root_obj = as_object_mut(&mut root);

    let gateway_value = root_obj
        .entry("gateway".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let gateway_obj = as_object_mut(gateway_value);
    gateway_obj.insert(
        "mode".to_string(),
        Value::String(match update.mode {
            GatewayMode::Local => "local".to_string(),
            GatewayMode::Remote => "remote".to_string(),
            GatewayMode::Unconfigured => "unconfigured".to_string(),
        }),
    );

    if update.mode == GatewayMode::Remote {
        let remote_value = gateway_obj
            .entry("remote".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let remote_obj = as_object_mut(remote_value);
        upsert_trimmed(remote_obj, "url", update.remote_url.as_deref());
        upsert_trimmed(remote_obj, "sshTarget", update.remote_target.as_deref());
        upsert_trimmed(remote_obj, "sshIdentity", update.remote_identity.as_deref());
    }

    save_config_value(&root)
}

pub fn ensure_local_gateway_auth_token() -> Result<Option<String>, String> {
    let mut root = load_config_value().unwrap_or_else(|| Value::Object(Map::new()));
    let root_obj = as_object_mut(&mut root);
    let gateway_value = root_obj
        .entry("gateway".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let gateway_obj = as_object_mut(gateway_value);
    let auth_value = gateway_obj
        .entry("auth".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let auth_obj = as_object_mut(auth_value);

    if let Some(token) = auth_obj
        .get("token")
        .and_then(value_as_trimmed_str)
        .filter(|token| !token.is_empty())
    {
        return Ok(Some(token));
    }

    if auth_obj
        .get("password")
        .and_then(value_as_trimmed_str)
        .filter(|password| !password.is_empty())
        .is_some()
    {
        return Ok(None);
    }

    let token = random_token(32)?;
    auth_obj.insert("mode".to_string(), Value::String("token".to_string()));
    auth_obj.insert("token".to_string(), Value::String(token.clone()));
    save_config_value(&root)?;
    Ok(Some(token))
}

pub fn resolve_state_dir() -> PathBuf {
    if let Some(path) =
        read_env_trimmed("OPENCLAW_STATE_DIR").or_else(|| read_env_trimmed("CLAWDBOT_STATE_DIR"))
    {
        return expand_user_path(&path);
    }

    default_state_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

fn as_object_mut(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value should be object after normalization")
}

fn upsert_trimmed(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    match value.and_then(trimmed_or_none) {
        Some(value) => {
            map.insert(key.to_string(), Value::String(value.to_string()));
        }
        None => {
            map.remove(key);
        }
    }
}

fn parse_gateway_mode(root: Option<&Value>) -> GatewayMode {
    let mode = root
        .and_then(|root| root.get("gateway"))
        .and_then(|gateway| gateway.get("mode"))
        .and_then(value_as_trimmed_str);

    match mode.as_deref() {
        Some("local") => GatewayMode::Local,
        Some("remote") => GatewayMode::Remote,
        Some("unconfigured") => GatewayMode::Unconfigured,
        _ => {
            if resolve_remote_field(root, "url").is_some() {
                GatewayMode::Remote
            } else {
                GatewayMode::Unconfigured
            }
        }
    }
}

fn load_config_value() -> Option<Value> {
    let primary = resolve_config_path();
    let mut root = primary
        .as_ref()
        .and_then(|path| read_json5_file(path))
        .unwrap_or_else(|| Value::Object(Map::new()));

    if let Some(override_value) = read_json5_file(&override_config_path()) {
        merge_json(&mut root, &override_value);
    }

    Some(root)
}

fn save_config_value(root: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(root).map_err(|err| err.to_string())?;
    let primary =
        resolve_config_path().unwrap_or_else(|| resolve_state_dir().join("openclaw.json"));
    if let Some(parent) = primary.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    match fs::write(&primary, &bytes) {
        Ok(()) => Ok(()),
        Err(error) => {
            if !is_read_only_path_error(&primary, &error) {
                return Err(error.to_string());
            }

            let fallback = override_config_path();
            if let Some(parent) = fallback.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            fs::write(&fallback, bytes).map_err(|err| {
                format!(
                    "Config path is read-only ({}), and fallback write failed ({}): {}",
                    primary.display(),
                    fallback.display(),
                    err
                )
            })
        }
    }
}

fn resolve_gateway_url(
    root: Option<&Value>,
    mode: &GatewayMode,
    port: u16,
    remote_url: Option<String>,
) -> Option<String> {
    match mode {
        GatewayMode::Remote => remote_url,
        GatewayMode::Local => {
            let gateway = root.and_then(|root| root.get("gateway"));
            let host = gateway
                .and_then(|g| g.get("bind"))
                .and_then(value_as_trimmed_str)
                .and_then(|bind| {
                    if bind == "custom" {
                        gateway
                            .and_then(|g| g.get("customBindHost"))
                            .and_then(value_as_trimmed_str)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "127.0.0.1".to_string());
            Some(format!("ws://{host}:{port}"))
        }
        GatewayMode::Unconfigured => None,
    }
}

fn resolve_local_port(root: Option<&Value>) -> u16 {
    let gateway = root.and_then(|root| root.get("gateway"));
    read_env_trimmed("OPENCLAW_GATEWAY_PORT")
        .and_then(|raw| raw.parse::<u16>().ok())
        .or_else(|| {
            gateway
                .and_then(|g| g.get("port"))
                .and_then(|v| v.as_u64())
                .and_then(|v| u16::try_from(v).ok())
        })
        .unwrap_or(18789)
}

fn resolve_remote_field(root: Option<&Value>, key: &str) -> Option<String> {
    root.and_then(|root| root.get("gateway"))
        .and_then(|gateway| gateway.get("remote"))
        .and_then(|remote| remote.get(key))
        .and_then(value_as_trimmed_str)
}

fn resolve_gateway_token(root: Option<&Value>, mode: &GatewayMode) -> Option<String> {
    let gateway = root.and_then(|root| root.get("gateway"));
    let value = if *mode == GatewayMode::Remote {
        gateway
            .and_then(|g| g.get("remote"))
            .and_then(|r| r.get("token"))
            .and_then(value_as_trimmed_str)
    } else {
        gateway
            .and_then(|g| g.get("auth"))
            .and_then(|a| a.get("token"))
            .and_then(value_as_trimmed_str)
    };
    expand_env_value(value)
}

fn resolve_gateway_password(root: Option<&Value>, mode: &GatewayMode) -> Option<String> {
    let gateway = root.and_then(|root| root.get("gateway"));
    let value = if *mode == GatewayMode::Remote {
        gateway
            .and_then(|g| g.get("remote"))
            .and_then(|r| r.get("password"))
            .and_then(value_as_trimmed_str)
    } else {
        gateway
            .and_then(|g| g.get("auth"))
            .and_then(|a| a.get("password"))
            .and_then(value_as_trimmed_str)
    };
    expand_env_value(value)
}

fn resolve_config_path() -> Option<PathBuf> {
    if let Some(path) = read_env_trimmed("OPENCLAW_CONFIG_PATH")
        .or_else(|| read_env_trimmed("CLAWDBOT_CONFIG_PATH"))
    {
        return Some(expand_user_path(&path));
    }

    let state_dir = read_env_trimmed("OPENCLAW_STATE_DIR")
        .or_else(|| read_env_trimmed("CLAWDBOT_STATE_DIR"))
        .map(|value| expand_user_path(&value));

    let state_candidates = if let Some(dir) = state_dir {
        vec![dir]
    } else {
        default_state_dirs()
    };

    let config_filenames = [
        "openclaw.json",
        "clawdbot.json",
        "moldbot.json",
        "moltbot.json",
    ];

    for dir in &state_candidates {
        for name in &config_filenames {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    state_candidates
        .first()
        .map(|dir| dir.join("openclaw.json"))
}

fn override_config_path() -> PathBuf {
    resolve_state_dir().join(LINUX_APP_CONFIG_OVERRIDE)
}

fn read_json5_file(path: &PathBuf) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    json5::from_str(&raw).ok()
}

fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            for (key, value) in overlay_obj {
                if let Some(existing) = base_obj.get_mut(key) {
                    merge_json(existing, value);
                } else {
                    base_obj.insert(key.clone(), value.clone());
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

fn is_read_only_path_error(path: &PathBuf, error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
    ) {
        return true;
    }
    if error.raw_os_error() == Some(30) {
        return true;
    }
    if let Ok(target) = fs::read_link(path) {
        if target.starts_with("/nix/store") {
            return true;
        }
    }
    error
        .to_string()
        .to_lowercase()
        .contains("read-only file system")
}

fn default_state_dirs() -> Vec<PathBuf> {
    let home = env::var("HOME").ok().map(PathBuf::from);
    let home = match home {
        Some(path) => path,
        None => return Vec::new(),
    };
    vec![
        home.join(".openclaw"),
        home.join(".clawdbot"),
        home.join(".moldbot"),
        home.join(".moltbot"),
    ]
}

fn expand_user_path(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(input)
}

fn trimmed_or_none(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn read_env_trimmed(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .and_then(|value| trimmed_or_none(&value).map(|trimmed| trimmed.to_string()))
}

fn value_as_trimmed_str(value: &Value) -> Option<String> {
    value
        .as_str()
        .and_then(|text| trimmed_or_none(text).map(|trimmed| trimmed.to_string()))
}

fn expand_env_value(value: Option<String>) -> Option<String> {
    let raw = value?;
    let trimmed = raw.trim();
    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        let key = trimmed.trim_start_matches("${").trim_end_matches('}');
        return read_env_trimmed(key);
    }
    Some(raw)
}

fn random_token(size: usize) -> Result<String, String> {
    let mut bytes = vec![0_u8; size];
    let mut random = fs::File::open("/dev/urandom").map_err(|err| err.to_string())?;
    random
        .read_exact(&mut bytes)
        .map_err(|err| err.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("openclaw-linux-{label}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn with_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let prev: Vec<(&str, Option<String>)> = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var(*key).ok()))
            .collect();
        for (key, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        f();
        for (key, value) in prev {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }

    #[test]
    fn remote_mode_uses_remote_url() {
        let dir = temp_dir("remote-url");
        fs::write(
            dir.join("openclaw.json"),
            r#"
            {
              "gateway": {
                "mode": "remote",
                "remote": {
                  "url": "wss://gateway.example.ts.net:443"
                }
              }
            }
            "#,
        )
        .expect("write config");

        with_env(
            &[("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap()))],
            || {
                let cfg = load_gateway_config();
                assert_eq!(cfg.mode, GatewayMode::Remote);
                assert_eq!(cfg.url.as_deref(), Some("wss://gateway.example.ts.net:443"));
            },
        );
    }

    #[test]
    fn unconfigured_mode_has_no_gateway_url() {
        let dir = temp_dir("unconfigured");
        fs::write(
            dir.join("openclaw.json"),
            r#"
            {
              "gateway": {
                "mode": "unconfigured"
              }
            }
            "#,
        )
        .expect("write config");

        with_env(
            &[("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap()))],
            || {
                let cfg = load_gateway_config();
                assert_eq!(cfg.mode, GatewayMode::Unconfigured);
                assert_eq!(cfg.url, None);
            },
        );
    }

    #[test]
    fn local_mode_ignores_node_json_fallback() {
        let dir = temp_dir("ignore-node-json");
        fs::write(
            dir.join("openclaw.json"),
            r#"
            {
              "gateway": {
                "mode": "local",
                "port": 18789
              }
            }
            "#,
        )
        .expect("write openclaw.json");
        fs::write(
            dir.join("node.json"),
            r#"
            {
              "gateway": {
                "host": "10.1.2.3",
                "port": 19001,
                "tls": false
              }
            }
            "#,
        )
        .expect("write node.json");

        with_env(
            &[("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap()))],
            || {
                let cfg = load_gateway_config();
                assert_eq!(cfg.mode, GatewayMode::Local);
                assert_eq!(cfg.url.as_deref(), Some("ws://127.0.0.1:18789"));
            },
        );
    }

    #[test]
    fn read_only_primary_config_writes_linux_override() {
        let dir = temp_dir("readonly-primary");
        let target = dir.join("openclaw-target.json");
        fs::write(&target, r#"{"gateway":{"mode":"local","port":18789}}"#).expect("write target");

        let mut perms = fs::metadata(&target).expect("metadata").permissions();
        perms.set_readonly(true);
        fs::set_permissions(&target, perms).expect("set readonly");

        let link = dir.join("openclaw.json");
        std::os::unix::fs::symlink(&target, &link).expect("symlink primary");

        with_env(
            &[
                ("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap())),
                ("OPENCLAW_CONFIG_PATH", Some(link.to_str().unwrap())),
            ],
            || {
                let update = GatewayConnectionUpdate {
                    mode: GatewayMode::Remote,
                    remote_url: Some("wss://gateway.example.ts.net".to_string()),
                    remote_target: None,
                    remote_identity: None,
                };
                save_gateway_connection(&update).expect("save should use override fallback");
                let override_path = dir.join(LINUX_APP_CONFIG_OVERRIDE);
                assert!(override_path.exists());
                let merged = load_gateway_config();
                assert_eq!(merged.mode, GatewayMode::Remote);
                assert_eq!(merged.url.as_deref(), Some("wss://gateway.example.ts.net"));
            },
        );
    }
}
