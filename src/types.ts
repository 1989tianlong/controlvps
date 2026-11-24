export type AuthMethod = "password" | "key" | "agent";

export type TrustedExecutor = <T>(
  server: Server,
  operation: (allowUnknownHost: boolean) => Promise<T>,
) => Promise<T>;

export interface ServerInput {
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  privateKeyPath?: string;
  password?: string;
  keyPassphrase?: string;
  groupName?: string;
  tags: string[];
  provider?: string;
  region?: string;
  notes?: string;
  cost?: number;
  currency?: string;
  expiresAt?: string;
  jumpServerId?: string;
}

export interface Server extends Omit<ServerInput, "password" | "keyPassphrase"> {
  id: string;
  createdAt: string;
  updatedAt: string;
  hasPassword: boolean;
  hasKeyPassphrase: boolean;
}

export interface ConnectionTest {
  success: boolean;
  message: string;
  latencyMs: number;
  hostname: string;
  os: string;
  fingerprint: string;
}

export interface DiagnosticStep {
  name: string;
  success: boolean;
  detail: string;
  durationMs: number;
}

export interface ConnectionReport {
  success: boolean;
  message: string;
  latencyMs: number;
  route: string[];
  steps: DiagnosticStep[];
  hostname: string;
  os: string;
  fingerprint: string;
}

export interface ServerMetrics {
  serverId: string;
  hostname: string;
  os: string;
  uptimeSeconds: number;
  cpuUsage: number;
  memoryTotal: number;
  memoryUsed: number;
  diskTotal: number;
  diskUsed: number;
  networkRxBytesPerSecond: number;
  networkTxBytesPerSecond: number;
  loadAverage: [number, number, number];
  collectedAt: string;
}

export interface MetricSample {
  id: number;
  serverId: string;
  cpuUsage: number;
  memoryUsage: number;
  diskUsage: number;
  networkRxBytesPerSecond: number;
  networkTxBytesPerSecond: number;
  loadAverage: number;
  collectedAt: string;
}

export interface MonitoringSettings {
  autoCollect: boolean;
  intervalSeconds: number;
  alertsEnabled: boolean;
  notificationsEnabled: boolean;
  cpuThreshold: number;
  memoryThreshold: number;
  diskThreshold: number;
  offlineAfter: number;
  cooldownMinutes: number;
}

export type AlertKind = "offline" | "cpu" | "memory" | "disk";
export type AlertStatus = "triggered" | "recovered";

export interface AlertEvent {
  id: number;
  serverId: string;
  serverName: string;
  kind: AlertKind;
  status: AlertStatus;
  message: string;
  value?: number;
  threshold?: number;
  createdAt: string;
}

export type QuickAction =
  | "system_info"
  | "processes"
  | "disk_usage"
  | "system_logs"
  | "clean_cache"
  | "reboot";

export interface ActionResult {
  action: QuickAction;
  command: string;
  output: string;
  exitCode: number;
  durationMs: number;
}

export interface OperationLog {
  id: number;
  serverId: string;
  serverName: string;
  action: string;
  success: boolean;
  summary: string;
  createdAt: string;
}

export interface TerminalSession {
  terminalId: string;
  fingerprint: string;
}

export type RemoteFileKind = "file" | "directory" | "symlink" | "other";

export interface RemoteFileEntry {
  name: string;
  path: string;
  kind: RemoteFileKind;
  size: number;
  modifiedAt?: string;
  permissions?: string;
}

export interface FileListing {
  path: string;
  parent?: string;
  entries: RemoteFileEntry[];
}

export interface FileTransfer {
  bytes: number;
  localPath: string;
  remotePath: string;
}

export type FileTransferStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface FileTransferTask {
  id: string;
  serverId: string;
  serverName: string;
  kind: "upload" | "download";
  status: FileTransferStatus;
  fileName: string;
  localPath: string;
  remotePath: string;
  totalBytes: number;
  transferredBytes: number;
  bytesPerSecond: number;
  error?: string;
  createdAt: string;
  startedAt?: string;
  finishedAt?: string;
}

export interface SnippetInput {
  name: string;
  description?: string;
  command: string;
}

export interface Snippet extends SnippetInput {
  id: string;
  createdAt: string;
  updatedAt: string;
}

export interface BatchExecutionResult {
  serverId: string;
  serverName: string;
  command: string;
  output: string;
  exitCode: number | null;
  success: boolean;
  error?: string;
  durationMs: number;
}

export interface BatchResult {
  results: BatchExecutionResult[];
  successCount: number;
  failureCount: number;
  durationMs: number;
}

export type PortForwardKind = "local" | "remote" | "dynamic";

export interface PortForwardInput {
  name: string;
  serverId: string;
  kind: PortForwardKind;
  listenAddress: string;
  listenPort: number;
  destinationHost: string;
  destinationPort: number;
}

export interface PortForwardRule extends PortForwardInput {
  id: string;
  createdAt: string;
  updatedAt: string;
}

export interface TunnelStatus {
  ruleId: string;
  serverId: string;
  localEndpoint: string;
  activeConnections: number;
  totalConnections: number;
  startedAt: string;
}

export interface SshConfigImport {
  imported: Server[];
  skipped: number;
  warnings: string[];
}

export interface DockerContainer {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  ports: string;
  services: DockerService[];
  createdAt: string;
  cpuPercent?: number;
  memoryPercent?: number;
  memoryUsage?: string;
  pids?: number;
}

export interface DockerService {
  containerId: string;
  containerName: string;
  hostPort: number;
  containerPort: number;
  protocol: string;
}

export interface DockerOverview {
  version: string;
  containers: DockerContainer[];
}

export type DockerAction = "start" | "stop" | "restart";

export interface DockerActionResult {
  containerId: string;
  action: DockerAction;
  output: string;
  exitCode: number;
}

export interface DockerLogs {
  containerId: string;
  output: string;
}

export interface SystemdService {
  name: string;
  load: string;
  active: string;
  sub: string;
  description: string;
  enabled: string;
}

export interface SystemdOverview {
  version: string;
  services: SystemdService[];
}

export type SystemdAction = "start" | "stop" | "restart";

export interface SystemdActionResult {
  unit: string;
  action: SystemdAction;
  output: string;
  exitCode: number;
}

export interface SystemdLogs {
  unit: string;
  output: string;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  knownHostsPath: string;
  credentialStore: string;
}

export interface CatalogOffer {
  id: string;
  provider: string;
  plan: string;
  regions: string[];
  cpu: number | null;
  memoryGb: number | null;
  diskGb: number | null;
  transferTb: number | null;
  monthlyPrice: number | null;
  currency: "USD" | "EUR";
  virtualization: string;
  sourceUrl: string;
  updatedAt: string;
}

export type FreeKind = "always_free" | "credit" | "trial" | "education";

export interface FreeProgram {
  id: string;
  provider: string;
  title: string;
  kind: FreeKind;
  summary: string;
  resources: string[];
  regions: string[];
  requiresCard: boolean | null;
  validity: string;
  billingRisk: string;
  sourceUrl: string;
  lastVerified: string;
  status: "verified" | "unverified";
  conditions: string[];
}
