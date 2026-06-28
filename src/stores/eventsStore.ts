import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import type { AlertEvent, ToastMessage } from "../types";

interface EventsState {
  events: AlertEvent[];
  toasts: ToastMessage[];
  error: string | null;
  loadEvents: () => Promise<void>;
  attachAlertListeners: () => Promise<() => void>;
  pushToast: (
    level: ToastMessage["level"],
    message: string,
    action?: ToastMessage["action"],
  ) => void;
  dismissToast: (toastId: string) => void;
}

const toastFromEvent = (event: AlertEvent): ToastMessage => ({
  id: event.id,
  level: event.level,
  message: event.message,
});

export const useEventsStore = create<EventsState>((set) => ({
  events: [],
  toasts: [],
  error: null,

  loadEvents: async () => {
    try {
      const events = await invoke<AlertEvent[]>("get_events");
      set((state) => {
        const knownIds = new Set(state.events.map((event) => event.id));
        const toastIds = new Set(state.toasts.map((toast) => toast.id));
        const newEvents = events.filter((event) => !knownIds.has(event.id));
        const newToasts = newEvents
          .filter((event) => !toastIds.has(event.id))
          .map(toastFromEvent);

        return {
          events,
          error: null,
          toasts: [...newToasts, ...state.toasts].slice(0, 5),
        };
      });
    } catch (error) {
      set({ error: error instanceof Error ? error.message : String(error) });
    }
  },

  attachAlertListeners: async () => {
    const unlistenEvent = await listen<AlertEvent>("alert-event", (event) => {
      const alert = event.payload;
      set((state) => ({
        events: [alert, ...state.events.filter((item) => item.id !== alert.id)].slice(0, 500),
        toasts: [toastFromEvent(alert), ...state.toasts].slice(0, 5),
      }));
    });
    const unlistenError = await listen<string>("alert-error", (event) => {
      const toast: ToastMessage = {
        id: crypto.randomUUID(),
        level: "error",
        message: event.payload,
      };
      set((state) => ({ toasts: [toast, ...state.toasts].slice(0, 5) }));
    });

    return () => {
      unlistenEvent();
      unlistenError();
    };
  },

  pushToast: (level, message, action) => {
    const toast: ToastMessage = {
      id: crypto.randomUUID(),
      level,
      message,
      action,
    };
    set((state) => ({ toasts: [toast, ...state.toasts].slice(0, 5) }));
  },

  dismissToast: (toastId) =>
    set((state) => ({
      toasts: state.toasts.filter((toast) => toast.id !== toastId),
    })),
}));
