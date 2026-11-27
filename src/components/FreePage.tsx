import { useState } from "react";
import { ArrowUpRight, Plus, RefreshCw, Search, ShieldAlert } from "lucide-react";
import { freeKindLabels, freePrograms as builtinPrograms } from "../data/catalog";
import { api, errorMessage, openExternal } from "../lib/api";
import type { FreeKind, FreeProgram, ServerInput } from "../types";

interface Props {
  onRegister?: (preset: Partial<ServerInput>) => void;
}

export function FreePage({ onRegister }: Props) {
  const [programs, setPrograms] = useState<FreeProgram[]>(builtinPrograms);
  const [kind, setKind] = useState<FreeKind | "">("");
  const [noCardOnly, setNoCardOnly] = useState(false);
  const [query, setQuery] = useState("");
  const [syncing, setSyncing] = useState(false);
  const [syncNote, setSyncNote] = useState(`本地内置 ${builtinPrograms.length} 条渠道`);
  const [error, setError] = useState("");

  const syncRemote = async () => {
    setSyncing(true);
    setError("");
    try {
      const remote = await api.fetchFreeCatalog();
      const merged = new Map<string, FreeProgram>();
      for (const item of builtinPrograms) merged.set(item.id, item);
      for (const item of remote.programs) merged.set(item.id, item);
      setPrograms([...merged.values()]);
      setSyncNote(`已同步远端目录（共 ${merged.size} 条，更新于 ${new Date(remote.fetchedAt).toLocaleString()}）`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSyncing(false);
    }
  };

  const registerInstance = (program: FreeProgram) => {
    if (!onRegister) return;
    const notes = [
      `免费渠道：${program.provider} · ${program.title}`,
      `有效期：${program.validity}`,
      program.keepAliveRule ? `保活/回收规则：${program.keepAliveRule}` : "",
      `计费风险：${program.billingRisk}`,
    ].filter(Boolean).join("\n");
    onRegister({
      name: `${program.provider} 免费实例`,
      provider: program.provider,
      cost: 0,
      currency: "USD",
      tags: ["free-tier", program.kind],
      notes,
    });
  };

  const filtered = programs.filter((program) => {
    if (kind && program.kind !== kind) return false;
    if (noCardOnly && program.requiresCard !== false) return false;
    if (query.trim()) {
      const hay = `${program.provider} ${program.title} ${program.summary} ${program.resources.join(" ")} ${program.regions.join(" ")}`.toLowerCase();
      if (!hay.includes(query.trim().toLowerCase())) return false;
    }
    return true;
  });

  return <>
    <div className="page-heading">
      <div>
        <h1>免费 VPS 雷达 2.0</h1>
        <p>收录 {programs.length} 条全球主流免费与学生算力渠道，看清额度、保活规则与计费风险，领取后一键登记为受管资产。</p>
      </div>
      <div className="heading-actions">
        <span className="table-meta">{syncNote}</span>
        <button className="btn" type="button" disabled={syncing} onClick={() => void syncRemote()}>
          <RefreshCw size={15} className={syncing ? "spin" : undefined} />{syncing ? "同步中" : "同步远端目录"}
        </button>
      </div>
    </div>
    <div className="risk flex items-start gap-3">
      <ShieldAlert size={20} className="shrink-0" />
      <p>免费不等于无费用。资格、额度和库存以服务商官方页面为准，请同时检查公网 IP、磁盘与出站流量费用，并在登记 VPS 时填好到期日以便「通知中心」提前提醒。</p>
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    <div className="filters" aria-label="免费类型筛选">
      <div className="search-wrap">
        <Search size={15} />
        <input className="input" type="search" placeholder="搜索厂商、规格或地区…" aria-label="搜索免费渠道" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <button className="btn" type="button" aria-pressed={kind === ""} onClick={() => setKind("")}>全部 ({programs.length})</button>
      {Object.entries(freeKindLabels).map(([value, label]) => (
        <button className="btn" type="button" key={value} aria-pressed={kind === value} onClick={() => setKind(value as FreeKind)}>{label}</button>
      ))}
      <label className="check-row">
        <input type="checkbox" checked={noCardOnly} onChange={(e) => setNoCardOnly(e.target.checked)} />
        仅看免信用卡
      </label>
    </div>
    {filtered.length === 0 ? (
      <div className="empty">
        <div>
          <h2>没有匹配的免费渠道</h2>
          <p>尝试放宽搜索关键词或取消「仅看免信用卡」筛选。</p>
          <button className="btn" type="button" onClick={() => { setKind(""); setNoCardOnly(false); setQuery(""); }}>重置筛选</button>
        </div>
      </div>
    ) : (
      <div className="free-list">
        {filtered.map((program) => (
          <article className="free-item" key={program.id}>
            <div>
              <div className="free-title">
                <h2>{program.provider}</h2>
                <span className={program.status === "verified" ? "badge badge-ok" : "badge"}>{program.status === "verified" ? "已核对来源" : "条款待复核"}</span>
                <span className="badge badge-cool">{freeKindLabels[program.kind]}</span>
                {program.requiresCard === false && <span className="badge badge-ok">免信用卡</span>}
              </div>
              <h3 className="mt-3">{program.title}</h3>
              <p className="free-summary">{program.summary}</p>
              <ul className="resource-list">{program.resources.map((resource) => <li key={resource}>{resource}</li>)}</ul>
              {program.keepAliveRule && (
                <p className="field-help mt-3"><strong>保活 / 回收规则：</strong>{program.keepAliveRule}</p>
              )}
              <h3 className="mt-4">申请与使用条件</h3>
              <ul className="condition-list">{program.conditions.map((condition) => <li key={condition}>{condition}</li>)}</ul>
              {program.claimSteps && program.claimSteps.length > 0 && (
                <details className="mt-3">
                  <summary className="table-meta">查看领取与避坑清单（{program.claimSteps.length} 步）</summary>
                  <ol className="condition-list mt-2">
                    {program.claimSteps.map((step) => <li key={step}>{step}</li>)}
                  </ol>
                </details>
              )}
            </div>
            <aside className="free-aside">
              <dl>
                <div><dt>适用地区</dt><dd>{program.regions.join("，")}</dd></div>
                <div><dt>信用卡</dt><dd>{program.requiresCard === null ? "以注册页面为准" : program.requiresCard ? "通常需要" : "无需信用卡"}</dd></div>
                <div><dt>有效期</dt><dd>{program.validity}</dd></div>
                <div><dt>来源核验</dt><dd><time>{program.lastVerified}</time></dd></div>
              </dl>
              <p className="risk">{program.billingRisk}</p>
              <div className="stack">
                <button className="btn btn-primary" type="button" onClick={() => void openExternal(program.sourceUrl)}>
                  查看官方条款<ArrowUpRight size={16} />
                </button>
                {onRegister && (
                  <button className="btn" type="button" onClick={() => registerInstance(program)}>
                    <Plus size={15} />登记为我的 VPS
                  </button>
                )}
              </div>
            </aside>
          </article>
        ))}
      </div>
    )}
  </>;
}
