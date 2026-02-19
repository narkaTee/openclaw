use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct GatewayConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct NodeConfig {
    host: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
}

pub fn load_gateway_config() -> GatewayConfig {
    let env_url = read_env_trimmed("OPENCLAW_GATEWAY_URL");
    let env_token = read_env_trimmed("OPENCLAW_GATEWAY_TOKEN");
    let env_password = read_env_trimmed("OPENCLAW_GATEWAY_PASSWORD");

    let config_value = load_config_value();
    let mode = config_value
        .as_ref()
        .and_then(|root| root.get("gateway"))
        .and_then(|gateway| gateway.get("mode"))
        .and_then(value_as_trimmed_str);

    let node_config = load_node_config();

    let url = if env_url.is_some() {
        env_url
    } else {
        resolve_gateway_url(config_value.as_ref(), mode.as_deref(), node_config.as_ref())
    };

    let token = if env_token.is_some() {
        env_token
    } else {
        resolve_gateway_token(config_value.as_ref(), mode.as_deref())
    };

    let password = if env_password.is_some() {
        env_password
    } else {
        resolve_gateway_password(config_value.as_ref(), mode.as_deref())
    };

    GatewayConfig {
        url,
        token,
        password,
    }
}

pub fn save_gateway_config(_config: &GatewayConfig) -> Result<(), String> {
    // TODO: persist to openclaw config file.
    Ok(())
}

fn load_config_value() -> Option<Value> {
    let path = resolve_config_path()?;
    let raw = fs::read_to_string(path).ok()?;
    json5::from_str(&raw).ok()
}

fn resolve_gateway_url(
    root: Option<&Value>,
    mode: Option<&str>,
    node_config: Option<&NodeConfig>,
) -> Option<String> {
    let gateway = root.and_then(|root| root.get("gateway"));
    let is_remote = matches!(mode, Some("remote"));
    if is_remote {
        if let Some(url) = gateway
            .and_then(|g| g.get("remote"))
            .and_then(|r| r.get("url"))
            .and_then(value_as_trimmed_str)
        {
            return Some(url);
        }
    }

    if let Some(node_config) = node_config {
        if let Some(host) = node_config.host.as_deref() {
            let port = node_config.port.unwrap_or(18789);
            let scheme = if node_config.tls.unwrap_or(false) {
                "wss"
            } else {
                "ws"
            };
            return Some(format!("{scheme}://{host}:{port}"));
        }
    }

    let port = read_env_trimmed("OPENCLAW_GATEWAY_PORT")
        .and_then(|raw| raw.parse::<u16>().ok())
        .or_else(|| {
            gateway
                .and_then(|g| g.get("port"))
                .and_then(|v| v.as_u64())
                .and_then(|v| u16::try_from(v).ok())
        })
        .unwrap_or(18789);

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

fn resolve_gateway_token(root: Option<&Value>, mode: Option<&str>) -> Option<String> {
    let gateway = root.and_then(|root| root.get("gateway"));
    let is_remote = matches!(mode, Some("remote"));
    let value = if is_remote {
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

fn resolve_gateway_password(root: Option<&Value>, mode: Option<&str>) -> Option<String> {
    let gateway = root.and_then(|root| root.get("gateway"));
    let is_remote = matches!(mode, Some("remote"));
    let value = if is_remote {
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

    state_candidates.get(0).map(|dir| dir.join("openclaw.json"))
}

fn load_node_config() -> Option<NodeConfig> {
    let path = resolve_node_config_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = json5::from_str(&raw).ok()?;
    let gateway = value.get("gateway")?;
    Some(NodeConfig {
        host: gateway.get("host").and_then(value_as_trimmed_str),
        port: gateway
            .get("port")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok()),
        tls: gateway.get("tls").and_then(|v| v.as_bool()),
    })
}

fn resolve_node_config_path() -> Option<PathBuf> {
    let base = read_env_trimmed("OPENCLAW_STATE_DIR")
        .or_else(|| read_env_trimmed("CLAWDBOT_STATE_DIR"))
        .map(|value| expand_user_path(&value))
        .or_else(|| default_state_dirs().get(0).cloned());
    base.map(|dir| dir.join("node.json"))
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

fn read_env_trimmed(key: &str) -> Option<String> {
    env::var(key).ok().and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn value_as_trimmed_str(value: &Value) -> Option<String> {
    value.as_str().and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
    fn loads_gateway_url_from_node_json() {
        let dir = temp_dir("node-json");
        let node_json = dir.join("node.json");
        fs::write(
            &node_json,
            r#"
            {
              "version": 1,
              "nodeId": "node-test",
              "displayName": "node-test",
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
            &[
                ("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap())),
                ("OPENCLAW_GATEWAY_URL", None),
            ],
            || {
                let cfg = load_gateway_config();
                assert_eq!(cfg.url.as_deref(), Some("ws://10.1.2.3:19001"));
            },
        );
    }

    #[test]
    fn node_json_tls_sets_wss() {
        let dir = temp_dir("node-json-tls");
        let node_json = dir.join("node.json");
        fs::write(
            &node_json,
            r#"
            {
              "version": 1,
              "nodeId": "node-test",
              "displayName": "node-test",
              "gateway": {
                "host": "gateway.local",
                "port": 18789,
                "tls": true
              }
            }
            "#,
        )
        .expect("write node.json");

        with_env(
            &[
                ("OPENCLAW_STATE_DIR", Some(dir.to_str().unwrap())),
                ("OPENCLAW_GATEWAY_URL", None),
            ],
            || {
                let cfg = load_gateway_config();
                assert_eq!(cfg.url.as_deref(), Some("wss://gateway.local:18789"));
            },
        );
    }
}
