import { invoke } from "@tauri-apps/api/core";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import type {
  ActionResult,
  AlertEvent,
  AppInfo,
  BatchResult,
  BackupPreview,
  BackupRestoreResult,
  BackupSummary,
  ConnectionReport,
  ConnectionTest,
  ComposeAction,
  ComposeActionResult,
  ComposeLogs,
  ComposeOverview,
  DockerAction,
  DockerActionResult,
  DockerLogs,
  DockerOverview,
  FileListing,
  FileTransfer,
  FileTransferTask,
  MetricSample,
  MonitoringSettings,
  OperationLog,
  PortForwardInput,
  PortForwardRule,
  QuickAction,
  Runbook,
  RunbookInput,
  RunbookResult,
  Server,
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
    if (["list_servers", "list_operation_logs", "list_snippets", "list_runbooks", "list_port_forwards", "list_tunnel_statuses", "list_metric_history", "list_alert_events", "list_active_alerts", "list_service_monitors", "list_service_samples", "list_service_events"].includes(command)) return [] as T;
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
        version: "1.1.0",
        dataDir: "桌面应用数据目录",
        knownHostsPath: "桌面应用 known_hosts",
        credentialStore: "macOS Keychain",
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
  createEncryptedBackup: (path: string, password: string, includeCredentials: boolean) =>
    call<BackupSummary>("create_encrypted_backup", { path, password, includeCredentials }),
  previewEncryptedBackup: (path: string, password: string) =>
    call<BackupPreview>("preview_encrypted_backup", { path, password }),
  restoreEncryptedBackup: (path: string, password: string) =>
    call<BackupRestoreResult>("restore_encrypted_backup", { path, password }),
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
  listSystemdServices: (id: string, allowUnknownHost = false) =>
    call<SystemdOverview>("list_systemd_services", { id, allowUnknownHost }),
  runSystemdAction: (id: string, unit: string, action: SystemdAction, allowUnknownHost = false) =>
    call<SystemdActionResult>("run_systemd_action", { id, unit, action, allowUnknownHost }),
  getSystemdLogs: (id: string, unit: string, allowUnknownHost = false) =>
    call<SystemdLogs>("get_systemd_logs", { id, unit, allowUnknownHost }),
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
  runRunbook: (id: string, ids: string[], concurrency: number) =>
    call<RunbookResult>("run_runbook", { id, ids, concurrency }),
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
};

export async function openExternal(url: string) {
  if (inTauri()) await tauriOpenUrl(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
