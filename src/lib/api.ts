import { invoke } from "@tauri-apps/api/core";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import type {
  ActionCenterSummary,
  ActionResult,
  AiActionProposal,
  AiChatRequest,
  AiChatResponse,
  AiProposalExecutionResult,
  AiSettings,
  AiSettingsInput,
  AlertEvent,
  AppInfo,
  BatchResult,
  BackupPreview,
  BackupRestoreResult,
  BackupSummary,
  ByoSyncStatus,
  AuthProfile,
  AuthProfileInput,
  CloudAccount,
  CloudAccountInput,
  CloudImportRequest,
  CloudImportResult,
  CloudPreview,
  ConnectionReport,
  ConnectionTest,
  DataProtectionInput,
  DataProtectionOverview,
  DataProtectionPolicy,
  DataProtectionRestoreResult,
  DataProtectionRun,
  DealFeed,
  LogStreamSource,
  McpApprovalRequest,
  NotificationChannel,
  NotificationChannelInput,
  NotificationDelivery,
  ProcessEntry,
  RecordingDetail,
  RecordingHandle,
  RecordingSummary,
  RemoteEditSession,
  ResticSnapshot,
  SecretProviderStatus,
  SecretRefTestResult,
  ComposeAction,
  ComposeActionResult,
  ComposeApplication,
  ComposeDeploymentHistory,
  ComposeDeploymentPlan,
  ComposeDeploymentResult,
  ComposeEditInput,
  ComposeLogs,
  ComposeOverview,
  DockerAction,
  DockerActionResult,
  DockerLogs,
  DockerOverview,
  FileListing,
  FileTransfer,
  FileTransferTask,
  McpCallRecord,
  McpSettingsInput,
  McpStatus,
  MetricSample,
  MonitoringSettings,
  OperationLog,
  PortForwardInput,
  PortForwardRule,
  QuickAction,
  RemoteFreeCatalogResult,
  Runbook,
  RunbookExecution,
  RunbookInput,
  RunbookResult,
  RunbookSchedule,
  RunbookScheduleInput,
  SecurityReport,
  Server,
  ServerBenchmark,
  ServerGroup,
  ServerGroupInput,
  ServerInput,
  ServerMetrics,
  ServiceCheckResult,
  ServiceEvent,
  ServiceMonitorInput,
  ServiceMonitorView,
  ServiceNotificationSettings,
  ServiceSample,
  Snippet,
  SnippetInput,
  SshConfigImport,
  SystemdAction,
  SystemdActionResult,
  SystemdLogs,
  SystemdOverview,
  TerminalSession,
  TunnelStatus,
} from "../types";

interface ErrorPayload {
  code: string;
  message: string;
  fingerprint?: string;
  host?: string;
}

export class CommandError extends Error {
  code: string;
  fingerprint?: string;
  host?: string;

  constructor(payload: ErrorPayload) {
    super(payload.message);
    this.code = payload.code;
    this.fingerprint = payload.fingerprint;
    this.host = payload.host;
  }
}

const inTauri = () => "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) {
    if (["list_servers", "list_operation_logs", "list_snippets", "list_runbooks", "list_runbook_schedules", "list_runbook_executions", "list_data_protection_runs", "list_port_forwards", "list_tunnel_statuses", "list_metric_history", "list_alert_events", "list_active_alerts", "list_service_monitors", "list_service_samples", "list_service_events", "list_cloud_accounts", "list_auth_profiles", "list_server_groups", "list_terminal_recordings", "list_file_transfers", "list_mcp_calls", "list_notification_channels", "list_notification_deliveries", "list_pending_mcp_approvals", "list_server_benchmarks"].includes(command)) return [] as T;
    if (command === "get_ai_settings") {
      return {
        enabled: false,
        provider: "open_ai_compatible",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        permissionMode: "confirm",
        hasApiKey: false,
      } as T;
    }
    if (command === "get_action_center") {
      return {
        expiringServers: [],
        activeHostAlerts: [],
        activeServiceEvents: [],
        expiringCertificates: [],
        failedBackups: [],
        failedRunbooks: [],
        enabledNotificationChannels: 0,
        scheduledBackupPolicies: 0,
      } as T;
    }
    if (command === "get_mcp_status") {
      return {
        enabled: false,
        port: 8799,
        allowExecute: false,
        writePolicy: "read_only",
        allowUnknownHost: false,
        token: "",
        running: false,
        endpoint: "http://127.0.0.1:8799/mcp",
        error: null,
        tools: [],
      } as T;
    }
    if (command === "get_monitoring_settings") {
      return {
        autoCollect: false,
        intervalSeconds: 30,
        alertsEnabled: true,
        notificationsEnabled: true,
        cpuThreshold: 90,
        memoryThreshold: 90,
        diskThreshold: 90,
        offlineAfter: 3,
        cooldownMinutes: 15,
      } as T;
    }
    if (command === "get_service_notification_settings") {
      return { enabled: false, hasWebhookUrl: false } as T;
    }
    if (command === "get_app_info") {
      return {
        version: "1.14.0",
        dataDir: "桌面应用数据目录",
        knownHostsPath: "桌面应用 known_hosts",
        credentialStore: "系统钥匙串",
      } as T;
    }
    throw new CommandError({
      code: "DESKTOP_REQUIRED",
      message: "此操作需要在 ControlVPS 桌面应用中运行",
    });
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (typeof error === "object" && error !== null && "message" in error) {
      throw new CommandError(error as ErrorPayload);
    }
    throw new CommandError({ code: "UNKNOWN", message: String(error) });
  }
}

export const api = {
  appInfo: () => call<AppInfo>("get_app_info"),
  fetchVpsDeals: () => call<DealFeed>("fetch_vps_deals"),
  createEncryptedBackup: (path: string, password: string, includeCredentials: boolean) =>
    call<BackupSummary>("create_encrypted_backup", { path, password, includeCredentials }),
  previewEncryptedBackup: (path: string, password: string) =>
    call<BackupPreview>("preview_encrypted_backup", { path, password }),
  restoreEncryptedBackup: (path: string, password: string) =>
    call<BackupRestoreResult>("restore_encrypted_backup", { path, password }),
  dataProtectionOverview: (id: string, allowUnknownHost = false) =>
    call<DataProtectionOverview>("get_data_protection_overview", { id, allowUnknownHost }),
  createDataProtectionPolicy: (id: string, input: DataProtectionInput) =>
    call<DataProtectionPolicy>("create_data_protection_policy", { id, input }),
  updateDataProtectionPolicy: (id: string, input: DataProtectionInput) =>
    call<DataProtectionPolicy>("update_data_protection_policy", { id, input }),
  deleteDataProtectionPolicy: (id: string) => call<void>("delete_data_protection_policy", { id }),
  initializeDataProtection: (id: string, allowUnknownHost = false) =>
    call<DataProtectionRun>("initialize_data_protection", { id, allowUnknownHost }),
  checkDataProtection: (id: string, allowUnknownHost = false) =>
    call<DataProtectionRun>("check_data_protection", { id, allowUnknownHost }),
  runDataProtectionBackup: (id: string, allowUnknownHost = false) =>
    call<DataProtectionRun>("run_data_protection_backup", { id, allowUnknownHost }),
  dataProtectionSnapshots: (id: string, allowUnknownHost = false) =>
    call<ResticSnapshot[]>("list_data_protection_snapshots", { id, allowUnknownHost }),
  restoreDataProtectionSnapshot: (id: string, snapshotId: string, target?: string, allowUnknownHost = false) =>
    call<DataProtectionRestoreResult>("restore_data_protection_snapshot", { id, snapshotId, target, allowUnknownHost }),
  dataProtectionRuns: (id: string) => call<DataProtectionRun[]>("list_data_protection_runs", { id }),
  cloudAccounts: () => call<CloudAccount[]>("list_cloud_accounts"),
  createCloudAccount: (input: CloudAccountInput) => call<CloudAccount>("create_cloud_account", { input }),
  updateCloudAccount: (id: string, input: CloudAccountInput) => call<CloudAccount>("update_cloud_account", { id, input }),
  deleteCloudAccount: (id: string) => call<void>("delete_cloud_account", { id }),
  previewCloudAssets: (id: string) => call<CloudPreview>("preview_cloud_assets", { id }),
  importCloudAssets: (request: CloudImportRequest) => call<CloudImportResult>("import_cloud_assets", { request }),
  authProfiles: () => call<AuthProfile[]>("list_auth_profiles"),
  createAuthProfile: (input: AuthProfileInput) => call<AuthProfile>("create_auth_profile", { input }),
  updateAuthProfile: (id: string, input: AuthProfileInput) => call<AuthProfile>("update_auth_profile", { id, input }),
  deleteAuthProfile: (id: string) => call<void>("delete_auth_profile", { id }),
  serverGroups: () => call<ServerGroup[]>("list_server_groups"),
  createServerGroup: (input: ServerGroupInput) => call<ServerGroup>("create_server_group", { input }),
  updateServerGroup: (id: string, input: ServerGroupInput) => call<ServerGroup>("update_server_group", { id, input }),
  deleteServerGroup: (id: string) => call<void>("delete_server_group", { id }),
  listServers: () => call<Server[]>("list_servers"),
  createServer: (input: ServerInput) => call<Server>("create_server", { input }),
  importSshConfig: (path: string) => call<SshConfigImport>("import_ssh_config", { path }),
  updateServer: (id: string, input: ServerInput) => call<Server>("update_server", { id, input }),
  deleteServer: (id: string) => call<void>("delete_server", { id }),
  testConnection: (id: string, allowUnknownHost = false) =>
    call<ConnectionTest>("test_connection", { id, allowUnknownHost }),
  diagnoseConnection: (id: string, allowUnknownHost = false) =>
    call<ConnectionReport>("diagnose_connection", { id, allowUnknownHost }),
  collectMetrics: (id: string, allowUnknownHost = false) =>
    call<ServerMetrics>("collect_metrics", { id, allowUnknownHost }),
  metricHistory: (serverId: string, hours: number) =>
    call<MetricSample[]>("list_metric_history", { serverId, hours }),
  monitoringSettings: () => call<MonitoringSettings>("get_monitoring_settings"),
  updateMonitoringSettings: (settings: MonitoringSettings) =>
    call<MonitoringSettings>("update_monitoring_settings", { settings }),
  alertEvents: (serverId?: string) => call<AlertEvent[]>("list_alert_events", { serverId }),
  activeAlerts: () => call<AlertEvent[]>("list_active_alerts"),
  listServiceMonitors: () => call<ServiceMonitorView[]>("list_service_monitors"),
  createServiceMonitor: (input: ServiceMonitorInput) =>
    call<ServiceMonitorView>("create_service_monitor", { input }),
  updateServiceMonitor: (id: string, input: ServiceMonitorInput) =>
    call<ServiceMonitorView>("update_service_monitor", { id, input }),
  setServiceMonitorEnabled: (id: string, enabled: boolean) =>
    call<ServiceMonitorView>("set_service_monitor_enabled", { id, enabled }),
  deleteServiceMonitor: (id: string) => call<void>("delete_service_monitor", { id }),
  checkServiceMonitor: (id: string) => call<ServiceCheckResult>("check_service_monitor", { id }),
  serviceSamples: (id: string, hours: number) => call<ServiceSample[]>("list_service_samples", { id, hours }),
  serviceEvents: (id?: string) => call<ServiceEvent[]>("list_service_events", { id }),
  serviceNotificationSettings: () =>
    call<ServiceNotificationSettings>("get_service_notification_settings"),
  updateServiceNotificationSettings: (enabled: boolean, url?: string, clearUrl = false) =>
    call<ServiceNotificationSettings>("update_service_notification_settings", { input: { enabled, url, clearUrl } }),
  testServiceWebhook: () => call<void>("test_service_webhook"),
  listDockerContainers: (id: string, allowUnknownHost = false) =>
    call<DockerOverview>("list_docker_containers", { id, allowUnknownHost }),
  runDockerAction: (id: string, containerId: string, action: DockerAction, allowUnknownHost = false) =>
    call<DockerActionResult>("run_docker_action", { id, containerId, action, allowUnknownHost }),
  getDockerLogs: (id: string, containerId: string, allowUnknownHost = false) =>
    call<DockerLogs>("get_docker_logs", { id, containerId, allowUnknownHost }),
  listComposeProjects: (id: string, allowUnknownHost = false) =>
    call<ComposeOverview>("list_compose_projects", { id, allowUnknownHost }),
  runComposeAction: (id: string, project: string, action: ComposeAction, allowUnknownHost = false) =>
    call<ComposeActionResult>("run_compose_action", { id, project, action, allowUnknownHost }),
  getComposeLogs: (id: string, project: string, allowUnknownHost = false) =>
    call<ComposeLogs>("get_compose_logs", { id, project, allowUnknownHost }),
  getComposeApplication: (id: string, project: string, allowUnknownHost = false) =>
    call<ComposeApplication>("get_compose_application", { id, project, allowUnknownHost }),
  previewComposeDeployment: (id: string, input: ComposeEditInput, allowUnknownHost = false) =>
    call<ComposeDeploymentPlan>("preview_compose_deployment", { id, input, allowUnknownHost }),
  deployComposeApplication: (id: string, input: ComposeEditInput, allowUnknownHost = false) =>
    call<ComposeDeploymentResult>("deploy_compose_application", { id, input, allowUnknownHost }),
  listComposeDeployments: (id: string, project: string, allowUnknownHost = false) =>
    call<ComposeDeploymentHistory>("list_compose_deployments", { id, project, allowUnknownHost }),
  rollbackComposeDeployment: (id: string, project: string, deploymentId: string, expectedRevision: string, allowUnknownHost = false) =>
    call<ComposeDeploymentResult>("rollback_compose_deployment", { id, project, deploymentId, expectedRevision, allowUnknownHost }),
  listSystemdServices: (id: string, allowUnknownHost = false) =>
    call<SystemdOverview>("list_systemd_services", { id, allowUnknownHost }),
  runSystemdAction: (id: string, unit: string, action: SystemdAction, allowUnknownHost = false) =>
    call<SystemdActionResult>("run_systemd_action", { id, unit, action, allowUnknownHost }),
  getSystemdLogs: (id: string, unit: string, allowUnknownHost = false) =>
    call<SystemdLogs>("get_systemd_logs", { id, unit, allowUnknownHost }),
  collectSecurityReport: (id: string, allowUnknownHost = false) =>
    call<SecurityReport>("collect_security_report", { id, allowUnknownHost }),
  quickAction: (id: string, action: QuickAction, allowUnknownHost = false) =>
    call<ActionResult>("run_quick_action", { id, action, allowUnknownHost }),
  operationLogs: (serverId?: string) => call<OperationLog[]>("list_operation_logs", { serverId }),
  listSnippets: () => call<Snippet[]>("list_snippets"),
  createSnippet: (input: SnippetInput) => call<Snippet>("create_snippet", { input }),
  updateSnippet: (id: string, input: SnippetInput) => call<Snippet>("update_snippet", { id, input }),
  deleteSnippet: (id: string) => call<void>("delete_snippet", { id }),
  listRunbooks: () => call<Runbook[]>("list_runbooks"),
  createRunbook: (input: RunbookInput) => call<Runbook>("create_runbook", { input }),
  updateRunbook: (id: string, input: RunbookInput) => call<Runbook>("update_runbook", { id, input }),
  deleteRunbook: (id: string) => call<void>("delete_runbook", { id }),
  runRunbook: (id: string, ids: string[], concurrency: number, canaryCount: number) =>
    call<RunbookResult>("run_runbook", { id, ids, concurrency, canaryCount }),
  startRunbookExecution: (id: string, ids: string[], concurrency: number, canaryCount: number, timeoutSeconds: number) =>
    call<RunbookExecution>("start_runbook_execution", { id, ids, concurrency, canaryCount, timeoutSeconds }),
  runbookExecution: (id: string) => call<RunbookExecution>("get_runbook_execution", { id }),
  runbookExecutions: (runbookId?: string) => call<RunbookExecution[]>("list_runbook_executions", { runbookId }),
  cancelRunbookExecution: (id: string) => call<RunbookExecution>("cancel_runbook_execution", { id }),
  retryRunbookExecution: (id: string) => call<RunbookExecution>("retry_runbook_execution", { id }),
  runbookSchedules: () => call<RunbookSchedule[]>("list_runbook_schedules"),
  createRunbookSchedule: (input: RunbookScheduleInput) => call<RunbookSchedule>("create_runbook_schedule", { input }),
  updateRunbookSchedule: (id: string, input: RunbookScheduleInput) => call<RunbookSchedule>("update_runbook_schedule", { id, input }),
  setRunbookScheduleEnabled: (id: string, enabled: boolean) => call<RunbookSchedule>("set_runbook_schedule_enabled", { id, enabled }),
  deleteRunbookSchedule: (id: string) => call<void>("delete_runbook_schedule", { id }),
  runRunbookScheduleNow: (id: string) => call<RunbookExecution>("run_runbook_schedule_now", { id }),
  runBatchCommand: (ids: string[], command: string, concurrency: number) =>
    call<BatchResult>("run_batch_command", { ids, command, concurrency }),
  listPortForwards: () => call<PortForwardRule[]>("list_port_forwards"),
  createPortForward: (input: PortForwardInput) => call<PortForwardRule>("create_port_forward", { input }),
  deletePortForward: (id: string) => call<void>("delete_port_forward", { id }),
  startPortForward: (id: string, allowUnknownHost = false) =>
    call<TunnelStatus>("start_port_forward", { id, allowUnknownHost }),
  stopPortForward: (id: string) => call<void>("stop_port_forward", { id }),
  listTunnelStatuses: () => call<TunnelStatus[]>("list_tunnel_statuses"),
  openTerminal: (id: string, allowUnknownHost = false) =>
    call<TerminalSession>("open_terminal", { id, allowUnknownHost }),
  terminalInput: (terminalId: string, data: string) =>
    call<void>("terminal_input", { terminalId, data }),
  terminalResize: (terminalId: string, columns: number, rows: number) =>
    call<void>("terminal_resize", { terminalId, columns, rows }),
  terminalClose: (terminalId: string) => call<void>("terminal_close", { terminalId }),
  startTerminalRecording: (terminalId: string) =>
    call<RecordingHandle>("start_terminal_recording", { terminalId }),
  stopTerminalRecording: (terminalId: string) =>
    call<void>("stop_terminal_recording", { terminalId }),
  listTerminalRecordings: () => call<RecordingSummary[]>("list_terminal_recordings"),
  readTerminalRecording: (id: string) => call<RecordingDetail>("read_terminal_recording", { id }),
  deleteTerminalRecording: (id: string) => call<void>("delete_terminal_recording", { id }),
  listRemoteFiles: (id: string, path?: string, allowUnknownHost = false) =>
    call<FileListing>("list_remote_files", { id, path, allowUnknownHost }),
  createRemoteDirectory: (id: string, parent: string, name: string, allowUnknownHost = false) =>
    call<string>("create_remote_directory", { id, parent, name, allowUnknownHost }),
  renameRemoteFile: (id: string, path: string, newName: string, allowUnknownHost = false) =>
    call<string>("rename_remote_file", { id, path, newName, allowUnknownHost }),
  deleteRemoteFile: (id: string, path: string, recursive: boolean, allowUnknownHost = false) =>
    call<void>("delete_remote_file", { id, path, recursive, allowUnknownHost }),
  uploadRemoteFile: (id: string, localPath: string, remoteDirectory: string, fileName: string, allowUnknownHost = false) =>
    call<FileTransfer>("upload_remote_file", { id, localPath, remoteDirectory, fileName, allowUnknownHost }),
  downloadRemoteFile: (id: string, remotePath: string, localPath: string, allowUnknownHost = false) =>
    call<FileTransfer>("download_remote_file", { id, remotePath, localPath, allowUnknownHost }),
  enqueueFileUpload: (id: string, localPath: string, remoteDirectory: string, fileName: string, allowUnknownHost = false) =>
    call<FileTransferTask>("enqueue_file_upload", { id, localPath, remoteDirectory, fileName, allowUnknownHost }),
  enqueueFileDownload: (id: string, remotePath: string, localPath: string, allowUnknownHost = false) =>
    call<FileTransferTask>("enqueue_file_download", { id, remotePath, localPath, allowUnknownHost }),
  listFileTransfers: () => call<FileTransferTask[]>("list_file_transfers"),
  cancelFileTransfer: (id: string) => call<FileTransferTask>("cancel_file_transfer", { id }),
  retryFileTransfer: (id: string) => call<FileTransferTask>("retry_file_transfer", { id }),
  clearFinishedFileTransfers: (serverId: string) => call<void>("clear_finished_file_transfers", { serverId }),
  mcpStatus: () => call<McpStatus>("get_mcp_status"),
  updateMcpSettings: (input: McpSettingsInput) => call<McpStatus>("update_mcp_settings", { input }),
  rotateMcpToken: () => call<McpStatus>("rotate_mcp_token"),
  mcpCalls: (limit = 30) => call<McpCallRecord[]>("list_mcp_calls", { limit }),
  clearMcpCalls: () => call<void>("clear_mcp_calls"),
  listNotificationChannels: () => call<NotificationChannel[]>("list_notification_channels"),
  createNotificationChannel: (input: NotificationChannelInput) =>
    call<NotificationChannel>("create_notification_channel", { input }),
  updateNotificationChannel: (id: string, input: NotificationChannelInput) =>
    call<NotificationChannel>("update_notification_channel", { id, input }),
  setNotificationChannelEnabled: (id: string, enabled: boolean) =>
    call<NotificationChannel>("set_notification_channel_enabled", { id, enabled }),
  deleteNotificationChannel: (id: string) => call<void>("delete_notification_channel", { id }),
  testNotificationChannel: (id: string) => call<NotificationDelivery>("test_notification_channel", { id }),
  listNotificationDeliveries: (limit = 50) =>
    call<NotificationDelivery[]>("list_notification_deliveries", { limit }),
  clearNotificationDeliveries: () => call<void>("clear_notification_deliveries"),
  actionCenter: () => call<ActionCenterSummary>("get_action_center"),
  listPendingMcpApprovals: () => call<McpApprovalRequest[]>("list_pending_mcp_approvals"),
  resolveMcpApproval: (id: string, approved: boolean) => call<boolean>("resolve_mcp_approval", { id, approved }),
  getAiSettings: () => call<AiSettings>("get_ai_settings"),
  updateAiSettings: (input: AiSettingsInput) => call<AiSettings>("update_ai_settings", { input }),
  testAiConnection: () => call<string>("test_ai_connection"),
  aiChat: (request: AiChatRequest) => call<AiChatResponse>("ai_chat", { request }),
  executeAiProposal: (proposal: AiActionProposal) =>
    call<AiProposalExecutionResult>("execute_ai_proposal", { proposal }),
  runServerBenchmark: (id: string, allowUnknownHost = false) =>
    call<ServerBenchmark>("run_server_benchmark", { id, allowUnknownHost }),
  listServerBenchmarks: (serverId?: string) =>
    call<ServerBenchmark[]>("list_server_benchmarks", { serverId }),
  fetchFreeCatalog: () => call<RemoteFreeCatalogResult>("fetch_free_catalog"),
  detectSecretProviders: () => call<SecretProviderStatus[]>("detect_secret_providers"),
  testSecretRef: (reference: string) => call<SecretRefTestResult>("test_secret_ref", { reference }),
  listRemoteProcesses: (id: string, limit = 60, allowUnknownHost = false) =>
    call<ProcessEntry[]>("list_remote_processes", { id, limit, allowUnknownHost }),
  signalRemoteProcess: (id: string, pid: number, signal: string, allowUnknownHost = false) =>
    call<string>("signal_remote_process", { id, pid, signal, allowUnknownHost }),
  startLogStream: (id: string, source: LogStreamSource, allowUnknownHost = false) =>
    call<string>("start_log_stream", { id, source, allowUnknownHost }),
  stopLogStream: (streamId: string) => call<boolean>("stop_log_stream", { streamId }),
  openRemoteFileInEditor: (id: string, remotePath: string, editor: string, allowUnknownHost = false) =>
    call<RemoteEditSession>("open_remote_file_in_editor", { id, remotePath, editor, allowUnknownHost }),
  listRemoteEditSessions: (serverId?: string) =>
    call<RemoteEditSession[]>("list_remote_edit_sessions", { serverId }),
  stopRemoteEditSession: (sessionId: string) =>
    call<boolean>("stop_remote_edit_session", { sessionId }),
  getByoSyncStatus: () => call<ByoSyncStatus>("get_byo_sync_status"),
  updateByoSyncConfig: (folderPath: string, includeCredentials: boolean) =>
    call<ByoSyncStatus>("update_byo_sync_config", { folderPath, includeCredentials }),
  byoSyncPush: (password: string) => call<ByoSyncStatus>("byo_sync_push", { password }),
  byoSyncPull: (password: string) => call<BackupRestoreResult>("byo_sync_pull", { password }),
};

export async function openExternal(url: string) {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error("无效的外部链接地址");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("仅允许打开 http 或 https 链接");
  }
  const safeUrl = parsed.toString();
  if (inTauri()) await tauriOpenUrl(safeUrl);
  else window.open(safeUrl, "_blank", "noopener,noreferrer");
}

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
