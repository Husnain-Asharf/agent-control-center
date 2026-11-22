import { useEffect, useRef, type FormEvent, type ReactNode } from 'react';

interface Props {
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Render as a form; called on submit. */
  onSubmit?: () => void;
  wide?: boolean;
}

/** Modal on the mockup's overlay. Escape or a click on the scrim closes it. */
export function Dialog({ title, onClose, children, onSubmit, wide }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  // Callers pass inline handlers. Keep the latest in a ref so the effect runs once.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    const el = ref.current;
    (el?.querySelector<HTMLElement>('[data-autofocus]') ?? el?.querySelector<HTMLElement>('.btn--primary'))?.focus();
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onCloseRef.current();
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      prev?.focus?.();
    };
  }, []);

  const cls = `dialog${wide ? ' dialog--wide' : ''}`;
  const body = (
    <>
      <h2 id="dialog-title">{title}</h2>
      {children}
    </>
  );
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={ref} role="dialog" aria-modal="true" aria-labelledby="dialog-title">
        {onSubmit ? (
          <form
            className={cls}
            onSubmit={(e: FormEvent) => {
              e.preventDefault();
              onSubmit();
            }}
          >
            {body}
          </form>
        ) : (
          <div className={cls}>{body}</div>
        )}
      </div>
    </div>
  );
}
