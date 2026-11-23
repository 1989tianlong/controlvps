import { useMemo, useState } from "react";
import { ArrowUpRight, Check, Search, Star } from "lucide-react";
import { catalogOffers } from "../data/catalog";
import { openExternal } from "../lib/api";
import type { CatalogOffer } from "../types";

function readFavorites() {
  try { return JSON.parse(localStorage.getItem("controlvps.catalogFavorites") ?? "[]") as string[]; }
  catch { return []; }
}

export function DiscoveryPage() {
  const [query, setQuery] = useState("");
  const [region, setRegion] = useState("");
  const [maxPrice, setMaxPrice] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [favorites, setFavorites] = useState<string[]>(readFavorites);
  const [compare, setCompare] = useState<string[]>([]);
  const regions = useMemo(() => [...new Set(catalogOffers.flatMap((offer) => offer.regions))].sort((a, b) => a.localeCompare(b, "zh-CN")), []);
  const offers = catalogOffers.filter((offer) => {
    const text = `${offer.provider} ${offer.plan}`.toLowerCase();
    return text.includes(query.toLowerCase()) &&
      (!region || offer.regions.includes(region)) &&
      (!maxPrice || (offer.monthlyPrice !== null && offer.monthlyPrice <= Number(maxPrice))) &&
      (!favoritesOnly || favorites.includes(offer.id));
  }).sort((a, b) => (a.monthlyPrice ?? Number.MAX_VALUE) - (b.monthlyPrice ?? Number.MAX_VALUE));
  const compared = compare.map((id) => catalogOffers.find((offer) => offer.id === id)).filter((offer): offer is CatalogOffer => Boolean(offer));

  const toggleFavorite = (id: string) => {
    const next = favorites.includes(id) ? favorites.filter((item) => item !== id) : [...favorites, id];
    setFavorites(next);
    localStorage.setItem("controlvps.catalogFavorites", JSON.stringify(next));
  };
  const toggleCompare = (id: string) => {
    setCompare((current) => current.includes(id) ? current.filter((item) => item !== id) : current.length < 3 ? [...current, id] : current);
  };

  return <>
    <div className="page-heading"><div><h1>发现全球 VPS</h1><p>按配置、地区和月付价格筛选，数据来源链接到服务商官网。</p></div></div>
    <div className="filters">
      <div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索套餐" placeholder="搜索服务商或套餐" value={query} onChange={(e) => setQuery(e.target.value)} /></div>
      <select className="select" aria-label="筛选地区" value={region} onChange={(e) => setRegion(e.target.value)}><option value="">全部地区</option>{regions.map((item) => <option key={item}>{item}</option>)}</select>
      <input className="input w-36" type="number" min={0} inputMode="decimal" aria-label="最高月付价格" placeholder="最高价格" value={maxPrice} onChange={(e) => setMaxPrice(e.target.value)} />
      <button className="btn" type="button" aria-pressed={favoritesOnly} onClick={() => setFavoritesOnly((value) => !value)}><Star size={15} fill={favoritesOnly ? "currentColor" : "none"} />只看收藏</button>
    </div>
    <p className="mb-4 text-muted text-sm">目录为本地快照，价格不含税费且可能变化。点击官网核对实时配置与机房库存。</p>
    {offers.length === 0 ? <div className="empty"><div><Search size={30} /><h2>没有匹配的套餐</h2><p>放宽地区或价格条件后再试。</p><button className="btn" type="button" onClick={() => { setQuery(""); setRegion(""); setMaxPrice(""); setFavoritesOnly(false); }}>清除筛选</button></div></div> : <div className="table-wrap panel"><table className="data-table"><thead><tr><th>对比</th><th>服务商与套餐</th><th>配置</th><th>流量</th><th>月付</th><th>官网</th><th>收藏</th></tr></thead><tbody>{offers.map((offer) => <tr key={offer.id}>
      <td><input type="checkbox" aria-label={`对比 ${offer.provider} ${offer.plan}`} checked={compare.includes(offer.id)} disabled={compare.length >= 3 && !compare.includes(offer.id)} onChange={() => toggleCompare(offer.id)} /></td>
      <td><div className="catalog-provider"><span className="provider-mark" aria-hidden="true">{offer.provider.slice(0, 1)}</span><div><span className="table-name">{offer.provider}</span><span className="table-meta">{offer.plan}</span></div></div></td>
      <td><span className="measured">{offer.cpu === null ? "CPU 见官网" : `${offer.cpu} vCPU`} / {offer.memoryGb === null ? "内存见官网" : `${offer.memoryGb} GB`}</span><span className="table-meta">{offer.diskGb === null ? "存储见官网" : `${offer.diskGb} GB SSD`}</span></td>
      <td className="measured">{offer.transferTb === null ? "见官网" : `${offer.transferTb} TB`}</td>
      <td className="measured">{offer.monthlyPrice === null ? "动态报价" : `${offer.currency === "EUR" ? "€" : "$"}${offer.monthlyPrice}`}</td>
      <td><button className="icon-btn" type="button" aria-label={`打开 ${offer.provider} 官网`} title={`官网，快照 ${offer.updatedAt}`} onClick={() => openExternal(offer.sourceUrl)}><ArrowUpRight size={17} /></button></td>
      <td><button className="icon-btn" type="button" aria-label={`${favorites.includes(offer.id) ? "取消收藏" : "收藏"} ${offer.provider}`} aria-pressed={favorites.includes(offer.id)} onClick={() => toggleFavorite(offer.id)}><Star size={16} fill={favorites.includes(offer.id) ? "currentColor" : "none"} /></button></td>
    </tr>)}</tbody></table></div>}
    {compared.length > 0 && <div className="compare-bar"><span>已选择 {compared.length} 个套餐{compare.length >= 3 ? "，最多对比 3 个" : ""}</span><button className="btn" type="button" onClick={() => setCompare([])}>清除对比</button></div>}
    {compared.length > 1 && <section className="panel mt-6"><div className="panel-header"><h2>套餐对比</h2><Check size={18} className="text-cool" /></div><div className="table-wrap"><table className="data-table"><thead><tr><th>项目</th>{compared.map((offer) => <th key={offer.id}>{offer.provider}</th>)}</tr></thead><tbody>
      <tr><td>套餐</td>{compared.map((offer) => <td key={offer.id}>{offer.plan}</td>)}</tr>
      <tr><td>计算与内存</td>{compared.map((offer) => <td className="measured" key={offer.id}>{offer.cpu ?? "见官网"} / {offer.memoryGb === null ? "见官网" : `${offer.memoryGb} GB`}</td>)}</tr>
      <tr><td>月付快照</td>{compared.map((offer) => <td className="measured" key={offer.id}>{offer.monthlyPrice === null ? "动态报价" : `${offer.currency} ${offer.monthlyPrice}`}</td>)}</tr>
      <tr><td>虚拟化</td>{compared.map((offer) => <td key={offer.id}>{offer.virtualization}</td>)}</tr>
    </tbody></table></div></section>}
  </>;
}
