use super::CmdResult;
use crate::core::flow_collect;
use crate::utils::dirs;
use serde::Serialize;
use serde_json;
use std::time::Instant;

#[derive(Serialize, Clone)]
pub struct FlowCollectStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub config: Option<FlowCollectConfig>,
}

#[derive(Serialize, Clone)]
pub struct FlowCollectConfig {
    pub remote_server: String,
    pub remote_token_masked: String,
    pub device_id: String,
}

#[derive(Serialize, Clone)]
pub struct FlowCollectStats {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub data: Option<String>,
    pub error: Option<String>,
}

/// Read x-flow-collect config from the runtime YAML file.
fn read_fc_config() -> Option<FlowCollectConfig> {
    let config_path = dirs::app_home_dir().ok()?.join("clash-verge.yaml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content).ok()?;

    let fc = doc.get("x-flow-collect")?.as_mapping()?;

    let remote_server = fc
        .get("remote-server")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let remote_token = fc
        .get("remote-token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let device_id = fc
        .get("device-id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if remote_server.is_empty() {
        return None;
    }

    let masked = if remote_token.len() > 4 {
        format!("****{}", &remote_token[remote_token.len() - 4..])
    } else if remote_token.is_empty() {
        "(empty)".to_string()
    } else {
        "****".to_string()
    };

    Some(FlowCollectConfig {
        remote_server,
        remote_token_masked: masked,
        device_id,
    })
}

/// Read the full remote-server and remote-token from config (internal use only).
fn read_fc_credentials() -> Option<(String, String)> {
    let config_path = dirs::app_home_dir().ok()?.join("clash-verge.yaml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content).ok()?;

    let fc = doc.get("x-flow-collect")?.as_mapping()?;

    let server = fc
        .get("remote-server")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = fc
        .get("remote-token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if server.is_empty() {
        return None;
    }
    Some((server, token))
}

#[tauri::command]
pub async fn get_flow_collect_status() -> FlowCollectStatus {
    let running = flow_collect::is_flow_collect_running();
    let pid = flow_collect::get_flow_collect_pid();
    let mut config = read_fc_config();

    // 如果 device_id 为空，尝试从服务器数据库查询实际设备名
    if let Some(ref mut cfg) = config {
        if cfg.device_id.is_empty() || cfg.device_id == "(auto)" {
            if let Some((server, token)) = read_fc_credentials() {
                let url = format!("{}/api/devices", server.trim_end_matches('/'));
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap_or_default();
                if let Ok(resp) = client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    if let Ok(body) = resp.text().await {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            if let Some(devices) = json["devices"].as_array() {
                                if let Some(first) = devices.first() {
                                    if let Some(name) = first.as_str() {
                                        cfg.device_id = name.to_string();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    FlowCollectStatus {
        running,
        pid,
        config,
    }
}

#[tauri::command]
pub async fn get_flow_collect_stats() -> FlowCollectStats {
    let Some((server, token)) = read_fc_credentials() else {
        return FlowCollectStats {
            reachable: false,
            latency_ms: None,
            data: None,
            error: Some("FlowCollect not configured".to_string()),
        };
    };

    let url = format!("{}/api/stats", server.trim_end_matches('/'));
    let start = Instant::now();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_default();

    match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
    {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            let status = resp.status();
            match resp.text().await {
                Ok(body) if status.is_success() => FlowCollectStats {
                    reachable: true,
                    latency_ms: Some(latency),
                    data: Some(body),
                    error: None,
                },
                Ok(body) => FlowCollectStats {
                    reachable: false,
                    latency_ms: Some(latency),
                    data: None,
                    error: Some(format!("HTTP {}: {}", status, &body[..body.len().min(200)])),
                },
                Err(e) => FlowCollectStats {
                    reachable: false,
                    latency_ms: Some(latency),
                    data: None,
                    error: Some(format!("Read body failed: {}", e)),
                },
            }
        }
        Err(e) => FlowCollectStats {
            reachable: false,
            latency_ms: None,
            data: None,
            error: Some(format!("Request failed: {}", e)),
        },
    }
}

#[tauri::command]
pub fn start_flow_collect_sidecar() -> CmdResult {
    let config_path = dirs::app_home_dir()
        .map_err(|e| e.to_string())?
        .join("clash-verge.yaml");
    let path_str = config_path
        .to_str()
        .ok_or_else(|| "Invalid config path".to_string())?
        .to_string();
    flow_collect::start_flow_collect(&path_str);
    Ok(())
}

#[tauri::command]
pub fn stop_flow_collect_sidecar() {
    flow_collect::stop_flow_collect();
}
