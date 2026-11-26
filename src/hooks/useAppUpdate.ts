import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "current" }
  | { status: "available"; version: string; notes: string }
  | { status: "downloading"; version: string; percent: number | null }
  | { status: "ready"; version: string }
  | { status: "error"; message: string };

function reasonText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function inDesktop(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export type AppUpdate = ReturnType<typeof useAppUpdate>;

export function useAppUpdate() {
  const [state, setState] = useState<UpdateState>({ status: "idle" });
  const [dismissed, setDismissed] = useState(false);
  const pending = useRef<Update | null>(null);
  const busy = useRef(false);

  const checkForUpdate = useCallback(async (interactive = false) => {
    if (!inDesktop() || busy.current) return;
    busy.current = true;
    setDismissed(false);
    setState({ status: "checking" });
    try {
      const update = await check({ timeout: 20_000 });
      pending.current = update;
      if (!update) {
        setState(interactive ? { status: "current" } : { status: "idle" });
      } else {
        setState({ status: "available", version: update.version, notes: update.body?.trim() ?? "" });
      }
    } catch (reason) {
      setState(interactive ? { status: "error", message: reasonText(reason) } : { status: "idle" });
    } finally {
      busy.current = false;
    }
  }, []);

  const install = useCallback(async () => {
    const update = pending.current;
    if (!update) return;
    setDismissed(false);
    setState({ status: "downloading", version: update.version, percent: null });
    try {
      let total = 0;
      let received = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
          setState({ status: "downloading", version: update.version, percent: total > 0 ? 0 : null });
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          setState({
            status: "downloading",
            version: update.version,
            percent: total > 0 ? Math.min(99, Math.round((received / total) * 100)) : null,
          });
        } else if (event.event === "Finished") {
          setState({ status: "downloading", version: update.version, percent: 100 });
        }
      });
      setState({ status: "ready", version: update.version });
    } catch (reason) {
      setState({ status: "error", message: reasonText(reason) });
    }
  }, []);

  const restart = useCallback(async () => {
    await relaunch();
  }, []);

  useEffect(() => {
    if (!import.meta.env.PROD) return;
    const timer = window.setTimeout(() => void checkForUpdate(), 4000);
    return () => window.clearTimeout(timer);
  }, [checkForUpdate]);

  return {
    state,
    dismissed,
    checkForUpdate,
    install,
    restart,
    dismiss: () => setDismissed(true),
  };
}
