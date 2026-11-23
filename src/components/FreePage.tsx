import { useState } from "react";
import { ArrowUpRight, ShieldAlert } from "lucide-react";
import { freeKindLabels, freePrograms } from "../data/catalog";
import { openExternal } from "../lib/api";
import type { FreeKind } from "../types";

export function FreePage() {
  const [kind, setKind] = useState<FreeKind | "">("");
  const programs = freePrograms.filter((program) => !kind || program.kind === kind);
  return <>
    <div className="page-heading"><div><h1>免费 VPS 渠道</h1><p>看清额度、申请条件和计费风险，再前往官方渠道。</p></div></div>
    <div className="risk flex items-start gap-3"><ShieldAlert size={20} className="shrink-0" /><p>免费不等于无费用。资格、额度和库存以服务商官方页面为准，请同时检查公网 IP、磁盘与流量费用。</p></div>
    <div className="filters" aria-label="免费类型筛选">
      <button className="btn" type="button" aria-pressed={kind === ""} onClick={() => setKind("")}>全部渠道</button>
      {Object.entries(freeKindLabels).map(([value, label]) => <button className="btn" type="button" key={value} aria-pressed={kind === value} onClick={() => setKind(value as FreeKind)}>{label}</button>)}
    </div>
    {programs.length === 0 ? <div className="empty"><div><h2>此分类暂无收录</h2><p>选择其他免费类型查看已收录渠道。</p><button className="btn" type="button" onClick={() => setKind("")}>查看全部</button></div></div> : <div className="free-list">{programs.map((program) => <article className="free-item" key={program.id}>
      <div>
        <div className="free-title"><h2>{program.provider}</h2><span className={program.status === "verified" ? "badge badge-ok" : "badge"}>{program.status === "verified" ? "已核对来源" : "条款待复核"}</span><span className="badge badge-cool">{freeKindLabels[program.kind]}</span></div>
        <h3 className="mt-3">{program.title}</h3>
        <p className="free-summary">{program.summary}</p>
        <ul className="resource-list">{program.resources.map((resource) => <li key={resource}>{resource}</li>)}</ul>
        <h3 className="mt-6">申请与使用条件</h3>
        <ul className="condition-list">{program.conditions.map((condition) => <li key={condition}>{condition}</li>)}</ul>
      </div>
      <aside className="free-aside">
        <dl>
          <div><dt>适用地区</dt><dd>{program.regions.join("，")}</dd></div>
          <div><dt>信用卡</dt><dd>{program.requiresCard === null ? "以注册页面为准" : program.requiresCard ? "通常需要" : "通常不需要"}</dd></div>
          <div><dt>有效期</dt><dd>{program.validity}</dd></div>
          <div><dt>来源核验</dt><dd><time>{program.lastVerified}</time></dd></div>
        </dl>
        <p className="risk">{program.billingRisk}</p>
        <button className="btn btn-primary" type="button" onClick={() => openExternal(program.sourceUrl)}>查看官方条款<ArrowUpRight size={16} /></button>
      </aside>
    </article>)}</div>}
  </>;
}
