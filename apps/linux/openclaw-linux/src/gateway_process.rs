use crate::config::{self, GatewayConfig, GatewayMode};
use crate::gateway_rpc;
use std::fs::{self, OpenOptions};
use std::process::Stdio;
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

const LOCAL_GATEWAY_START_TIMEOUT: Duration = Duration::from_secs(12);
const LOCAL_GATEWAY_PROBE_INTERVAL: Duration = Duration::from_millis(350);
const LOCAL_GATEWAY_LOG: &str = "/tmp/openclaw/openclaw-linux-gateway.log";

#[derive(Default)]
pub struct GatewayProcessManager {
    child: Mutex<Option<Child>>,
}

impl GatewayProcessManager {
    pub async fn ensure_local_gateway(&self, config: &GatewayConfig) -> Result<(), String> {
        if config.mode != GatewayMode::Local {
            return Ok(());
        }

        let mut effective = config.clone();
        if effective.token.is_none() && effective.password.is_none() {
            if let Some(token) = config::ensure_local_gateway_auth_token()? {
                effective.token = Some(token);
            }
        }

        if gateway_rpc::health_ok(&effective).await.is_ok() {
            return Ok(());
        }

        self.cleanup_exited_child().await;
        if self.child.lock().await.is_none() {
            self.spawn_local_gateway(effective.port, effective.token.as_deref())
                .await?;
        }

        let deadline = Instant::now() + LOCAL_GATEWAY_START_TIMEOUT;
        while Instant::now() < deadline {
            if gateway_rpc::health_ok(&effective).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(LOCAL_GATEWAY_PROBE_INTERVAL).await;
        }

        Err(format!(
            "Local gateway did not become ready in time. Check logs at {LOCAL_GATEWAY_LOG}."
        ))
    }

    pub async fn stop(&self) {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
        }
    }

    async fn cleanup_exited_child(&self) {
        let mut guard = self.child.lock().await;
        let should_clear = match guard.as_mut() {
            Some(child) => child.try_wait().ok().flatten().is_some(),
            None => false,
        };
        if should_clear {
            *guard = None;
        }
    }

    async fn spawn_local_gateway(&self, port: u16, token: Option<&str>) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(LOCAL_GATEWAY_LOG).parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOCAL_GATEWAY_LOG)
            .map_err(|err| err.to_string())?;
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOCAL_GATEWAY_LOG)
            .map_err(|err| err.to_string())?;

        let mut cmd = tokio::process::Command::new("openclaw");
        cmd.args([
            "gateway",
            "run",
            "--bind",
            "loopback",
            "--port",
            &port.to_string(),
            "--force",
        ]);
        if let Some(token) = token {
            cmd.env("OPENCLAW_GATEWAY_TOKEN", token);
        }
        cmd.stdout(Stdio::from(stdout));
        cmd.stderr(Stdio::from(stderr));

        let child = cmd.spawn().map_err(|err| err.to_string())?;
        let mut guard = self.child.lock().await;
        *guard = Some(child);
        Ok(())
    }
}
