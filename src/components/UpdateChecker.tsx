import { useCallback, useEffect, useRef } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { useEventsStore } from "../stores/eventsStore";

export default function UpdateChecker() {
  const checkedRef = useRef(false);
  const installingRef = useRef(false);
  const pushToast = useEventsStore((state) => state.pushToast);

  const installUpdate = useCallback(async () => {
    if (installingRef.current) return;

    installingRef.current = true;
    let update: Awaited<ReturnType<typeof check>> = null;

    try {
      pushToast("info", "Installing update...");
      update = await check();

      if (!update?.available) {
        pushToast("info", "No update available");
        return;
      }

      await update.downloadAndInstall();
      pushToast("info", "Update installed. Restarting...");
      await relaunch();
    } catch (error) {
      pushToast("error", error instanceof Error ? error.message : String(error));
    } finally {
      installingRef.current = false;
      await update?.close().catch(() => undefined);
    }
  }, [pushToast]);

  useEffect(() => {
    if (checkedRef.current) return;

    checkedRef.current = true;
    let cancelled = false;

    void check()
      .then(async (update) => {
        if (cancelled || !update?.available) {
          await update?.close().catch(() => undefined);
          return;
        }

        const version = update.version;
        await update.close().catch(() => undefined);

        if (cancelled) return;

        pushToast("info", `Update v${version} available`, {
          label: "Install & Restart",
          onClick: installUpdate,
        });
      })
      .catch((error) => {
        console.warn("Update check failed", error);
      });

    return () => {
      cancelled = true;
    };
  }, [installUpdate, pushToast]);

  return null;
}
