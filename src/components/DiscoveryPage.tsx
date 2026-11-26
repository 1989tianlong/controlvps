import { useState } from "react";
import { CatalogOffers } from "./CatalogOffers";
import { DealFeedPanel } from "./DealFeedPanel";

type DiscoverTab = "live" | "catalog";

export function DiscoveryPage() {
  const [tab, setTab] = useState<DiscoverTab>("live");
  return <>
    <div className="page-heading"><div><h1>发现全球 VPS</h1><p>实时优惠抓取自站长社区，云商套餐是随应用发布的快照，两者都可以收藏留档。</p></div></div>
    <div className="tabs discover-tabs" role="tablist" aria-label="发现内容">
      <button className="tab" type="button" role="tab" aria-selected={tab === "live"} onClick={() => setTab("live")}>实时优惠</button>
      <button className="tab" type="button" role="tab" aria-selected={tab === "catalog"} onClick={() => setTab("catalog")}>云商套餐</button>
    </div>
    {tab === "live" ? <DealFeedPanel /> : <CatalogOffers />}
  </>;
}
