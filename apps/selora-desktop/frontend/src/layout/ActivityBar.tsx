// Selora IDE — Phase Z2 activity bar (DESIGN §3.2).
//
// 48px wide, full workbench height, 1px right hairline. 28×28 pills with
// 16×16 icons, order files/search/git/problems/agent, settings pinned to
// the bottom. Every button dispatches ToggleSurface (ARCHITECTURE §2.2's
// routing); the problems button targets the bottom dock's problems tab.
// Selected state: --ink-low pill + the 2px painted leading edge mark.
// Badge slots are inert until their data slices (Z4).
import { useSyncExternalStore } from 'react';
import { getLayoutSnapshot, subscribeLayout, type LayoutState } from '../features/layout/layout-store';
import { layoutStore } from '../features/layout/layout-store';
import { Icon, type IconName } from '../components/Icon';
import type { SurfaceId } from '../bridge/bridge';

type Activity = {
  surface: SurfaceId;
  label: string;
  icon: IconName;
};

const ACTIVITIES: Activity[] = [
  { surface: 'files', label: 'Files', icon: 'file' },
  { surface: 'search', label: 'Search', icon: 'search' },
  { surface: 'git', label: 'Git', icon: 'git-branch' },
  { surface: 'problems', label: 'Problems', icon: 'error' },
  { surface: 'agent', label: 'Agent', icon: 'chat' },
];

const SETTINGS: Activity = { surface: 'settings', label: 'Settings', icon: 'settings' };

function ActivityButton({ activity, selected }: { activity: Activity; selected: boolean }) {
  return (
    <button
      type="button"
      className={`activity-button${selected ? ' is-selected' : ''}`}
      aria-pressed={selected}
      aria-label={activity.label}
      title={activity.label}
      onClick={() => void layoutStore.toggle(activity.surface)}
    >
      <span className="activity-icon">
        <Icon name={activity.icon} />
      </span>
    </button>
  );
}

export function ActivityBar() {
  const state: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const layout = state.layout;
  if (!layout) return null;
  return (
    <>
      {ACTIVITIES.map((activity) => (
        <ActivityButton
          key={activity.surface}
          activity={activity}
          selected={
            activity.surface === 'agent' ? layout.agent_dock_open : layout.primary_sidebar_open && layout.activity_view === activity.surface
          }
        />
      ))}
      <div className="activity-spacer" aria-hidden="true" />
      <ActivityButton activity={SETTINGS} selected={layout.primary_sidebar_open && layout.activity_view === 'settings'} />
    </>
  );
}
