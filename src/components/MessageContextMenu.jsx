import ContextMenu from './ContextMenu.jsx';

/**
 * Right-click menu for a chat message. Currently single-item (Reply) but
 * structured so future items (copy, pin, delete-as-mod, etc.) can be added
 * without touching every row component.
 *
 * Props:
 *   - x, y: viewport coordinates of the right-click
 *   - canReply: bool — false hides the Reply item
 *   - onReply: () => void
 *   - onClose: () => void
 */
export default function MessageContextMenu({ x, y, canReply, onReply, onClose }) {
  return (
    <ContextMenu x={x} y={y} onClose={onClose}>
      {canReply && (
        <ContextMenu.Item
          onClick={() => {
            onReply();
            onClose();
          }}
        >
          Reply
        </ContextMenu.Item>
      )}
    </ContextMenu>
  );
}
