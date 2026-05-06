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
        <div key={toast.id} className={`toast ${toast.level}`}>
          <span>{toast.message}</span>
          <button onClick={() => onDismiss(toast.id)} title="Dismiss">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
