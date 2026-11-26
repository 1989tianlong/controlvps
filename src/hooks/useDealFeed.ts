import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import type { DealFeed } from "../types";

const CACHE_KEY = ["controlvps", "dealFeed", "v1"].join(".");
const MAX_AGE_MS = 30 * 60 * 1000;

function readCache(): DealFeed | null {
  try {
    const raw = localStorage.getItem(CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as DealFeed;
    return Array.isArray(parsed.deals) ? parsed : null;
  } catch {
    return null;
  }
}

export function useDealFeed() {
  const [feed, setFeed] = useState<DealFeed | null>(readCache);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const busy = useRef(false);

  const refresh = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    setLoading(true);
    setError("");
    try {
      const next = await api.fetchVpsDeals();
      setFeed(next);
      try {
        localStorage.setItem(CACHE_KEY, JSON.stringify(next));
      } catch {
        // 缓存失败不影响本次展示
      }
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      busy.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const cached = readCache();
    const stale = !cached || Date.now() - Date.parse(cached.fetchedAt) > MAX_AGE_MS;
    if (stale) void refresh();
  }, [refresh]);

  return { feed, loading, error, refresh };
}
