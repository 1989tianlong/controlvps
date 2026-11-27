use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub jump_server_id: Option<String>,
    #[serde(default)]
    pub jump_chain: Vec<String>,
    #[serde(default)]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub cloud_account_id: Option<String>,
    #[serde(default)]
    pub cloud_resource_id: Option<String>,
    #[serde(default)]
    pub inherit_port: bool,
    #[serde(default)]
    pub inherit_username: bool,
    #[serde(default)]
    pub inherit_jump: bool,
    #[serde(default)]
    pub inherit_auth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    DigitalOcean,
    Tailscale,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAccountInput {
    pub name: String,
    pub provider: CloudProvider,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAccount {
    pub id: String,
    pub name: String,
    pub provider: CloudProvider,
    pub has_token: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileInput {
    pub name: String,
    pub auth_method: AuthMethod,
    pub private_key_path: Option<String>,
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfile {
    pub id: String,
    pub name: String,
    pub auth_method: AuthMethod,
    pub private_key_path: Option<String>,
    pub has_password: bool,
    pub has_key_passphrase: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerGroupInput {
    pub name: String,
    pub jump_server_id: Option<String>,
    pub auth_profile_id: Option<String>,
    pub default_port: Option<u16>,
    pub default_username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerGroup {
    pub id: String,
    pub name: String,
    pub jump_server_id: Option<String>,
    pub auth_profile_id: Option<String>,
    pub default_port: Option<u16>,
    pub default_username: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudResource {
    pub provider: CloudProvider,
    pub resource_id: String,
    pub name: String,
    pub host: String,
    pub region: Option<String>,
    pub status: String,
    pub tags: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudChangeKind {
    Added,
    Changed,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudChange {
    pub kind: CloudChangeKind,
    pub resource: CloudResource,
    pub server_id: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudPreview {
    pub account_id: String,
    pub provider: CloudProvider,
    pub fetched_at: String,
    pub added: usize,
    pub changed: usize,
    pub missing: usize,
    pub unchanged: usize,
    pub changes: Vec<CloudChange>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudImportRequest {
    pub account_id: String,
    pub resource_ids: Vec<String>,
    pub group_name: Option<String>,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub auth_profile_id: Option<String>,
    pub jump_server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudImportResult {
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub jump_server_id: Option<String>,
    #[serde(default)]
    pub jump_chain: Vec<String>,
    pub auth_profile_id: Option<String>,
    pub cloud_account_id: Option<String>,
    pub cloud_resource_id: Option<String>,
    #[serde(default)]
    pub inherit_port: bool,
    #[serde(default)]
    pub inherit_username: bool,
    #[serde(default)]
    pub inherit_jump: bool,
    #[serde(default)]
    pub inherit_auth: bool,
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
pub struct DiagnosticStep {
    pub name: String,
    pub success: bool,
    pub detail: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReport {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    pub route: Vec<String>,
    pub steps: Vec<DiagnosticStep>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSample {
    pub id: i64,
    pub server_id: String,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
    pub network_rx_bytes_per_second: f64,
    pub network_tx_bytes_per_second: f64,
    pub load_average: f64,
    pub collected_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringSettings {
    pub auto_collect: bool,
    pub interval_seconds: u32,
    pub alerts_enabled: bool,
    pub notifications_enabled: bool,
    pub cpu_threshold: f64,
    pub memory_threshold: f64,
    pub disk_threshold: f64,
    pub offline_after: u32,
    pub cooldown_minutes: u32,
}

impl Default for MonitoringSettings {
    fn default() -> Self {
        Self {
            auto_collect: false,
            interval_seconds: 30,
            alerts_enabled: true,
            notifications_enabled: true,
            cpu_threshold: 90.0,
            memory_threshold: 90.0,
            disk_threshold: 90.0,
            offline_after: 3,
            cooldown_minutes: 15,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMonitorKind {
    Http,
    Tcp,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorInput {
    pub name: String,
    pub kind: ServiceMonitorKind,
    pub target: String,
    pub port: Option<u16>,
    pub server_id: Option<String>,
    pub interval_seconds: u32,
    pub timeout_seconds: u32,
    pub expected_status: u16,
    pub keyword: Option<String>,
    pub failure_threshold: u32,
    pub certificate_warning_days: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitor {
    pub id: String,
    pub name: String,
    pub kind: ServiceMonitorKind,
    pub target: String,
    pub port: Option<u16>,
    pub server_id: Option<String>,
    pub interval_seconds: u32,
    pub timeout_seconds: u32,
    pub expected_status: u16,
    pub keyword: Option<String>,
    pub failure_threshold: u32,
    pub certificate_warning_days: u32,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSample {
    pub id: i64,
    pub monitor_id: String,
    pub success: bool,
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub detail: String,
    pub certificate_expires_at: Option<String>,
    pub certificate_days_remaining: Option<i32>,
    pub checked_at: String,
}

#[derive(Debug, Clone)]
pub struct ServiceProbe {
    pub success: bool,
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub detail: String,
    pub certificate_expires_at: Option<String>,
    pub certificate_days_remaining: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceEventKind {
    Availability,
    Certificate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEvent {
    pub id: i64,
    pub monitor_id: String,
    pub monitor_name: String,
    pub kind: ServiceEventKind,
    pub status: AlertStatus,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorView {
    pub monitor: ServiceMonitor,
    pub latest: Option<ServiceSample>,
    pub active_events: Vec<ServiceEvent>,
    pub consecutive_failures: u32,
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    pub uptime_30d: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCheckResult {
    pub view: ServiceMonitorView,
    pub events: Vec<ServiceEvent>,
    pub notification_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceNotificationSettings {
    pub enabled: bool,
    pub has_webhook_url: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceNotificationInput {
    pub enabled: bool,
    pub url: Option<String>,
    pub clear_url: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Offline,
    Cpu,
    Memory,
    Disk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertStatus {
    Triggered,
    Recovered,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub id: i64,
    pub server_id: String,
    pub server_name: String,
    pub kind: AlertKind,
    pub status: AlertStatus,
    pub message: String,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
    pub created_at: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub allow_execute: bool,
    pub allow_unknown_host: bool,
    pub token: String,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8799,
            allow_execute: false,
            allow_unknown_host: false,
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettingsInput {
    pub enabled: bool,
    pub port: u16,
    pub allow_execute: bool,
    pub allow_unknown_host: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallRecord {
    pub id: i64,
    pub tool: String,
    pub target: String,
    pub success: bool,
    pub duration_ms: u64,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub terminal_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub kind: RemoteFileKind,
    pub size: u64,
    pub modified_at: Option<String>,
    pub permissions: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<RemoteFileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTransfer {
    pub bytes: u64,
    pub local_path: String,
    pub remote_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileTransferKind {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileTransferStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTransferTask {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub kind: FileTransferKind,
    pub status: FileTransferStatus,
    pub file_name: String,
    pub local_path: String,
    pub remote_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub bytes_per_second: u64,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetInput {
    pub name: String,
    pub description: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub command: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExecutionResult {
    pub server_id: String,
    pub server_name: String,
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookStepInput {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookStep {
    pub id: String,
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookInput {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<RunbookStepInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runbook {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<RunbookStep>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookStepOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookStepResult {
    pub step_id: String,
    pub step_name: String,
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub outcome: RunbookStepOutcome,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookServerResult {
    pub server_id: String,
    pub server_name: String,
    pub steps: Vec<RunbookStepResult>,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookResult {
    pub runbook_name: String,
    pub results: Vec<RunbookServerResult>,
    pub success_count: usize,
    pub failure_count: usize,
    pub skipped_count: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookTrigger {
    Manual,
    Scheduled,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunbookExecutionStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Partial,
    Cancelled,
    TimedOut,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookExecution {
    pub id: String,
    pub runbook_id: String,
    pub runbook_name: String,
    pub schedule_id: Option<String>,
    pub retry_of: Option<String>,
    pub trigger: RunbookTrigger,
    pub status: RunbookExecutionStatus,
    pub target_ids: Vec<String>,
    pub concurrency: usize,
    pub canary_count: usize,
    pub timeout_seconds: u64,
    pub success_count: usize,
    pub failure_count: usize,
    pub skipped_count: usize,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub result: Option<RunbookResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookScheduleInput {
    pub runbook_id: String,
    pub server_ids: Vec<String>,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub concurrency: usize,
    pub canary_count: usize,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookSchedule {
    pub id: String,
    pub runbook_id: String,
    pub runbook_name: String,
    pub server_ids: Vec<String>,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub concurrency: usize,
    pub canary_count: usize,
    pub timeout_seconds: u64,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_execution_id: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct RunbookExecutionRequest {
    pub runbook: Runbook,
    pub target_ids: Vec<String>,
    pub concurrency: usize,
    pub canary_count: usize,
    pub timeout_seconds: u64,
    pub trigger: RunbookTrigger,
    pub schedule_id: Option<String>,
    pub retry_of: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult {
    pub results: Vec<BatchExecutionResult>,
    pub success_count: usize,
    pub failure_count: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortForwardKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardInput {
    pub name: String,
    pub server_id: String,
    pub kind: PortForwardKind,
    pub listen_address: String,
    pub listen_port: u16,
    pub destination_host: String,
    pub destination_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardRule {
    pub id: String,
    pub name: String,
    pub server_id: String,
    pub kind: PortForwardKind,
    pub listen_address: String,
    pub listen_port: u16,
    pub destination_host: String,
    pub destination_port: u16,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub rule_id: String,
    pub server_id: String,
    pub local_endpoint: String,
    pub active_connections: usize,
    pub total_connections: u64,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigImport {
    pub imported: Vec<Server>,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub services: Vec<DockerService>,
    pub created_at: String,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub memory_usage: Option<String>,
    pub pids: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerService {
    pub container_id: String,
    pub container_name: String,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerOverview {
    pub version: String,
    pub containers: Vec<DockerContainer>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerActionResult {
    pub container_id: String,
    pub action: DockerAction,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerLogs {
    pub container_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeProject {
    pub name: String,
    pub status: String,
    pub config_files: Vec<String>,
    pub working_directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeOverview {
    pub version: String,
    pub projects: Vec<ComposeProject>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeAction {
    Up,
    Down,
    Restart,
    Pull,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeActionResult {
    pub project: String,
    pub action: ComposeAction,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeLogs {
    pub project: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeEnvironmentEntry {
    pub key: String,
    pub managed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeEnvironmentChange {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeEditInput {
    pub project_name: String,
    pub working_directory: String,
    #[serde(default)]
    pub config_files: Vec<String>,
    pub content: String,
    #[serde(default)]
    pub environment_changes: Vec<ComposeEnvironmentChange>,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeApplication {
    pub project: ComposeProject,
    pub content: String,
    pub environment: Vec<ComposeEnvironmentEntry>,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeServiceChange {
    pub service: String,
    pub change: String,
    pub current_image: Option<String>,
    pub desired_image: Option<String>,
    pub current_image_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeploymentPlan {
    pub project: String,
    pub revision: String,
    pub services: Vec<ComposeServiceChange>,
    pub create_count: usize,
    pub update_count: usize,
    pub remove_count: usize,
    pub unchanged_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeImageSnapshot {
    pub service: String,
    pub image: String,
    pub image_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeDeploymentAction {
    Deploy,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeDeploymentStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeploymentRecord {
    pub id: String,
    pub project: String,
    pub action: ComposeDeploymentAction,
    pub status: ComposeDeploymentStatus,
    pub created_at: String,
    pub operator: String,
    pub snapshot_available: bool,
    pub snapshot_image_count: usize,
    pub revision_before: Option<String>,
    pub revision_after: Option<String>,
    pub changes: Vec<ComposeServiceChange>,
    pub images: Vec<ComposeImageSnapshot>,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeploymentResult {
    pub record: ComposeDeploymentRecord,
    pub plan: ComposeDeploymentPlan,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeploymentHistory {
    pub project: String,
    pub current_revision: String,
    pub records: Vec<ComposeDeploymentRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdService {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
    pub enabled: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdOverview {
    pub version: String,
    pub services: Vec<SystemdService>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemdAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdActionResult {
    pub unit: String,
    pub action: SystemdAction,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemdLogs {
    pub unit: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupCredentials {
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBackup {
    pub format_version: u32,
    pub created_at: String,
    pub servers: Vec<Server>,
    pub snippets: Vec<Snippet>,
    pub runbooks: Vec<Runbook>,
    pub port_forwards: Vec<PortForwardRule>,
    #[serde(default)]
    pub service_monitors: Vec<ServiceMonitor>,
    pub monitoring: MonitoringSettings,
    #[serde(default)]
    pub service_webhook_enabled: Option<bool>,
    #[serde(default)]
    pub service_webhook_url: Option<String>,
    pub includes_credentials: bool,
    pub credentials: HashMap<String, BackupCredentials>,
    #[serde(default)]
    pub compose_environments: HashMap<String, HashMap<String, HashMap<String, String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageUpdateKind {
    Normal,
    Security,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageUpdate {
    pub name: String,
    pub current_version: String,
    pub candidate_version: String,
    pub source: String,
    pub kind: PackageUpdateKind,
    pub kernel: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListeningPort {
    pub protocol: String,
    pub bind_address: String,
    pub port: u16,
    pub process: String,
    pub exposure: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirewallStatus {
    pub backend: String,
    pub state: String,
    pub active: bool,
    pub details: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySeverity {
    Pass,
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityFinding {
    pub id: String,
    pub title: String,
    pub severity: SecuritySeverity,
    pub value: String,
    pub detail: String,
    pub recommendation: String,
    pub remediation_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReport {
    pub server_id: String,
    pub hostname: String,
    pub os: String,
    pub package_manager: String,
    pub updates: Vec<PackageUpdate>,
    pub reboot_required: bool,
    pub reboot_reason: String,
    pub listening_ports: Vec<ListeningPort>,
    pub firewall: FirewallStatus,
    pub findings: Vec<SecurityFinding>,
    pub warnings: Vec<String>,
    pub collected_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResticRepositoryKind {
    Local,
    S3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionSourceKind {
    Directory,
    DockerVolume,
    Postgresql,
    Mysql,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProtectionInput {
    pub name: String,
    pub repository_kind: ResticRepositoryKind,
    pub repository: String,
    pub repository_password: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub source_kind: ProtectionSourceKind,
    pub paths: Vec<String>,
    pub volume_name: Option<String>,
    pub database_container: Option<String>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub database_password: Option<String>,
    pub keep_last: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProtectionPolicy {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub repository_kind: ResticRepositoryKind,
    pub repository: String,
    pub source_kind: ProtectionSourceKind,
    pub paths: Vec<String>,
    pub volume_name: Option<String>,
    pub database_container: Option<String>,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub keep_last: u32,
    pub has_repository_password: bool,
    pub has_s3_credentials: bool,
    pub has_database_password: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataProtectionAction {
    Initialize,
    Check,
    Backup,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataProtectionRunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProtectionRun {
    pub id: String,
    pub policy_id: String,
    pub action: DataProtectionAction,
    pub status: DataProtectionRunStatus,
    pub snapshot_id: Option<String>,
    pub target: Option<String>,
    pub output: String,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResticSnapshot {
    pub id: String,
    #[serde(alias = "short_id")]
    pub short_id: String,
    pub time: String,
    pub hostname: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProtectionOverview {
    pub restic_version: String,
    pub policies: Vec<DataProtectionPolicy>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProtectionRestoreResult {
    pub snapshot_id: String,
    pub target: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub path: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub server_count: usize,
    pub snippet_count: usize,
    pub runbook_count: usize,
    pub port_forward_count: usize,
    pub service_monitor_count: usize,
    pub includes_credentials: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub created_at: String,
    pub server_count: usize,
    pub new_server_count: usize,
    pub existing_server_count: usize,
    pub snippet_count: usize,
    pub runbook_count: usize,
    pub port_forward_count: usize,
    pub service_monitor_count: usize,
    pub includes_credentials: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreResult {
    pub imported_servers: usize,
    pub skipped_servers: usize,
    pub imported_snippets: usize,
    pub skipped_snippets: usize,
    pub imported_runbooks: usize,
    pub skipped_runbooks: usize,
    pub imported_port_forwards: usize,
    pub skipped_port_forwards: usize,
    pub imported_service_monitors: usize,
    pub skipped_service_monitors: usize,
}
