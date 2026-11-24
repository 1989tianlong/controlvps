import { invoke } from "@tauri-apps/api/core";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import type {
  ActionResult,
  AppInfo,
  BatchResult,
  ConnectionTest,
  DockerAction,
  DockerActionResult,
  DockerLogs,
  DockerOverview,
  FileListing,
  FileTransfer,
  OperationLog,
  PortForwardInput,
  PortForwardRule,
  QuickAction,
  Server,
  ServerInput,
  ServerMetrics,
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
}

export class CommandError extends Error {
  code: string;
  fingerprint?: string;

  constructor(payload: ErrorPayload) {
    super(payload.message);
    this.code = payload.code;
    this.fingerprint = payload.fingerprint;
  }
}

const inTauri = () => "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) {
    if (["list_servers", "list_operation_logs", "list_snippets", "list_port_forwards", "list_tunnel_statuses"].includes(command)) return [] as T;
    if (command === "get_app_info") {
      return {
        version: "0.6.0",
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
  listServers: () => call<Server[]>("list_servers"),
  createServer: (input: ServerInput) => call<Server>("create_server", { input }),
  importSshConfig: (path: string) => call<SshConfigImport>("import_ssh_config", { path }),
  updateServer: (id: string, input: ServerInput) => call<Server>("update_server", { id, input }),
  deleteServer: (id: string) => call<void>("delete_server", { id }),
  testConnection: (id: string, allowUnknownHost = false) =>
    call<ConnectionTest>("test_connection", { id, allowUnknownHost }),
  collectMetrics: (id: string, allowUnknownHost = false) =>
    call<ServerMetrics>("collect_metrics", { id, allowUnknownHost }),
  listDockerContainers: (id: string, allowUnknownHost = false) =>
    call<DockerOverview>("list_docker_containers", { id, allowUnknownHost }),
  runDockerAction: (id: string, containerId: string, action: DockerAction, allowUnknownHost = false) =>
    call<DockerActionResult>("run_docker_action", { id, containerId, action, allowUnknownHost }),
  getDockerLogs: (id: string, containerId: string, allowUnknownHost = false) =>
    call<DockerLogs>("get_docker_logs", { id, containerId, allowUnknownHost }),
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
};

export async function openExternal(url: string) {
  if (inTauri()) await tauriOpenUrl(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
