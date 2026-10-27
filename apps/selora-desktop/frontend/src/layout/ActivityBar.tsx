// Selora IDE — Phase Z2 activity bar (DESIGN §3.2, ARCHITECTURE §2.2).
//
// 48px wide, full workbench height, 1px right hairline. 28×28 pills with
// 16×16 icons, order files/search/git/problems/agent/workflows, settings
// pinned to the bottom. Includes sliding/fading active indicator, notification
// badges with pulse animations, hover tooltips with shortcut hints (omitted
// where no keybind exists — never a fake hint), and motion toggle.
//
// Phase 8 (ROADMAP §"Phase 8 — Workflows", task 8d) adds the Workflows
// activity: it is the same primary-sidebar route as the Z2 views, so the
// panel lives in `PrimarySidebar`. The route id is owned by the Phase 8
// protocol slice; until `SurfaceId` carries it, `WORKFLOWS_SURFACE` names it
// here (bridging to the existing `ToggleSurface` command) so nothing in the
// shell guesses a second identifier.
import { useState, useSyncExternalStore } from 'react';
import { getLayoutSnapshot, subscribeLayout, type LayoutState, layoutStore } from '../features/layout/layout-store';
import { getAppSnapshot, subscribeApp } from '../app/app-store';
import { agentStore } from '../features/agent/agent-store';
import { Icon, type IconName } from '../components/Icon';
import { Zap } from '../components/icons';
import type { BootstrapState, SurfaceId } from '../bridge/bridge';

type Activity = {
  surface: SurfaceId;
  label: string;
  icon: IconName;
  /** Empty when the view has no keybind yet — the tooltip then omits the hint. */
  shortcut: string;
};

/**
 * Phase 8 (8d): the Workflows primary-sidebar route. `SurfaceId` is bridge
 * owned (and outside this slice), so the id is asserted once, here, and
 * reused by the panel mount — one literal, not two.
 */
export const WORKFLOWS_SURFACE: SurfaceId = 'workflows' as SurfaceId;

const ACTIVITIES: Activity[] = [
  { surface: 'files', label: 'Files', icon: 'file', shortcut: 'Ctrl+Shift+E' },
  { surface: 'search', label: 'Search', icon: 'search', shortcut: 'Ctrl+Shift+F' },
  { surface: 'git', label: 'Git', icon: 'git-branch', shortcut: 'Ctrl+Shift+G' },
  { surface: 'problems', label: 'Problems', icon: 'error', shortcut: 'Ctrl+Shift+M' },
  { surface: 'agent', label: 'Agent', icon: 'chat', shortcut: 'Ctrl+L' },
  // No keybind ships with the panel slice (ROADMAP 8d is the panel, not the
  // keymap), so the tooltip shows the label alone.
  { surface: WORKFLOWS_SURFACE, label: 'Workflows', icon: 'play', shortcut: '' },
];

const SETTINGS: Activity = {
  surface: 'settings',
  label: 'Settings',
  icon: 'settings',
  shortcut: 'Ctrl+,',
};

export type ActivityBarProps = {
  viewMode?: 'agent' | 'split' | 'editor';
  onSelectViewMode?: (mode: 'agent' | 'split' | 'editor') => void;
  bootstrap?: BootstrapState;
};

type BadgeInfo = {
  count?: number;
  variant: 'error' | 'warn' | 'git' | 'success' | 'pulse';
  pulse?: boolean;
};

function ActivityButton({
  activity,
  selected,
  badge,
  onClick,
}: {
  activity: Activity;
  selected: boolean;
  badge?: BadgeInfo | null;
  onClick: () => void;
}) {
  const [hovered, setHovered] = useState(false);

  return (
    <div
      className="activity-item-wrapper"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <button
        type="button"
        className={`activity-button${selected ? ' is-selected' : ''}`}
        aria-pressed={selected}
        aria-label={activity.label}
        aria-keyshortcuts={activity.shortcut || undefined}
        title={activity.label}
        onClick={onClick}
      >
        <span className="activity-icon">
          <Icon name={activity.icon} />
        </span>
        {badge ? (
          <span
            className={`activity-badge is-${badge.variant}${badge.pulse ? ' is-pulse' : ''}`}
            aria-hidden="true"
          >
            {badge.count !== undefined ? (badge.count > 99 ? '99+' : badge.count) : '•'}
          </span>
        ) : null}
      </button>

      {/* Floating tooltip on hover */}
      {hovered && (
        <div className="activity-tooltip" role="tooltip" aria-hidden="true">
          <span className="activity-tooltip-label">{activity.label}</span>
          {activity.shortcut ? (
            <span className="activity-tooltip-key">{activity.shortcut}</span>
          ) : null}
        </div>
      )}
    </div>
  );
}

export function ActivityBar({ viewMode, onSelectViewMode, bootstrap: initialBootstrap }: ActivityBarProps = {}) {
  const layoutState: LayoutState = useSyncExternalStore(subscribeLayout, getLayoutSnapshot);
  const appState = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const agentState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);

  const [reducedMotion, setReducedMotion] = useState(() => {
    if (typeof document !== 'undefined') {
      return document.documentElement.dataset.reducedMotion === 'true';
    }
    return false;
  });

  const bootstrap = initialBootstrap ?? appState.bootstrap;
  const layout = layoutState.layout;
  if (!layout) return null;

  const handleActivityClick = (surface: SurfaceId) => {
    if (surface === 'agent') {
      // Phase 9a: the Agent activity opens the dedicated full-height workbench,
      // not the narrow split dock. The editor/split switch lives in the dock
      // header (and the title bar), so leaving is always one explicit click.
      onSelectViewMode?.('agent');
      if (!layout.agent_dock_open) void layoutStore.toggle('agent');
    } else if (surface === 'settings') {
      void layoutStore.toggle('settings');
    } else {
      if (viewMode === 'agent' && onSelectViewMode) {
        onSelectViewMode('split');
      }
      void layoutStore.toggle(surface);
    }
  };

  const toggleReducedMotion = () => {
    const next = !reducedMotion;
    setReducedMotion(next);
    if (typeof document !== 'undefined') {
      if (next) {
        document.documentElement.dataset.reducedMotion = 'true';
      } else {
        delete document.documentElement.dataset.reducedMotion;
      }
    }
  };

  // Badge calculations. A file with both slots (git `MM`) is one changed
  // file, so count the union rather than summing the two lists.
  const gitModifiedCount = new Set([
    ...(bootstrap?.git?.changed_paths ?? []),
    ...(bootstrap?.git?.staged_paths ?? []),
  ]).size;
  const diagnosticsErrors = bootstrap?.diagnostics?.filter((d) => d.severity === 'error').length ?? 0;
  const diagnosticsWarnings = bootstrap?.diagnostics?.filter((d) => d.severity === 'warning').length ?? 0;
  const totalProblems = diagnosticsErrors + diagnosticsWarnings;

  const agentInFlight = agentState.inFlight || agentState.thinkingState !== 'breathing';
  const agentWaitingApproval = Boolean(bootstrap?.chat?.pending_approval);
  const agentActiveTodos = agentState.todos?.filter((t) => t.status === 'in_progress').length ?? 0;

  const getBadgeForActivity = (surface: SurfaceId): BadgeInfo | null => {
    if (surface === 'git' && gitModifiedCount > 0) {
      return { count: gitModifiedCount, variant: 'git' };
    }
    if (surface === 'problems' && totalProblems > 0) {
      return {
        count: totalProblems,
        variant: diagnosticsErrors > 0 ? 'error' : 'warn',
        pulse: diagnosticsErrors > 0,
      };
    }
    if (surface === 'agent') {
      // Phase 9e: a finished background child outranks the running pulse -
      // the user needs to see that a result is waiting.
      if (agentState.unreadSubagentCount > 0) {
        return { count: agentState.unreadSubagentCount, variant: 'success' };
      }
      if (agentInFlight) {
        return { variant: 'pulse', pulse: true };
      }
      if (agentWaitingApproval) {
        return { count: 1, variant: 'warn', pulse: true };
      }
      if (agentActiveTodos > 0) {
        return { count: agentActiveTodos, variant: 'git' };
      }
    }
    return null;
  };

  return (
    <>
      {ACTIVITIES.map((activity) => (
        <ActivityButton
          key={activity.surface}
          activity={activity}
          badge={getBadgeForActivity(activity.surface)}
          selected={
            activity.surface === 'agent'
              ? viewMode === 'agent' || layout.agent_dock_open
              : layout.primary_sidebar_open && layout.activity_view === activity.surface
          }
          onClick={() => handleActivityClick(activity.surface)}
        />
      ))}

      <div className="activity-spacer" aria-hidden="true" />

      {/* Motion / Calm mode toggle (a real button with switch semantics) */}
      <div className="activity-item-wrapper">
        <button
          type="button"
          role="switch"
          aria-label="Toggle motion effects"
          aria-checked={!reducedMotion}
          className={`activity-motion-toggle ${reducedMotion ? 'is-calm' : 'is-active'}`}
          onClick={toggleReducedMotion}
          title={
            reducedMotion
              ? 'Calm Mode active (motion loops paused) — Click to enable animations'
              : 'Motion effects active — Click to enable Calm Mode'
          }
        >
          <Zap size={15} aria-hidden="true" />
        </button>
        <div className="activity-tooltip" role="tooltip" aria-hidden="true">
          <span className="activity-tooltip-label">
            {reducedMotion ? 'Calm Mode: On' : 'Motion: On'}
          </span>
          <span className="activity-tooltip-key">Toggle</span>
        </div>
      </div>

      <ActivityButton
        activity={SETTINGS}
        selected={layout.primary_sidebar_open && layout.activity_view === 'settings'}
        onClick={() => handleActivityClick('settings')}
      />
    </>
  );
}
