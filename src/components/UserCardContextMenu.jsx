import ContextMenu from './ContextMenu';

/**
 * Right-click / "more" menu for a chat username. Items: Set/Edit nickname,
 * Add/Edit note. Blocking lives on the user card's own Block button (with
 * own-user protection), so it's intentionally not offered here.
 *
 * Props:
 *   open, point ({ x, y }), user, metadata,
 *   onClose, onEditNickname, onEditNote
 */
export default function UserCardContextMenu({
  open, point, metadata,
  onClose, onEditNickname, onEditNote,
}) {
  if (!open || !point) return null;

  return (
    <ContextMenu x={point.x} y={point.y} onClose={onClose}>
      <ContextMenu.Item onClick={() => { onEditNickname?.(); onClose(); }}>
        {metadata?.nickname ? 'Edit nickname…' : 'Set nickname…'}
      </ContextMenu.Item>
      <ContextMenu.Item onClick={() => { onEditNote?.(); onClose(); }}>
        {metadata?.note ? 'Edit note…' : 'Add note…'}
      </ContextMenu.Item>
    </ContextMenu>
  );
}
