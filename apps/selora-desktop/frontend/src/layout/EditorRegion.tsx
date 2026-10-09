// Selora IDE — Phase Z2 editor region (DESIGN §3.4, ARCHITECTURE §8.1).
//
// The 32px tab bar chrome is reserved (tabs land in Z4); the canvas carries
// the no-workspace empty state: the 64px splash mark, "No folder open",
// the italic hint, Open Folder and Clone Repo both disabled with their
// later-slice captions (no FrontendCommand variant exists for either in
// Z2 — none is invented), and the shortcut chip row.
import { EmptyState, EmptyStateButton, KeyChip } from '../components/EmptyState';
import { SeloraMark } from '../components/SeloraMark';

export function EditorRegion() {
  return (
    <div className="editor-region">
      <div className="editor-tabbar" aria-hidden="true" />
      <div className="editor-canvas">
        <EmptyState
          heading="No folder open"
          hint={<em>Open a folder to start editing.</em>}
          mark={<SeloraMark size={64} />}
        >
          <EmptyStateButton disabled title="Workspace opening lands in a later slice">
            Open Folder
          </EmptyStateButton>
          <EmptyStateButton disabled title="Cloning a repository lands in a later slice">
            Clone Repo
          </EmptyStateButton>
          <div className="empty-chips" role="list" aria-label="Shell shortcuts">
            <KeyChip>Ctrl+B</KeyChip>
            <KeyChip>Ctrl+L</KeyChip>
            <KeyChip>Ctrl+J</KeyChip>
            <KeyChip>Ctrl+Shift+P</KeyChip>
          </div>
        </EmptyState>
      </div>
    </div>
  );
}
