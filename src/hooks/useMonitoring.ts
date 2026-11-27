import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import type { MonitoringSettings, Server, ServerMetrics } from "../types";

export function useMonitoring(
  servers: Server[],
  settings: MonitoringSettings | undefined,
  onMetrics: (serverId: string, metrics: ServerMetrics) => void,
) {
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [collecting, setCollecting] = useState(false);
  const metricsCallback = useRef(onMetrics);
  metricsCallback.current = onMetrics;
  const serversRef = useRef(servers);
  serversRef.current = servers;
  const serverIdsKey = servers.map((server) => server.id).join(",");

  useEffect(() => {
    const validIds = new Set(serverIdsKey ? serverIdsKey.split(",") : []);
    setErrors((current) => Object.fromEntries(Object.entries(current).filter(([id]) => validIds.has(id))));
  }, [serverIdsKey]);

  useEffect(() => {
    if (!settings?.autoCollect || !serverIdsKey) return;
    let stopped = false;
    let running = false;
    const collect = async () => {
      if (running) return;
      running = true;
      if (!stopped) setCollecting(true);
      const targets = serversRef.current;
      for (let offset = 0; offset < targets.length; offset += 2) {
        await Promise.all(targets.slice(offset, offset + 2).map(async (server) => {
          try {
            const result = await api.collectMetrics(server.id);
            if (!stopped) {
              metricsCallback.current(server.id, result);
              setErrors((current) => ({ ...current, [server.id]: "" }));
            }
          } catch (reason) {
            if (!stopped) setErrors((current) => ({ ...current, [server.id]: errorMessage(reason) }));
          }
        }));
        if (stopped) break;
      }
      running = false;
      if (!stopped) setCollecting(false);
    };
    void collect();
    const timer = window.setInterval(() => { void collect(); }, settings.intervalSeconds * 1000);
    return () => {
      stopped = true;
      setCollecting(false);
      window.clearInterval(timer);
    };
  }, [serverIdsKey, settings?.autoCollect, settings?.intervalSeconds]);

  return { errors, collecting };
}
