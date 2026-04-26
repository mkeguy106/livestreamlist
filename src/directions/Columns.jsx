/* Direction B — "Columns"
 * Temporarily disabled. The previous TweetDeck-style live-column
 * layout is being redesigned, so for now this view renders no
 * channels and mounts no chat tasks — keeping the layout switcher
 * functional without keeping the old behavior alive.
 */

export default function Columns({ ctx }) {
  const { openAddDialog } = ctx;

  return (
    <>
      <div
        style={{
          height: 36,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '0 12px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        <button type="button" className="rx-btn" onClick={openAddDialog}>＋ Add channel</button>
        <div style={{ flex: 1 }} />
        <span className="rx-chiclet">redesign in progress</span>
      </div>

      <div
        style={{
          flex: 1,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexDirection: 'column',
          gap: 6,
          color: 'var(--zinc-500)',
          fontSize: 'var(--t-12)',
        }}
      >
        <div>Columns view is being redesigned.</div>
        <div className="rx-chiclet">Use Command (A) or Focus (C) for now.</div>
      </div>

      <div
        style={{
          height: 24,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          borderTop: 'var(--hair)',
          gap: 12,
          flexShrink: 0,
        }}
      >
        <span className="rx-chiclet">columns: paused</span>
      </div>
    </>
  );
}
