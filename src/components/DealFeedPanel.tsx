import { useMemo, useState } from "react";
import { ArrowUpRight, Clock, RefreshCw, Search, Star } from "lucide-react";
import { useDealFeed } from "../hooks/useDealFeed";
import { openExternal } from "../lib/api";
import type { DealPeriod, VpsDeal } from "../types";

const FAVORITES_KEY = ["controlvps", "dealFavorites"].join(".");

const periodLabels: Record<DealPeriod, string> = {
  month: "月付",
  quarter: "季付",
  year: "年付",
  multi_year: "多年付",
  custom: "自定义周期",
};

function readFavorites(): string[] {
  try {
    return JSON.parse(localStorage.getItem(FAVORITES_KEY) ?? "[]") as string[];
  } catch {
    return [];
  }
}

function currencySymbol(currency: string) {
  return currency === "EUR" ? "€" : "$";
}

function amountText(value: number) {
  return value % 1 === 0 ? String(value) : value.toFixed(2);
}

function periodLabel(price: NonNullable<VpsDeal["price"]>) {
  if (price.period === "custom") return `${price.billingMonths} 个月`;
  return periodLabels[price.period] ?? "月付";
}

function dateText(value: string) {
  if (!value) return "时间未知";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

function skeletonRows() {
  return <div className="deal-list" aria-hidden="true">{[0, 1, 2].map((index) => <div className="deal-item" key={index}><div className="skeleton deal-skeleton" /><div className="skeleton deal-skeleton-short" /></div>)}</div>;
}

export function DealFeedPanel() {
  const { feed, loading, error, refresh } = useDealFeed();
  const [query, setQuery] = useState("");
  const [location, setLocation] = useState("");
  const [maxMonthly, setMaxMonthly] = useState("");
  const [minMemory, setMinMemory] = useState("");
  const [pricedOnly, setPricedOnly] = useState(false);
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [favorites, setFavorites] = useState<string[]>(readFavorites);
  const [sort, setSort] = useState<"new" | "price">("new");

  const locations = useMemo(() => [...new Set((feed?.deals ?? []).flatMap((deal) => deal.locations))].sort((a, b) => a.localeCompare(b, "zh-CN")), [feed]);
  const deals = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const limit = maxMonthly ? Number(maxMonthly) : null;
    const memory = minMemory ? Number(minMemory) : null;
    return (feed?.deals ?? []).filter((deal) => {
      const text = `${deal.title} ${deal.provider ?? ""} ${deal.summary}`.toLowerCase();
      return (!needle || text.includes(needle)) &&
        (!location || deal.locations.includes(location)) &&
        (!limit || (deal.price !== null && deal.price.monthlyEquivalent <= limit)) &&
        (!memory || (deal.memoryGb !== null && deal.memoryGb >= memory)) &&
        (!pricedOnly || deal.price !== null) &&
        (!favoritesOnly || favorites.includes(deal.id));
    }).sort((a, b) => sort === "price"
      ? (a.price?.monthlyEquivalent ?? Number.MAX_VALUE) - (b.price?.monthlyEquivalent ?? Number.MAX_VALUE)
      : b.publishedAt.localeCompare(a.publishedAt));
  }, [feed, query, location, maxMonthly, minMemory, pricedOnly, favoritesOnly, favorites, sort]);

  const toggleFavorite = (id: string) => {
    const next = favorites.includes(id) ? favorites.filter((item) => item !== id) : [...favorites, id];
    setFavorites(next);
    localStorage.setItem(FAVORITES_KEY, JSON.stringify(next));
  };

  const resetFilters = () => {
    setQuery("");
    setLocation("");
    setMaxMonthly("");
    setMinMemory("");
    setPricedOnly(false);
    setFavoritesOnly(false);
  };

  return <>
    <div className="filters">
      <div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索优惠" placeholder="搜索服务商、配置或关键词" value={query} onChange={(e) => setQuery(e.target.value)} /></div>
      <select className="select" aria-label="筛选机房地区" value={location} onChange={(e) => setLocation(e.target.value)}><option value="">全部地区</option>{locations.map((item) => <option key={item}>{item}</option>)}</select>
      <input className="input w-36" type="number" min={0} inputMode="decimal" aria-label="最高月均价格" placeholder="月均上限" value={maxMonthly} onChange={(e) => setMaxMonthly(e.target.value)} />
      <select className="select" aria-label="最小内存" value={minMemory} onChange={(e) => setMinMemory(e.target.value)}>
        <option value="">内存不限</option>
        <option value="1">≥ 1 GB</option>
        <option value="2">≥ 2 GB</option>
        <option value="4">≥ 4 GB</option>
        <option value="8">≥ 8 GB</option>
      </select>
      <select className="select" aria-label="排序方式" value={sort} onChange={(e) => setSort(e.target.value as "new" | "price")}>
        <option value="new">最新发布</option>
        <option value="price">月均最低</option>
      </select>
      <button className="btn" type="button" aria-pressed={pricedOnly} onClick={() => setPricedOnly((value) => !value)}>只看带价格</button>
      <button className="btn" type="button" aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly((value) => !value)}><Star size={15} fill={favoritesOnly ? "currentColor" : "none"} />只看收藏</button>
      <button className="btn" type="button" aria-busy={loading} disabled={loading} onClick={() => void refresh()}><RefreshCw size={15} className={loading ? "spin" : undefined} />{loading ? "抓取中" : "刷新"}</button>
    </div>
    <div className="deal-meta">
      <span><Clock size={14} />{feed ? `抓取于 ${dateText(feed.fetchedAt)}` : "尚未抓取"}</span>
      {(feed?.sources ?? []).map((source) => <span key={source.id} className={source.ok ? "badge badge-ok" : "badge badge-bad"} title={source.message ?? source.url}>{source.name} · {source.ok ? `${source.itemCount} 条` : "抓取失败"}</span>)}
      <span className="text-muted">价格与配置由帖子正文自动解析，实际以下单页为准。</span>
    </div>
    {error && <div className="error-banner" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void refresh()}>重试</button></div>}
    {!feed && loading && skeletonRows()}
    {!feed && !loading && !error && <div className="empty"><div><Search size={30} /><h2>还没有优惠数据</h2><p>点击刷新，从 LowEndBox 与 LowEndTalk 抓取最新 VPS 优惠。</p><button className="btn btn-primary" type="button" onClick={() => void refresh()}>立即抓取</button></div></div>}
    {feed && deals.length === 0 && <div className="empty"><div><Search size={30} /><h2>没有匹配的优惠</h2><p>放宽价格、内存或地区条件后再试。</p><button className="btn" type="button" onClick={resetFilters}>清除筛选</button></div></div>}
    {feed && deals.length > 0 && <div className="deal-list">{deals.map((deal) => <article className="deal-item" key={deal.id}>
      <div className="deal-main">
        <div className="deal-head">
          <span className="badge badge-cool">{deal.provider ?? deal.sourceName}</span>
          <span className="deal-source">{deal.sourceName}</span>
          <time className="deal-date" dateTime={deal.publishedAt}>{dateText(deal.publishedAt)}</time>
        </div>
        <button className="deal-title" type="button" onClick={() => void openExternal(deal.url)}>{deal.title}</button>
        {deal.summary && <p className="deal-summary">{deal.summary}</p>}
        <div className="deal-specs">
          {deal.memoryGb !== null && <span className="badge">{amountText(deal.memoryGb)} GB 内存</span>}
          {deal.diskGb !== null && <span className="badge">{amountText(deal.diskGb >= 1024 ? deal.diskGb / 1024 : deal.diskGb)} {deal.diskGb >= 1024 ? "TB" : "GB"} 存储</span>}
          {deal.cpuCores !== null && <span className="badge">{amountText(deal.cpuCores)} vCPU</span>}
          {deal.bandwidthTb !== null && <span className="badge">{amountText(deal.bandwidthTb)} TB 流量</span>}
          {deal.locations.map((item) => <span className="badge" key={item}>{item}</span>)}
        </div>
      </div>
      <aside className="deal-aside">
        <div className="deal-price">
          {deal.price ? <>
            <strong>{currencySymbol(deal.price.currency)}{amountText(deal.price.amount)}</strong>
            <span>{periodLabel(deal.price)} · 月均 {currencySymbol(deal.price.currency)}{amountText(deal.price.monthlyEquivalent)}</span>
          </> : <span className="deal-price-missing">价格见帖子</span>}
        </div>
        <div className="deal-actions">
          <button className="icon-btn" type="button" aria-label={`${favorites.includes(deal.id) ? "取消收藏" : "收藏"} ${deal.title}`} aria-pressed={favorites.includes(deal.id)} onClick={() => toggleFavorite(deal.id)}><Star size={16} fill={favorites.includes(deal.id) ? "currentColor" : "none"} /></button>
          <button className="icon-btn" type="button" aria-label="打开原帖" title="打开原帖" onClick={() => void openExternal(deal.url)}><ArrowUpRight size={17} /></button>
        </div>
      </aside>
    </article>)}</div>}
    {feed && deals.length > 0 && <p className="deal-foot">共 {deals.length} 条，来自站长社区公开 RSS。收藏仅保存在本机。</p>}
  </>;
}
