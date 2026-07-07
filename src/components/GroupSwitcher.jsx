/* Columns toolbar group switcher — replaces the static "Live now" chiclet.
 *
 * Trigger button shows the active group's name + a ▾ caret. The dropdown
 * pins "Live now" (non-deletable, non-renamable) first, then the user's
 * manual groups (double-click to rename inline, × to delete via
 * ConfirmDialog), then a "New group…" row that swaps to an inline input.
 *
 * Built as a standalone outside-click + Esc dropdown rather than importing
 * Command.jsx's `Dropdown` — that component isn't exported, and its rows
 * are plain labels with no per-row affordances (rename input, delete
 * button) or Esc-to-close handling, both of which this menu needs.
 *
 * Props: groups, activeId, onSwitch(id), onCreate(name), onRename(id,name),
 * onDelete(id).
 */
import { useEffect, useRef, useState } from 'react';
import ConfirmDialog from './ConfirmDialog.jsx';
import Tooltip from './Tooltip.jsx';

export default function GroupSwitcher({ groups, activeId, onSwitch, onCreate, onRename, onDelete }) {
  const [open, setOpen] = useState(false);
  const [renamingId, setRenamingId] = useState(null);
  const [renameValue, setRenameValue] = useState('');
  const [creating, setCreating] = useState(false);
  const [createValue, setCreateValue] = useState('');
  const [deleteTarget, setDeleteTarget] = useState(null); // { id, name } | null

  const containerRef = useRef(null);

  // Click-vs-double-click disambiguation for manual group rows. The DOM
  // always fires `click` (twice) before `dblclick`, so a naive `onClick`
  // handler would switch groups and close the menu on the double-click's
  // first click — closing the dropdown before the rename input could ever
  // mount. Debounce the click's effect briefly; `onDoubleClick` cancels the
  // pending switch and opens the rename input instead.
  const clickTimerRef = useRef(null);
  useEffect(() => () => {
    if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
  }, []);

  const activeGroup = groups.find((g) => g.id === activeId);
  const activeName =
    activeId === 'live-now' ? 'Live now' : activeGroup ? activeGroup.name : 'Choose group…';

  const closeMenu = () => {
    setOpen(false);
    setRenamingId(null);
    setCreating(false);
  };

  // Outside-click + Esc close, attached only while the menu is open — same
  // arm-while-open idiom as ColumnView's resize drag and Command.jsx's
  // DragResizeHandle.
  useEffect(() => {
    if (!open) return undefined;
    const onMouseDown = (e) => {
      if (containerRef.current && !containerRef.current.contains(e.target)) closeMenu();
    };
    const onKeyDown = (e) => {
      if (e.key === 'Escape' && deleteTarget === null) closeMenu();
    };
    document.addEventListener('mousedown', onMouseDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onMouseDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open, deleteTarget]);

  const beginRename = (g) => {
    setCreating(false); // mutually exclusive with the "New group…" input
    setRenamingId(g.id);
    setRenameValue(g.name);
  };
  const commitRename = () => {
    const name = renameValue.trim();
    if (name) onRename(renamingId, name);
    setRenamingId(null);
  };
  const cancelRename = (e) => {
    e.stopPropagation(); // don't also trigger the menu's Esc-close listener
    setRenamingId(null);
  };

  const beginCreate = () => {
    setRenamingId(null); // mutually exclusive with a group's rename input
    setCreating(true);
  };
  const commitCreate = () => {
    const name = createValue.trim();
    if (name) onCreate(name);
    setCreating(false);
    setCreateValue('');
    closeMenu();
  };
  const cancelCreate = (e) => {
    e.stopPropagation();
    setCreating(false);
    setCreateValue('');
  };

  return (
    <div ref={containerRef} style={{ position: 'relative' }}>
      <Tooltip text="Switch group">
        <button
          type="button"
          aria-label="Switch group"
          aria-haspopup="menu"
          onClick={() => (open ? closeMenu() : setOpen(true))}
          className="rx-btn rx-btn-ghost"
          style={{ display: 'inline-flex', alignItems: 'center', gap: 5, padding: '3px 8px' }}
        >
          <span
            className="rx-mono"
            style={{
              fontSize: 10,
              color: 'var(--zinc-300)',
              maxWidth: 160,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {activeName}
          </span>
          <span style={{ color: 'var(--zinc-500)', fontSize: 10, lineHeight: 1 }}>▾</span>
        </button>
      </Tooltip>

      {open && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            left: 0,
            marginTop: 4,
            width: 200,
            zIndex: 11,
            background: 'var(--zinc-925)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 4,
            boxShadow: '0 8px 24px rgba(0,0,0,.6), 0 0 0 1px rgba(255,255,255,.03)',
            padding: '3px 0',
          }}
        >
          {/* Pinned "Live now" — non-deletable, non-renamable. */}
          <MenuRow
            selected={activeId === 'live-now'}
            onClick={() => {
              onSwitch('live-now');
              closeMenu();
            }}
          >
            Live now
          </MenuRow>

          {groups.length > 0 && <Sep />}

          {groups.map((g) => (
            <div key={g.id}>
              {renamingId === g.id ? (
                <div style={{ padding: '3px 10px' }}>
                  <input
                    autoFocus
                    className="rx-input"
                    style={{ width: '100%' }}
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.stopPropagation();
                        commitRename();
                      } else if (e.key === 'Escape') {
                        cancelRename(e);
                      }
                    }}
                  />
                </div>
              ) : (
                <MenuRow
                  selected={activeId === g.id}
                  onClick={() => {
                    if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
                    // 400ms matches GTK's gtk-double-click-time default; avoids interpreting
                    // moderately-paced double-clicks as single-click-then-switch.
                    clickTimerRef.current = setTimeout(() => {
                      onSwitch(g.id);
                      closeMenu();
                      clickTimerRef.current = null;
                    }, 400);
                  }}
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    if (clickTimerRef.current) {
                      clearTimeout(clickTimerRef.current);
                      clickTimerRef.current = null;
                    }
                    beginRename(g);
                  }}
                  action={
                    <Tooltip text={`Delete group "${g.name}"`}>
                      <span
                        role="button"
                        aria-label={`Delete group "${g.name}"`}
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteTarget({ id: g.id, name: g.name });
                        }}
                        style={{
                          color: 'var(--zinc-500)',
                          cursor: 'pointer',
                          lineHeight: 1,
                          padding: '0 2px',
                        }}
                        onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--zinc-200)'; }}
                        onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--zinc-500)'; }}
                      >
                        ×
                      </span>
                    </Tooltip>
                  }
                >
                  {g.name}
                </MenuRow>
              )}
            </div>
          ))}

          <Sep />

          {creating ? (
            <div style={{ padding: '3px 10px' }}>
              <input
                autoFocus
                className="rx-input"
                style={{ width: '100%' }}
                placeholder="Group name…"
                value={createValue}
                onChange={(e) => setCreateValue(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.stopPropagation();
                    commitCreate();
                  } else if (e.key === 'Escape') {
                    cancelCreate(e);
                  }
                }}
              />
            </div>
          ) : (
            <MenuRow onClick={beginCreate}>
              <span style={{ color: 'var(--zinc-400)' }}>New group…</span>
            </MenuRow>
          )}
        </div>
      )}

      <ConfirmDialog
        open={deleteTarget != null}
        title="Delete group?"
        body={deleteTarget ? `Delete group "${deleteTarget.name}"? This does not remove the channels themselves.` : ''}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        danger
        onConfirm={() => {
          onDelete(deleteTarget.id);
          setDeleteTarget(null);
          closeMenu();
        }}
        onClose={() => setDeleteTarget(null)}
      />
    </div>
  );
}

function Sep() {
  return <div style={{ height: 1, background: 'var(--zinc-800)', margin: '3px 0' }} />;
}

function MenuRow({ children, selected, onClick, onDoubleClick, action }) {
  return (
    <div
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      style={{
        padding: '5px 10px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        fontSize: 'var(--t-12)',
        color: selected ? 'var(--zinc-100)' : 'var(--zinc-400)',
        background: selected ? 'var(--zinc-900)' : 'transparent',
        cursor: 'pointer',
      }}
      onMouseEnter={(e) => {
        if (!selected) e.currentTarget.style.background = 'var(--zinc-900)';
      }}
      onMouseLeave={(e) => {
        if (!selected) e.currentTarget.style.background = 'transparent';
      }}
    >
      <span
        style={{
          flex: 1,
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {children}
      </span>
      {action}
    </div>
  );
}
