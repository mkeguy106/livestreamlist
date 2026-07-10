/* Backend switch for inline video: mpv (native surface — Linux, slice A)
 * vs mpegts.js in a <video> (macOS/Windows + browser-dev). All shared
 * behavior (settings shape, autoplay gating, per-channel persistence) lives
 * below this switch in the two implementations.
 */
import InlineVideo from './InlineVideo.jsx';
import MpvVideo from './MpvVideo.jsx';
import { useVideoBackend } from '../hooks/useVideoBackend.js';

export default function VideoPanel(props) {
  const backend = useVideoBackend();
  if (backend === 'mpv') return <MpvVideo {...props} />;
  if (backend === 'mpegts') return <InlineVideo {...props} />;
  return null; // backend probe in flight — first frames only
}
