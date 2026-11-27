export interface ComposeAppTemplate {
  id: string;
  name: string;
  category: string;
  summary: string;
  serviceName: string;
  internalPort: number;
  hostPort: number;
  yaml: string;
}

export const composeAppTemplates: ComposeAppTemplate[] = [
  {
    id: "uptime-kuma",
    name: "Uptime Kuma",
    category: "监控告警",
    summary: "自托管网站/TCP/Docker 可用性监控面板，支持 90+ 通知渠道。",
    serviceName: "uptime-kuma",
    internalPort: 3001,
    hostPort: 3001,
    yaml: `services:
  uptime-kuma:
    image: louislam/uptime-kuma:1
    restart: unless-stopped
    ports:
      - "127.0.0.1:3001:3001"
    volumes:
      - uptime_kuma_data:/app/data

volumes:
  uptime_kuma_data:
`,
  },
  {
    id: "vaultwarden",
    name: "Vaultwarden",
    category: "安全密码",
    summary: "轻量级 Rust 实现的 Bitwarden 兼容自托管密码库，内存占用 < 30 MiB。",
    serviceName: "vaultwarden",
    internalPort: 80,
    hostPort: 8082,
    yaml: `services:
  vaultwarden:
    image: vaultwarden/server:latest
    restart: unless-stopped
    environment:
      SIGNUPS_ALLOWED: "false"
    ports:
      - "127.0.0.1:8082:80"
    volumes:
      - vw_data:/data

volumes:
  vw_data:
`,
  },
  {
    id: "n8n",
    name: "n8n Workflow",
    category: "自动化 / AI",
    summary: "开源可视化工作流与 AI Agent 编排引擎，支持 Webhook 与定时任务。",
    serviceName: "n8n",
    internalPort: 5678,
    hostPort: 5678,
    yaml: `services:
  n8n:
    image: docker.n8n.io/n8nio/n8n:latest
    restart: unless-stopped
    environment:
      GENERIC_TIMEZONE: Asia/Shanghai
      TZ: Asia/Shanghai
    ports:
      - "127.0.0.1:5678:5678"
    volumes:
      - n8n_data:/home/node/.n8n

volumes:
  n8n_data:
`,
  },
  {
    id: "open-webui",
    name: "Open WebUI",
    category: "自动化 / AI",
    summary: "现代自托管 AI 对话界面，支持 OpenAI / Anthropic / Ollama 与知识库。",
    serviceName: "open-webui",
    internalPort: 8080,
    hostPort: 3000,
    yaml: `services:
  open-webui:
    image: ghcr.io/open-webui/open-webui:main
    restart: unless-stopped
    ports:
      - "127.0.0.1:3000:8080"
    volumes:
      - open_webui_data:/app/backend/data

volumes:
  open_webui_data:
`,
  },
  {
    id: "memos",
    name: "Memos",
    category: "知识笔记",
    summary: "轻量隐私优先的 Markdown 碎片笔记与个人知识流服务。",
    serviceName: "memos",
    internalPort: 5230,
    hostPort: 5230,
    yaml: `services:
  memos:
    image: neosmemo/memos:stable
    restart: unless-stopped
    ports:
      - "127.0.0.1:5230:5230"
    volumes:
      - memos_data:/var/opt/memos

volumes:
  memos_data:
`,
  },
  {
    id: "beszel",
    name: "Beszel Hub",
    category: "监控告警",
    summary: "超轻量服务器与 Docker 资源监控中心，支持历史图表与告警。",
    serviceName: "beszel",
    internalPort: 8090,
    hostPort: 8090,
    yaml: `services:
  beszel:
    image: henrygd/beszel:latest
    restart: unless-stopped
    ports:
      - "127.0.0.1:8090:8090"
    volumes:
      - beszel_data:/beszel_data

volumes:
  beszel_data:
`,
  },
  {
    id: "umami",
    name: "Umami Analytics",
    category: "网站分析",
    summary: "无 Cookie、符合 GDPR 的轻量级开源网站访问统计服务（含 PostgreSQL）。",
    serviceName: "umami",
    internalPort: 3000,
    hostPort: 3005,
    yaml: `services:
  umami:
    image: ghcr.io/umami-software/umami:postgresql-latest
    restart: unless-stopped
    ports:
      - "127.0.0.1:3005:3000"
    environment:
      DATABASE_URL: postgresql://umami:change-me-strong-password@umami-db:5432/umami
      APP_SECRET: replace-with-random-32-char-secret
    depends_on:
      - umami-db

  umami-db:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_DB: umami
      POSTGRES_USER: umami
      POSTGRES_PASSWORD: change-me-strong-password
    volumes:
      - umami_pg:/var/lib/postgresql/data

volumes:
  umami_pg:
`,
  },
  {
    id: "caddy-proxy",
    name: "Caddy 自动 HTTPS 网关",
    category: "网络代理",
    summary: "自动申请与续期 Let's Encrypt TLS 证书的反向代理入口。",
    serviceName: "caddy",
    internalPort: 80,
    hostPort: 80,
    yaml: `services:
  caddy:
    image: caddy:2-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"
    command: caddy reverse-proxy --from example.com --to host.docker.internal:8080
    volumes:
      - caddy_data:/data
      - caddy_config:/config

volumes:
  caddy_data:
  caddy_config:
`,
  },
];

export function buildComposeWithAutoHttps(
  template: ComposeAppTemplate,
  domain?: string,
): string {
  const cleanDomain = domain?.trim().replace(/^https?:\/\//i, "").replace(/\/.*$/, "");
  if (!cleanDomain || template.id === "caddy-proxy") {
    if (cleanDomain && template.id === "caddy-proxy") {
      return template.yaml.replace("example.com", cleanDomain);
    }
    return template.yaml;
  }
  const caddyService = `
  caddy:
    image: caddy:2-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"
    command: caddy reverse-proxy --from ${cleanDomain} --to ${template.serviceName}:${template.internalPort}
    depends_on:
      - ${template.serviceName}
    volumes:
      - caddy_data:/data
      - caddy_config:/config
`;
  const hasVolumes = /\nvolumes:\s*\n/.test(template.yaml);
  if (hasVolumes) {
    return template.yaml
      .replace(/\nvolumes:\s*\n/, `${caddyService}\nvolumes:\n  caddy_data:\n  caddy_config:\n`);
  }
  return `${template.yaml}${caddyService}\nvolumes:\n  caddy_data:\n  caddy_config:\n`;
}
