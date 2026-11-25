import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { sendSystemNotification } from "../lib/notifications";
import type { ServiceCheckResult, ServiceMonitorView } from "../types";

export interface ServiceMonitoringController {
  views: ServiceMonitorView[];
  loading: boolean;
  checkingIds: string[];
  errors: Record<string, string>;
  loadError: string;
  refresh: () => Promise<void>;
  check: (id: string) => Promise<ServiceCheckResult | undefined>;
}

export function useServiceMonitoring(notificationsEnabled: boolean): ServiceMonitoringController {
  const [views, setViews] = useState<ServiceMonitorView[]>([]);
  const [loading, setLoading] = useState(true);
  const [checkingIds, setCheckingIds] = useState<string[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loadError, setLoadError] = useState("");
  const viewsRef = useRef<ServiceMonitorView[]>([]);
  const running = useRef(new Set<string>());
  const notify = useRef(notificationsEnabled);
  notify.current = notificationsEnabled;

  const replace = useCallback((view: ServiceMonitorView) => {
    setViews((current) => {
      const exists = current.some((item) => item.monitor.id === view.monitor.id);
      const next = exists
        ? current.map((item) => item.monitor.id === view.monitor.id ? view : item)
        : [...current, view].sort((left, right) => left.monitor.name.localeCompare(right.monitor.name, "zh-CN"));
      viewsRef.current = next;
      return next;
    });
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await api.listServiceMonitors();
      viewsRef.current = next;
      setViews(next);
      setLoadError("");
    } catch (reason) {
      setLoadError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  const check = useCallback(async (id: string) => {
    if (running.current.has(id)) return undefined;
    running.current.add(id);
    setCheckingIds([...running.current]);
    try {
      const result = await api.checkServiceMonitor(id);
      replace(result.view);
      setErrors((current) => ({ ...current, [id]: result.notificationError ?? "" }));
      for (const event of result.events) {
        if (!notify.current) continue;
        const recovered = event.status === "recovered";
        void sendSystemNotification(
          `ControlVPS · ${recovered ? "服务恢复" : "服务告警"}`,
          `${event.monitorName}：${event.message}`,
        ).catch(() => undefined);
      }
      return result;
    } catch (reason) {
      setErrors((current) => ({ ...current, [id]: errorMessage(reason) }));
      return undefined;
    } finally {
      running.current.delete(id);
      setCheckingIds([...running.current]);
    }
  }, [replace]);

  useEffect(() => {
    let stopped = false;
    const tick = async () => {
      if (stopped) return;
      const now = Date.now();
      const due = viewsRef.current.filter(({ monitor, latest }) => {
        if (!monitor.enabled || running.current.has(monitor.id)) return false;
        const checkedAt = latest ? Date.parse(latest.checkedAt) : 0;
        return !Number.isFinite(checkedAt) || now - checkedAt >= monitor.intervalSeconds * 1000;
      });
      for (let offset = 0; offset < due.length; offset += 4) {
        await Promise.all(due.slice(offset, offset + 4).map((view) => check(view.monitor.id)));
        if (stopped) return;
      }
    };
    void refresh().then(tick);
    const timer = window.setInterval(() => { void tick(); }, 5000);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [check, refresh]);

  return { views, loading, checkingIds, errors, loadError, refresh, check };
}
