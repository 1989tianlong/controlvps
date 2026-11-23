use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Password,
    Key,
    Agent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub private_key_path: Option<String>,
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub notes: Option<String>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub private_key_path: Option<String>,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub notes: Option<String>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub has_password: bool,
    pub has_key_passphrase: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    pub hostname: String,
    pub os: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetrics {
    pub server_id: String,
    pub hostname: String,
    pub os: String,
    pub uptime_seconds: f64,
    pub cpu_usage: f64,
    pub memory_total: u64,
    pub memory_used: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub network_rx_bytes_per_second: f64,
    pub network_tx_bytes_per_second: f64,
    pub load_average: [f64; 3],
    pub collected_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickAction {
    SystemInfo,
    Processes,
    DiskUsage,
    SystemLogs,
    CleanCache,
    Reboot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub action: QuickAction,
    pub command: String,
    pub output: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationLog {
    pub id: i64,
    pub server_id: String,
    pub server_name: String,
    pub action: String,
    pub success: bool,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub terminal_id: String,
    pub fingerprint: String,
}
