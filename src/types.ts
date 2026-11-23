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
