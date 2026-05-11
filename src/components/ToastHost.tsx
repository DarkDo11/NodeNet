import { X } from "lucide-react";
import type { ToastMessage } from "../types";

interface ToastHostProps {
  toasts: ToastMessage[];
  onDismiss: (toastId: string) => void;
}

export default function ToastHost({ toasts, onDismiss }: ToastHostProps) {
  if (toasts.length === 0) return null;

  return (
    <div className="toast-host">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast ${toast.level}${toast.action ? " has-action" : ""}`}>
          <span>{toast.message}</span>
          {toast.action ? (
            <button
              className="toast-action"
              onClick={() => {
                void toast.action?.onClick();
              }}
            >
              {toast.action.label}
            </button>
          ) : null}
          <button className="toast-dismiss" onClick={() => onDismiss(toast.id)} title="Dismiss">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
