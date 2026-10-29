// Selora IDE — title-bar subagent pet worlds.
//
// Every child of the active parent session gets one small world badge in the
// title bar. A child owns a deterministic world (stable FNV-1a hash of its id),
// so its pet keeps the same face across renders, transcript switches and
// reloads. Clicking a pet opens that child's session and brings the agent
// workbench forward. Purely decorative art lives in SVG/CSS — no emoji.
import { agentStore } from '../agent-store';
import type { SubagentInfo } from '../types';
import './subagent-pets.css';

export const PET_WORLDS = ['sun', 'moon', 'star', 'comet', 'nebula', 'orbit'] as const;
export type PetWorld = (typeof PET_WORLDS)[number];

export const PET_WORLD_LABEL: Record<PetWorld, string> = {
  sun: 'Sun',
  moon: 'Moon',
  star: 'Star',
  comet: 'Comet',
  nebula: 'Nebula',
  orbit: 'Orbit',
};

/**
 * FNV-1a (32-bit). Deterministic across runs and platforms: the same child id
 * always lands in the same world, with no `Math.random` and no clock.
 */
export function petWorldFor(childId: string): PetWorld {
  let hash = 0x811c9dc5;
  for (let index = 0; index < childId.length; index += 1) {
    hash ^= childId.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return PET_WORLDS[hash % PET_WORLDS.length]!;
}

/**
 * The pet's display label. The child's real session name wins; only when the
 * backend has not named it yet do we derive `Subagent <4 chars of id>`. No
 * completion or mission text is ever invented here.
 */
export function petLabel(child: Pick<SubagentInfo, 'id' | 'name'>): string {
  const name = typeof child.name === 'string' ? child.name.trim() : '';
  if (name) return name;
  const short = child.id.trim().slice(0, 4);
  return short ? `Subagent ${short}` : 'Subagent';
}

export const TERMINAL_SUBAGENT_STATUSES: readonly string[] = ['done', 'failed', 'cancelled'];

/** A child whose parent is the active session — never another session's child. */
export function petsForSession(
  subagents: readonly SubagentInfo[],
  activeSessionId: string,
): SubagentInfo[] {
  if (!activeSessionId) return [];
  return subagents.filter((child) => child.parentSessionId === activeSessionId);
}

function WorldArt({ world }: { world: PetWorld }) {
  switch (world) {
    case 'sun':
      return (
        <>
          <path
            className="pet-sun-rays"
            d="M12 2.4v2.3M12 19.3v2.3M2.4 12h2.3M19.3 12h2.3M5.3 5.3l1.6 1.6M17.1 17.1l1.6 1.6M18.7 5.3l-1.6 1.6M6.9 17.1l-1.6 1.6"
          />
          <circle cx="12" cy="12" r="4.3" />
        </>
      );
    case 'moon':
      return (
        <>
          <path d="M15.9 3.6a8.4 8.4 0 1 0 4.5 12.8A6.7 6.7 0 0 1 15.9 3.6Z" />
          <circle cx="17.6" cy="6.1" r="0.9" opacity="0.7" />
        </>
      );
    case 'star':
      return (
        <path
          className="pet-star"
          d="m12 3.3 2.1 5.3 5.7.5-4.3 3.8 1.3 5.7-4.8-3-4.8 3 1.3-5.7-4.3-3.8 5.7-.5Z"
        />
      );
    case 'comet':
      return (
        <>
          <circle cx="15.6" cy="8.4" r="3.1" />
          <path
            className="pet-comet-tail"
            d="M12.9 11.1 4.2 19.8M10.3 9.5 5.1 13.3M14.5 12.3l-4.2 5.2"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            opacity="0.75"
          />
        </>
      );
    case 'nebula':
      return (
        <g className="pet-nebula">
          <circle cx="9.4" cy="10.2" r="4.3" opacity="0.85" />
          <circle cx="15.1" cy="13.6" r="3.5" opacity="0.5" />
          <circle cx="13.6" cy="7.4" r="1.9" opacity="0.75" />
        </g>
      );
    case 'orbit':
      return (
        <>
          <circle cx="12" cy="12" r="2.6" />
          <g className="pet-orbit-ring">
            <ellipse
              cx="12"
              cy="12"
              rx="8.1"
              ry="3.4"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              opacity="0.8"
              transform="rotate(-24 12 12)"
            />
          </g>
        </>
      );
  }
}

export type SubagentPetsProps = {
  subagents: readonly SubagentInfo[];
  activeSessionId: string;
  activeSubagentId?: string | null;
  onSelectViewMode?: (mode: 'agent' | 'split' | 'editor') => void;
  /** Optional visible-pet cap; by default every pet is scrollable in the strip. */
  maxVisible?: number;
};

/**
 * Open a child's session through the store. The store action is additive in
 * this phase, so the call stays tolerant of an older snapshot shape; when it
 * lands it is the only way the workbench can show that child.
 */
function openSubagentSession(childId: string): void {
  const action = (agentStore as unknown as {
    openSubagentSession?: (id: string) => boolean;
  }).openSubagentSession;
  if (typeof action === 'function') action.call(agentStore, childId);
}

export function SubagentPets({
  subagents,
  activeSessionId,
  activeSubagentId = null,
  onSelectViewMode,
  maxVisible = Number.POSITIVE_INFINITY,
}: SubagentPetsProps) {
  const pets = petsForSession(subagents, activeSessionId);
  if (pets.length === 0) return null;

  const limit = Number.isFinite(maxVisible) ? Math.max(1, Math.floor(maxVisible)) : pets.length;
  const visible = pets.slice(0, limit);
  const hiddenCount = pets.length - visible.length;

  const activate = (childId: string) => {
    openSubagentSession(childId);
    onSelectViewMode?.('agent');
  };

  return (
    <div
      className="subagent-pets"
      role="group"
      aria-label="Subagent pets"
      onPointerDown={(event) => event.stopPropagation()}
      onWheel={(event) => {
        // The strip scrolls horizontally; a mouse wheel has no native horizontal
        // axis, so translate it. Every pet stays reachable without a scrollbar.
        if (event.deltaY !== 0) {
          event.currentTarget.scrollLeft += event.deltaY;
          event.preventDefault();
        }
      }}
    >
      {visible.map((child) => {
        const world = petWorldFor(child.id);
        const label = petLabel(child);
        const worldLabel = PET_WORLD_LABEL[world];
        const active = child.id === activeSubagentId;
        const terminal = TERMINAL_SUBAGENT_STATUSES.includes(child.status);
        const ariaLabel = `Open pet ${label} · ${worldLabel} · ${child.status}`;
        return (
          <button
            key={child.id}
            type="button"
            className={`subagent-pet is-${child.status} world-${world}${active ? ' is-active' : ''}`}
            data-world={world}
            data-status={child.status}
            aria-label={ariaLabel}
            aria-pressed={active}
            title={ariaLabel}
            onClick={() => activate(child.id)}
            onKeyDown={(event) => {
              if (event.key !== 'Enter' && event.key !== ' ') return;
              // Stop the native duplicate: the browser would also fire a click
              // for Enter/Space. One deliberate activation per key press.
              event.preventDefault();
              event.stopPropagation();
              activate(child.id);
            }}
          >
            <span className="subagent-pet-world" aria-hidden="true">
              <svg className={`subagent-pet-art art-${world}`} viewBox="0 0 24 24" focusable="false">
                <WorldArt world={world} />
              </svg>
              {terminal ? (
                <svg
                  className={`subagent-pet-mark mark-${child.status}`}
                  viewBox="0 0 12 12"
                  focusable="false"
                >
                  {child.status === 'done' ? (
                    <path d="m2.5 6.2 2.2 2.2 4.8-5" />
                  ) : (
                    <path d="m3 3 6 6M9 3 3 9" />
                  )}
                </svg>
              ) : null}
            </span>
            <span className="subagent-pet-name">{label}</span>
          </button>
        );
      })}
      {hiddenCount > 0 ? (
        <span
          className="subagent-pets-overflow"
          title={`${hiddenCount} more subagent ${hiddenCount === 1 ? 'pet' : 'pets'}`}
          aria-label={`${hiddenCount} more subagent ${hiddenCount === 1 ? 'pet' : 'pets'}`}
        >
          +{hiddenCount}
        </span>
      ) : null}
    </div>
  );
}