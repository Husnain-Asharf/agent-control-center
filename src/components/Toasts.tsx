import { useEffect } from 'react';
import { useStore } from '../store/store';
import type { Toast } from '../lib/types';
import { lampClass } from './Lamp';

const TOAST_MS = 3800;

function ToastItem({ toast }: { toast: Toast }) {
  const dismiss = useStore((s) => s.dismissToast);
  useEffect(() => {
    const t = setTimeout(() => dismiss(toast.id), TOAST_MS);
    return () => clearTimeout(t);
  }, [toast.id, dismiss]);
  const lamp = toast.kind === 'needs' ? 'lamp lamp--needs' : lampClass(toast.kind);
  return (
    <div className="toast">
      <span className={lamp} />
      <span className="toast-msg">{toast.msg}</span>
      {toast.actions && (
        <span className="toast-actions">
          {toast.actions.map((a, i) => (
            <button
              className={`btn${i === 0 ? ' btn--primary' : ''}`}
              type="button"
              key={a.label}
              onClick={() => {
                a.run();
                dismiss(toast.id);
              }}
            >
              {a.label}
            </button>
          ))}
        </span>
      )}
    </div>
  );
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
