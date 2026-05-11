import { useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { useEventsStore } from "../stores/eventsStore";

export default function UpdateChecker() {
  const checkedRef = useRef(false);
  const pushToast = useEventsStore((state) => state.pushToast);

  useEffect(() => {
    if (checkedRef.current) return;

    checkedRef.current = true;
    let cancelled = false;

    void check()
      .then((update) => {
        if (cancelled || !update?.available) return;

        pushToast("info", `Update v${update.version} available`, {
          label: "Install & Restart",
          onClick: async () => {
            try {
              await update.downloadAndInstall();
            } catch (error) {
              pushToast("error", error instanceof Error ? error.message : String(error));
            } finally {
              await update.close().catch(() => undefined);
            }
          },
        });
      })
      .catch((error) => {
        console.warn("Update check failed", error);
      });

    return () => {
      cancelled = true;
    };
  }, [pushToast]);

  return null;
}
