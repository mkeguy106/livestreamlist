import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

export default function NicknameDialog({ open, user, currentValue, onClose, onSave, onClear }) {
  const [val, setVal] = useState('');
  useEffect(() => {
    if (open) setVal(currentValue || '');
  }, [open, currentValue]);
  if (!open) return null;
  const handleSave = e => {
    e.preventDefault();
    const trimmed = val.trim();
    if (trimmed.length === 0) {
      onClear();
    } else {
      onSave(trimmed);
    }
  };
  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.55)',
        zIndex: 300, display: 'grid', placeItems: 'center',
      }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <form
        onSubmit={handleSave}
        style={{
          width: 340, background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)', borderRadius: 8,
          padding: 16, display: 'flex', flexDirection: 'column', gap: 12,
        }}
      >
        <div style={{ color: 'var(--zinc-200)', fontSize: 13 }}>
          Nickname for <strong>{user?.display_name || user?.login}</strong>
        </div>
        <input
          className="rx-input"
          autoFocus
          value={val}
          onChange={e => setVal(e.target.value)}
          placeholder="Empty to clear"
          maxLength={64}
        />
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose}>Cancel</button>
          <button type="submit" className="rx-btn rx-btn-primary">Save</button>
        </div>
      </form>
    </div>,
    document.body
  );
}
