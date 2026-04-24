import ContextMenu from './ContextMenu';

/**
 * Right-click menu for a chat username. Items: Set/Edit/Clear nickname,
 * Edit/Add note, Block/Unblock.
 *
 * Props:
 *   open, point ({ x, y }), user, metadata,
 *   onClose, onEditNickname, onEditNote, onToggleBlocked
 */
export default function UserCardContextMenu({
  open, point, user, metadata,
  onClose, onEditNickname, onEditNote, onToggleBlocked,
}) {
  if (!open || !point) return null;
  const displayName = user?.display_name || user?.login || 'this user';
  const blocked = !!metadata?.blocked;

  return (
    <ContextMenu x={point.x} y={point.y} onClose={onClose}>
      <ContextMenu.Item onClick={() => { onEditNickname?.(); onClose(); }}>
        {metadata?.nickname ? 'Edit nickname…' : 'Set nickname…'}
      </ContextMenu.Item>
      <ContextMenu.Item onClick={() => { onEditNote?.(); onClose(); }}>
        {metadata?.note ? 'Edit note…' : 'Add note…'}
      </ContextMenu.Item>
      <ContextMenu.Separator />
      <ContextMenu.Item
        danger={!blocked}
        onClick={() => { onToggleBlocked?.(); onClose(); }}
      >
        {blocked ? `Unblock ${displayName}` : `Block ${displayName}`}
      </ContextMenu.Item>
    </ContextMenu>
  );
}
