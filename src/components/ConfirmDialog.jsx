import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

/**
 * Generic confirmation modal. Backdrop-click and Esc cancel; Enter confirms.
 * Pass `danger` for a red confirm button (destructive actions like blocking).
 *
 * Props: open, title, body, confirmLabel, cancelLabel, danger, onConfirm, onClose
 */
export default function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  danger = false,
  onConfirm,
  onClose,
}) {
  const confirmRef = useRef(null);

  useEffect(() => {
    if (!open) return;
    confirmRef.current?.focus();
    const onKey = e => {
      if (e.key === 'Escape') onClose();
      else if (e.key === 'Enter') onConfirm();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose, onConfirm]);

  if (!open) return null;

  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.55)',
        zIndex: 320, display: 'grid', placeItems: 'center',
      }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        role="alertdialog"
        aria-label={title}
        style={{
          width: 340, background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)', borderRadius: 8,
          padding: 16, display: 'flex', flexDirection: 'column', gap: 10,
        }}
      >
        <div style={{ color: 'var(--zinc-100)', fontSize: 13, fontWeight: 600 }}>{title}</div>
        {body ? (
          <div style={{ color: 'var(--zinc-400)', font: '12px var(--font-sans)', lineHeight: 1.45 }}>
            {body}
          </div>
        ) : null}
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 4 }}>
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose}>
            {cancelLabel}
          </button>
          <button
            ref={confirmRef}
            type="button"
            className="rx-btn"
            onClick={onConfirm}
            style={
              danger
                ? {
                    background: 'rgba(239,68,68,.12)',
                    borderColor: 'rgba(239,68,68,.5)',
                    color: '#fca5a5',
                  }
                : undefined
            }
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
