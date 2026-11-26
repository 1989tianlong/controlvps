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
  authProfileId?: string;
  cloudAccountId?: string;
  cloudResourceId?: string;
  inheritPort?: boolean;
  inheritUsername?: boolean;
  inheritJump?: boolean;
  inheritAuth?: boolean;
}

export type CloudProvider = "digital_ocean" | "tailscale";

export interface CloudAccountInput {
  name: string;
  provider: CloudProvider;
  token?: string;
}

export interface CloudAccount {
  id: string;
  name: string;
  provider: CloudProvider;
  hasToken: boolean;
  createdAt: string;
  updatedAt: string;
  lastSyncedAt?: string;
}

export interface AuthProfileInput {
  name: string;
  authMethod: AuthMethod;
  privateKeyPath?: string;
  password?: string;
  keyPassphrase?: string;
}

export interface AuthProfile {
  id: string;
  name: string;
  authMethod: AuthMethod;
  privateKeyPath?: string;
  hasPassword: boolean;
  hasKeyPassphrase: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ServerGroupInput {
  name: string;
  jumpServerId?: string;
  authProfileId?: string;
  defaultPort?: number;
  defaultUsername?: string;
}

export interface ServerGroup {
  id: string;
  name: string;
  jumpServerId?: string;
  authProfileId?: string;
  defaultPort?: number;
  defaultUsername?: string;
  createdAt: string;
  updatedAt: string;
}

export type CloudChangeKind = "added" | "changed" | "missing";

export interface CloudResource {
  provider: CloudProvider;
  resourceId: string;
  name: string;
  host: string;
  region?: string;
  status: string;
  tags: string[];
  detail: string;
}

export interface CloudChange {
  kind: CloudChangeKind;
  resource: CloudResource;
  serverId?: string;
  summary: string;
}

export interface CloudPreview {
  accountId: string;
  provider: CloudProvider;
  fetchedAt: string;
  added: number;
  changed: number;
  missing: number;
  unchanged: number;
  changes: CloudChange[];
}

export interface CloudImportRequest {
  accountId: string;
  resourceIds: string[];
  groupName?: string;
  username?: string;
  port?: number;
  authProfileId?: string;
  jumpServerId?: string;
}

export interface CloudImportResult {
  created: number;
  updated: number;
  skipped: number;
}

export interface Server extends Omit<ServerInput, "password" | "keyPassphrase"> {
  id: string;
  createdAt: string;
  updatedAt: string;
  hasPassword: boolean;
  hasKeyPassphrase: boolean;
  inheritPort: boolean;
  inheritUsername: boolean;
  inheritJump: boolean;
  inheritAuth: boolean;
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

export type ServiceMonitorKind = "http" | "tcp" | "dns";

export interface ServiceMonitorInput {
  name: string;
  kind: ServiceMonitorKind;
  target: string;
  port?: number;
  serverId?: string;
  intervalSeconds: number;
  timeoutSeconds: number;
  expectedStatus: number;
  keyword?: string;
  failureThreshold: number;
  certificateWarningDays: number;
  enabled: boolean;
}

export interface ServiceMonitor extends ServiceMonitorInput {
  id: string;
  createdAt: string;
  updatedAt: string;
}

export interface ServiceSample {
  id: number;
  monitorId: string;
  success: boolean;
  latencyMs: number;
  statusCode?: number;
  detail: string;
  certificateExpiresAt?: string;
  certificateDaysRemaining?: number;
  checkedAt: string;
}

export type ServiceEventKind = "availability" | "certificate";

export interface ServiceEvent {
  id: number;
  monitorId: string;
  monitorName: string;
  kind: ServiceEventKind;
  status: AlertStatus;
  message: string;
  createdAt: string;
}

export interface ServiceMonitorView {
  monitor: ServiceMonitor;
  latest?: ServiceSample;
  activeEvents: ServiceEvent[];
  consecutiveFailures: number;
  uptime24h?: number;
  uptime7d?: number;
  uptime30d?: number;
}

export interface ServiceCheckResult {
  view: ServiceMonitorView;
  events: ServiceEvent[];
  notificationError?: string;
}

export interface ServiceNotificationSettings {
  enabled: boolean;
  hasWebhookUrl: boolean;
}

export type QuickAction =
  | "system_info"
  | "processes"
  | "disk_usage"
  | "system_logs"| "clean_cache"
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

export interface RunbookStepInput {
  name: string;
  command: string;
}

export interface RunbookStep extends RunbookStepInput {
  id: string;
}

export interface RunbookInput {
  name: string;
  description?: string;
  steps: RunbookStepInput[];
}

export interface Runbook extends Omit<RunbookInput, "steps"> {
  id: string;
  steps: RunbookStep[];
  createdAt: string;
  updatedAt: string;
}

export interface RunbookStepResult {
  stepId: string;
  stepName: string;
  command: string;
  output: string;
  exitCode: number | null;
  success: boolean;
  outcome: "succeeded" | "failed" | "cancelled" | "timed_out";
  error?: string;
  durationMs: number;
}

export interface RunbookServerResult {
  serverId: string;
  serverName: string;
  steps: RunbookStepResult[];
  success: boolean;
  error?: string;
  durationMs: number;
}

export interface RunbookResult {
  runbookName: string;
  results: RunbookServerResult[];
  successCount: number;
  failureCount: number;
  skippedCount: number;
  durationMs: number;
}

export type RunbookExecutionStatus = "queued" | "running" | "succeeded" | "failed" | "partial" | "cancelled" | "timed_out" | "interrupted";
export type RunbookTrigger = "manual" | "scheduled" | "retry";

export interface RunbookExecution {
  id: string;
  runbookId: string;
  runbookName: string;
  scheduleId?: string;
  retryOf?: string;
  trigger: RunbookTrigger;
  status: RunbookExecutionStatus;
  targetIds: string[];
  concurrency: number;
  canaryCount: number;
  timeoutSeconds: number;
  successCount: number;
  failureCount: number;
  skippedCount: number;
  createdAt: string;
  startedAt?: string;
  finishedAt?: string;
  result?: RunbookResult;
  error?: string;
}

export interface RunbookScheduleInput {
  runbookId: string;
  serverIds: string[];
  intervalMinutes: number;
  enabled: boolean;
  concurrency: number;
  canaryCount: number;
  timeoutSeconds: number;
}

export interface RunbookSchedule extends RunbookScheduleInput {
  id: string;
  runbookName: string;
  nextRunAt?: string;
  lastRunAt?: string;
  lastExecutionId?: string;
  lastError?: string;
  createdAt: string;
  updatedAt: string;
}

export type ResticRepositoryKind = "local" | "s3";
export type ProtectionSourceKind = "directory" | "docker_volume" | "postgresql" | "mysql";

export interface DataProtectionInput {
  name: string;
  repositoryKind: ResticRepositoryKind;
  repository: string;
  repositoryPassword?: string;
  s3AccessKeyId?: string;
  s3SecretAccessKey?: string;
  sourceKind: ProtectionSourceKind;
  paths: string[];
  volumeName?: string;
  databaseContainer?: string;
  databaseName?: string;
  databaseUser?: string;
  databasePassword?: string;
  keepLast: number;
}

export interface DataProtectionPolicy {
  id: string;
  serverId: string;
  name: string;
  repositoryKind: ResticRepositoryKind;
  repository: string;
  sourceKind: ProtectionSourceKind;
  paths: string[];
  volumeName?: string;
  databaseContainer?: string;
  databaseName?: string;
  databaseUser?: string;
  keepLast: number;
  hasRepositoryPassword: boolean;
  hasS3Credentials: boolean;
  hasDatabasePassword: boolean;
  createdAt: string;
  updatedAt: string;
}

export type DataProtectionAction = "initialize" | "check" | "backup" | "restore";
export type DataProtectionRunStatus = "running" | "succeeded" | "failed";

export interface DataProtectionRun {
  id: string;
  policyId: string;
  action: DataProtectionAction;
  status: DataProtectionRunStatus;
  snapshotId?: string;
  target?: string;
  output: string;
  error?: string;
  startedAt: string;
  finishedAt?: string;
  durationMs: number;
}

export interface ResticSnapshot {
  id: string;
  shortId: string;
  time: string;
  hostname: string;
  paths: string[];
  tags: string[];
}

export interface DataProtectionOverview {
  resticVersion: string;
  policies: DataProtectionPolicy[];
}

export interface DataProtectionRestoreResult {
  snapshotId: string;
  target: string;
  output: string;
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
  cpuPercent: number | null;
  memoryPercent: number | null;
  memoryUsage: string | null;
  pids: number | null;
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

export interface ComposeProject {
  name: string;
  status: string;
  configFiles: string[];
  workingDirectory: string;
}

export interface ComposeOverview {
  version: string;
  projects: ComposeProject[];
}

export type ComposeAction = "up" | "down" | "restart" | "pull";

export interface ComposeActionResult {
  project: string;
  action: ComposeAction;
  output: string;
  exitCode: number;
}

export interface ComposeLogs {
  project: string;
  output: string;
}

export interface ComposeEnvironmentEntry {
  key: string;
  managed: boolean;
}

export interface ComposeEnvironmentChange {
  key: string;
  value?: string | null;
}

export interface ComposeEditInput {
  projectName: string;
  workingDirectory: string;
  configFiles: string[];
  content: string;
  environmentChanges: ComposeEnvironmentChange[];
  expectedRevision: string;
}

export interface ComposeApplication {
  project: ComposeProject;
  content: string;
  environment: ComposeEnvironmentEntry[];
  revision: string;
}

export type ComposeServiceChangeKind = "create" | "update" | "remove" | "unchanged";

export interface ComposeServiceChange {
  service: string;
  change: ComposeServiceChangeKind;
  currentImage?: string;
  desiredImage?: string;
  currentImageId?: string;
}

export interface ComposeDeploymentPlan {
  project: string;
  revision: string;
  services: ComposeServiceChange[];
  createCount: number;
  updateCount: number;
  removeCount: number;
  unchangedCount: number;
}

export interface ComposeImageSnapshot {
  service: string;
  image: string;
  imageId?: string;
}

export interface ComposeDeploymentRecord {
  id: string;
  project: string;
  action: "deploy" | "rollback";
  status: "succeeded" | "failed";
  createdAt: string;
  operator: string;
  snapshotAvailable: boolean;
  snapshotImageCount: number;
  revisionBefore?: string;
  revisionAfter?: string;
  changes: ComposeServiceChange[];
  images: ComposeImageSnapshot[];
  output: string;
}

export interface ComposeDeploymentResult {
  record: ComposeDeploymentRecord;
  plan: ComposeDeploymentPlan;
}

export interface ComposeDeploymentHistory {
  project: string;
  currentRevision: string;
  records: ComposeDeploymentRecord[];
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

export type PackageUpdateKind = "normal" | "security";

export interface PackageUpdate {
  name: string;
  currentVersion: string;
  candidateVersion: string;
  source: string;
  kind: PackageUpdateKind;
  kernel: boolean;
}

export interface ListeningPort {
  protocol: string;
  bindAddress: string;
  port: number;
  process: string;
  exposure: "all_interfaces" | "external_interface" | "localhost";
}

export interface FirewallStatus {
  backend: string;
  state: string;
  active: boolean;
  details: string;
}

export type SecuritySeverity = "pass" | "info" | "warning" | "critical";

export interface SecurityFinding {
  id: string;
  title: string;
  severity: SecuritySeverity;
  value: string;
  detail: string;
  recommendation: string;
  remediationCommand?: string;
}

export interface SecurityReport {
  serverId: string;
  hostname: string;
  os: string;
  packageManager: "apt" | "dnf" | "yum" | "apk" | "unknown";
  updates: PackageUpdate[];
  rebootRequired: boolean;
  rebootReason: string;
  listeningPorts: ListeningPort[];
  firewall: FirewallStatus;
  findings: SecurityFinding[];
  warnings: string[];
  collectedAt: string;
}

export interface BackupSummary {
  path: string;
  createdAt: string;
  sizeBytes: number;
  serverCount: number;
  snippetCount: number;
  runbookCount: number;
  portForwardCount: number;
  serviceMonitorCount: number;
  includesCredentials: boolean;
}

export interface BackupPreview {
  createdAt: string;
  serverCount: number;
  newServerCount: number;
  existingServerCount: number;
  snippetCount: number;
  runbookCount: number;
  portForwardCount: number;
  serviceMonitorCount: number;
  includesCredentials: boolean;
}

export interface BackupRestoreResult {
  importedServers: number;
  skippedServers: number;
  importedSnippets: number;
  skippedSnippets: number;
  importedRunbooks: number;
  skippedRunbooks: number;
  importedPortForwards: number;
  skippedPortForwards: number;
  importedServiceMonitors: number;
  skippedServiceMonitors: number;
}

export interface AppInfo {  version: string;
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

export type DealPeriod = "month" | "quarter" | "year" | "multi_year" | "custom";

export interface DealPrice {
  amount: number;
  currency: "USD" | "EUR";
  period: DealPeriod;
  billingMonths: number;
  monthlyEquivalent: number;
}

export interface VpsDeal {
  id: string;
  source: string;
  sourceName: string;
  title: string;
  url: string;
  publishedAt: string;
  provider: string | null;
  summary: string;
  price: DealPrice | null;
  memoryGb: number | null;
  diskGb: number | null;
  cpuCores: number | null;
  bandwidthTb: number | null;
  locations: string[];
}

export interface DealSource {
  id: string;
  name: string;
  url: string;
  ok: boolean;
  message: string | null;
  itemCount: number;
}

export interface DealFeed {
  fetchedAt: string;
  sources: DealSource[];
  deals: VpsDeal[];
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
