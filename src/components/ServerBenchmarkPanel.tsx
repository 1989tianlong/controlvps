import { useCallback, useEffect, useState } from "react";
import { BookOpenCheck, CheckCircle2, Copy, Cpu, Gauge, Globe2, HardDrive, LoaderCircle, ShieldCheck, XCircle } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { RunbookInput, RunbookStepInput, Server, ServerBenchmark, TrustedExecutor } from "../types";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
  onOpenRunbooks?: () => void;
}

function shellQuote(value: string) {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}

function buildShareMarkdown(server: Server, b: ServerBenchmark) {
  const endpoints = b.endpoints
    .map((e) => `${e.name}: ${e.reachable ? `OK (${e.latencyMs}ms)` : "不可达"}`)
    .join(" | ");
  const costText = typeof server.cost === "number"
    ? `${server.currency || "USD"} ${server.cost}/月`
    : "未登记";
  return [
    `### ${server.provider ? `${server.provider} ` : ""}${server.region ? `(${server.region}) ` : ""}VPS 验机报告 · 综合评分 ${b.score}/100`,
    `- **规格**：${b.cpuCores} vCPU (${b.cpuModel}, ${b.arch}, ${b.virtualization}) / ${b.memoryMb} MB RAM / ${b.diskGb} GB 磁盘`,
    `- **性能**：CPU SHA-256 ${b.cpuMbPerSec} MB/s · 磁盘顺序写 ${b.diskWriteMbPerSec} MB/s · AES-NI ${b.aesNi ? "支持" : "无"}`,
    `- **网络**：本机延迟 ${b.localRttMs ?? "—"} ms · IPv4 ${b.publicIpv4 ?? "—"} · IPv6 ${b.hasIpv6 ? "支持" : "无"} · ${b.asn ?? "未知 ASN"} (${b.location ?? "未知地区"})`,
    `- **开发源可达性**：${endpoints || "未测试"}`,
    `- **月成本**：${costText}（由 ControlVPS 无 Agent 验机生成）`,
  ].join("\n");
}

export function ServerBenchmarkPanel({ server, trusted, onOpenRunbooks }: Props) {
  const [history, setHistory] = useState<ServerBenchmark[]>([]);
  const [running, setRunning] = useState(false);
  const [creatingRunbook, setCreatingRunbook] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  // Provisioning wizard options
  const [createUser, setCreateUser] = useState(true);
  const [opsUsername, setOpsUsername] = useState("ops");
  const [hardenSsh, setHardenSsh] = useState(true);
  const [enableUfw, setEnableUfw] = useState(true);
  const [allowWebPorts, setAllowWebPorts] = useState(true);
  const [enableBbr, setEnableBbr] = useState(true);
  const [setupSwap, setSetupSwap] = useState(true);
  const [swapGb, setSwapGb] = useState(2);
  const [configureTimezone, setConfigureTimezone] = useState(true);
  const [timezone, setTimezone] = useState("Asia/Shanghai");
  const [installFail2ban, setInstallFail2ban] = useState(false);

  const loadHistory = useCallback(async () => {
    try {
      setHistory(await api.listServerBenchmarks(server.id));
    } catch {
      // ignore initial history load error
    }
  }, [server.id]);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  const runBenchmark = async () => {
    setRunning(true);
    setError("");
    setNotice("");
    try {
      const result = await trusted(server, (allow) => api.runServerBenchmark(server.id, allow));
      setHistory((current) => [result, ...current.filter((item) => item.id !== result.id)].slice(0, 20));
      setNotice(`验机完成（耗时 ${(result.durationMs / 1000).toFixed(1)} 秒），综合评分 ${result.score} / 100`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setRunning(false);
    }
  };

  const copyShare = async (benchmark: ServerBenchmark) => {
    try {
      await navigator.clipboard.writeText(buildShareMarkdown(server, benchmark));
      setNotice("已复制脱敏 Markdown 验机分享卡（已隐藏 IP 末段与主机敏感信息）");
    } catch {
      setError("复制失败，请检查剪贴板权限");
    }
  };

  const generateProvisionRunbook = async () => {
    const steps: RunbookStepInput[] = [];
    if (createUser) {
      const user = opsUsername.trim();
      if (!/^[a-z_][a-z0-9_-]{0,30}$/.test(user)) {
        setError("普通用户名仅支持小写字母、数字、下划线和连字符");
        return;
      }
      steps.push({
        name: `创建运维用户 ${user} 并复制 SSH 公钥`,
        command: `set -eu\nif ! id -u ${shellQuote(user)} >/dev/null 2>&1; then\n  useradd -m -s /bin/bash ${shellQuote(user)}\nfi\nif getent group sudo >/dev/null 2>&1; then usermod -aG sudo ${shellQuote(user)}; elif getent group wheel >/dev/null 2>&1; then usermod -aG wheel ${shellQuote(user)}; fi\nif [ -f /root/.ssh/authorized_keys ]; then\n  install -d -m 700 -o ${shellQuote(user)} -g ${shellQuote(user)} /home/${user}/.ssh\n  install -m 600 -o ${shellQuote(user)} -g ${shellQuote(user)} /root/.ssh/authorized_keys /home/${user}/.ssh/authorized_keys\nfi\nid ${shellQuote(user)}`,
      });
    }
    if (hardenSsh) {
      steps.push({
        name: "加固 SSH 配置（禁密码登录 + 校验后重载）",
        command: `set -eu\nmkdir -p /etc/ssh/sshd_config.d\ncat <<'EOF' > /etc/ssh/sshd_config.d/99-controlvps-hardening.conf\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin prohibit-password\nPubkeyAuthentication yes\nEOF\nsshd -t\nif command -v systemctl >/dev/null 2>&1; then systemctl reload sshd || systemctl reload ssh; else service ssh reload || service sshd reload; fi\necho "SSH 加固已生效（已通过 sshd -t 语法校验）"`,
      });
    }
    if (enableUfw) {
      const sshPort = Number.isInteger(server.port) && server.port > 0 ? server.port : 22;
      const webCmd = allowWebPorts ? "\nufw allow 80/tcp\nufw allow 443/tcp" : "";
      steps.push({
        name: `配置 UFW 防火墙（放行 SSH ${sshPort}${allowWebPorts ? " 与 80/443" : ""}）`,
        command: `set -eu\nif ! command -v ufw >/dev/null 2>&1; then\n  if command -v apt-get >/dev/null 2>&1; then apt-get update && apt-get install -y ufw; else echo "未检测到 ufw，请人工配置 firewalld/nftables"; exit 1; fi\nfi\nufw default deny incoming\nufw default allow outgoing\nufw allow ${sshPort}/tcp${webCmd}\nufw --force enable\nufw status verbose`,
      });
    }
    if (enableBbr) {
      steps.push({
        name: "开启 TCP BBR 拥塞控制与 FQ 队列",
        command: `set -eu\ncat <<'EOF' > /etc/sysctl.d/99-controlvps-bbr.conf\nnet.core.default_qdisc = fq\nnet.ipv4.tcp_congestion_control = bbr\nEOF\nsysctl --system >/dev/null\nsysctl net.ipv4.tcp_congestion_control net.core.default_qdisc`,
      });
    }
    if (setupSwap) {
      steps.push({
        name: `配置 ${swapGb} GB Swap 虚拟内存`,
        command: `set -eu\nif swapon --show | grep -q .; then\n  echo "系统已存在 Swap，跳过创建："\n  swapon --show\nelse\n  fallocate -l ${swapGb}G /swapfile || dd if=/dev/zero of=/swapfile bs=1M count=${swapGb * 1024} status=none\n  chmod 600 /swapfile\n  mkswap /swapfile\n  swapon /swapfile\n  grep -q '/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab\n  free -h\nfi`,
      });
    }
    if (configureTimezone) {
      steps.push({
        name: `设置时区为 ${timezone} 并启用 NTP`,
        command: `set -eu\nif command -v timedatectl >/dev/null 2>&1; then\n  timedatectl set-timezone ${shellQuote(timezone)}\n  timedatectl set-ntp true || true\n  timedatectl status | head -n 6\nelse\n  ln -sf /usr/share/zoneinfo/${timezone} /etc/localtime\n  date\nfi`,
      });
    }
    if (installFail2ban) {
      steps.push({
        name: "安装并启用 Fail2ban 防暴力破解",
        command: `set -eu\nif command -v apt-get >/dev/null 2>&1; then\n  apt-get update && apt-get install -y fail2ban\nelif command -v dnf >/dev/null 2>&1; then\n  dnf install -y fail2ban\nelse\n  echo "请人工安装 fail2ban"; exit 1\nfi\nsystemctl enable --now fail2ban\nfail2ban-client status`,
      });
    }
    if (steps.length === 0) {
      setError("请至少勾选一项初始化步骤");
      return;
    }

    setCreatingRunbook(true);
    setError("");
    setNotice("");
    try {
      const payload: RunbookInput = {
        name: `新机初始化 · ${server.name}`,
        description: `由 ControlVPS 新机初始化向导生成，共 ${steps.length} 个加固与优化步骤。执行前请逐行审查命令并确保已配置 SSH 公钥登录。`,
        steps,
      };
      await api.createRunbook(payload);
      setNotice(`已生成可审查的初始化 Runbook「${payload.name}」（${steps.length} 步），请前往「批量运维 → Runbook」审查并执行。`);
      if (onOpenRunbooks) onOpenRunbooks();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setCreatingRunbook(false);
    }
  };

  const latest = history[0];
  const valuePerDollar = latest && typeof server.cost === "number" && server.cost > 0
    ? (latest.score / server.cost).toFixed(1)
    : null;

  return (
    <div className="panel-body stack">
      {error && <div className="error-banner" role="alert">{error}</div>}
      {notice && <div className="file-notice" role="status">{notice}</div>}

      <section className="panel panel-body">
        <div className="section-heading mb-4">
          <div>
            <h3><Gauge size={16} /> 新机一键验机（无 Agent）</h3>
            <p className="table-meta">轻量实测 CPU SHA-256、磁盘顺序写、本机延迟、IPv4/IPv6、ASN 归属与开发源可达性（耗时约 8–15 秒，不留临时文件）。</p>
          </div>
          <div className="heading-actions">
            {latest && (
              <button className="btn" type="button" onClick={() => void copyShare(latest)}>
                <Copy size={14} />复制脱敏分享卡
              </button>
            )}
            <button className="btn btn-primary" type="button" disabled={running} onClick={() => void runBenchmark()}>
              {running ? <LoaderCircle size={15} className="spin" /> : <Gauge size={15} />}
              {running ? "正在验机…" : latest ? "重新验机" : "立即验机"}
            </button>
          </div>
        </div>

        {!latest ? (
          <p className="table-meta">尚未对 {server.name} 执行过验机。点击「立即验机」获取硬件与网络基线并保存到本地档案。</p>
        ) : (
          <>
            <div className="stats mb-4">
              <div className="stat">
                <span className="stat-label">综合评分</span>
                <strong className="stat-value">{latest.score} / 100</strong>
                <span className="stat-note">{valuePerDollar ? `性价比 ${valuePerDollar} 分/月币` : formatTime(latest.createdAt)}</span>
              </div>
              <div className="stat">
                <span className="stat-label"><Cpu size={13} /> CPU 吞吐</span>
                <strong className="stat-value">{latest.cpuMbPerSec} MB/s</strong>
                <span className="stat-note">{latest.cpuCores} 核 · {latest.arch} · {latest.aesNi ? "AES-NI" : "无 AES"}</span>
              </div>
              <div className="stat">
                <span className="stat-label"><HardDrive size={13} /> 磁盘写入</span>
                <strong className="stat-value">{latest.diskWriteMbPerSec} MB/s</strong>
                <span className="stat-note">{latest.diskGb} GB 根分区 · {latest.memoryMb} MB 内存</span>
              </div>
              <div className="stat">
                <span className="stat-label"><Globe2 size={13} /> 本机延迟</span>
                <strong className="stat-value">{latest.localRttMs !== undefined ? `${latest.localRttMs} ms` : "—"}</strong>
                <span className="stat-note">{latest.hasIpv6 ? "IPv4 + IPv6 双栈" : "仅 IPv4"} · {latest.virtualization}</span>
              </div>
            </div>

            <dl className="facts mb-4">
              <div><dt>处理器</dt><dd>{latest.cpuModel}</dd></div>
              <div><dt>系统与内核</dt><dd>{latest.os} ({latest.kernel})</dd></div>
              <div><dt>出口 IP 与 ASN</dt><dd>{latest.publicIpv4 || "未检测"} · {latest.asn || "未知 ASN"}</dd></div>
              <div><dt>机房位置</dt><dd>{latest.location || "未知"}</dd></div>
            </dl>

            {latest.endpoints.length > 0 && (
              <div>
                <span className="field-label">开发与 AI 端点可达性（从 VPS 出站实测）</span>
                <div className="flex flex-wrap gap-2 mt-2">
                  {latest.endpoints.map((ep) => (
                    <span key={ep.name} className={ep.reachable ? "badge badge-ok" : "badge badge-bad"}>
                      {ep.reachable ? <CheckCircle2 size={12} /> : <XCircle size={12} />}
                      {ep.name} {ep.reachable ? `${ep.latencyMs}ms` : "不可达"}
                    </span>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </section>

      <section className="panel panel-body">
        <div className="section-heading mb-4">
          <div>
            <h3><ShieldCheck size={16} /> 新机安全初始化向导</h3>
            <p className="table-meta">勾选所需加固与优化项，一键生成可逐行审查的 Runbook，不会未经确认直接修改服务器。</p>
          </div>
          <button className="btn btn-primary" type="button" disabled={creatingRunbook} onClick={() => void generateProvisionRunbook()}>
            <BookOpenCheck size={15} />{creatingRunbook ? "正在生成…" : "生成初始化 Runbook"}
          </button>
        </div>

        <div className="form-grid">
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={createUser} onChange={(e) => setCreateUser(e.target.checked)} />
              创建普通 sudo 用户并复制 root 公钥
            </label>
            {createUser && (
              <input className="input mt-2" value={opsUsername} onChange={(e) => setOpsUsername(e.target.value)} placeholder="例如 ops" aria-label="普通运维用户名" />
            )}
          </div>
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={hardenSsh} onChange={(e) => setHardenSsh(e.target.checked)} />
              SSH 加固（禁用密码登录、先 `sshd -t` 校验再重载）
            </label>
            <p className="field-help">执行前请确认当前已能通过 SSH 公钥登录，避免失联。</p>
          </div>
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={enableUfw} onChange={(e) => setEnableUfw(e.target.checked)} />
              开启 UFW 防火墙（自动放行当前 SSH 端口 {server.port}）
            </label>
            {enableUfw && (
              <label className="check-row mt-2">
                <input type="checkbox" checked={allowWebPorts} onChange={(e) => setAllowWebPorts(e.target.checked)} />
                同时放行 HTTP (80) 与 HTTPS (443)
              </label>
            )}
          </div>
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={enableBbr} onChange={(e) => setEnableBbr(e.target.checked)} />
              开启 TCP BBR 拥塞控制 + FQ 队列优化
            </label>
          </div>
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={setupSwap} onChange={(e) => setSetupSwap(e.target.checked)} />
              无 Swap 时自动创建 `/swapfile`
            </label>
            {setupSwap && (
              <select className="select mt-2" value={swapGb} onChange={(e) => setSwapGb(Number(e.target.value))} aria-label="Swap 大小">
                <option value={1}>1 GB Swap</option>
                <option value={2}>2 GB Swap</option>
                <option value={4}>4 GB Swap</option>
              </select>
            )}
          </div>
          <div className="field">
            <label className="check-row">
              <input type="checkbox" checked={configureTimezone} onChange={(e) => setConfigureTimezone(e.target.checked)} />
              统一系统时区并启用 NTP 校时
            </label>
            {configureTimezone && (
              <select className="select mt-2" value={timezone} onChange={(e) => setTimezone(e.target.value)} aria-label="系统时区">
                <option value="Asia/Shanghai">Asia/Shanghai (UTC+8)</option>
                <option value="UTC">UTC</option>
                <option value="Asia/Tokyo">Asia/Tokyo (UTC+9)</option>
                <option value="America/Los_Angeles">America/Los_Angeles</option>
              </select>
            )}
          </div>
          <div className="field span-2">
            <label className="check-row">
              <input type="checkbox" checked={installFail2ban} onChange={(e) => setInstallFail2ban(e.target.checked)} />
              安装并启用 Fail2ban（防 SSH 暴力破解）
            </label>
          </div>
        </div>
      </section>
    </div>
  );
}
