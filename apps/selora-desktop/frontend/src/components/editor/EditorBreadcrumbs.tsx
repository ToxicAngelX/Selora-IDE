// Selora IDE — Editor Breadcrumbs (DESIGN §3.4)
//
// Interactive hierarchical path navigation: workspace > folders > file > symbols.
// Includes clickable chevron dropdowns for rapid sibling switching and symbol exploration.
import { useState, useRef, useEffect, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import type { BreadcrumbSegment } from './editor-types';
import { ChevronRight, ChevronDown, Code2, FileText } from '../icons';

export type EditorBreadcrumbsProps = {
  segments: BreadcrumbSegment[];
  onSelectSibling?: (segmentId: string, siblingLabel: string) => void;
};

function SegmentKindIcon({ kind }: { kind: BreadcrumbSegment['kind'] }) {
  if (kind === 'workspace') {
    return (
      <svg className="breadcrumb-icon" width="12" height="12" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path d="M2 3H14V13H2V3Z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
        <path d="M6 3V13" stroke="currentColor" strokeWidth="1.1" />
      </svg>
    );
  }
  if (kind === 'dir') {
    return (
      <svg className="breadcrumb-icon" width="12" height="12" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path d="M2 3.5H6.5L8 5.5H14V12.5H2V3.5Z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
      </svg>
    );
  }
  if (kind === 'file') {
    return <FileText size={12} className="breadcrumb-icon" />;
  }
  // symbol
  return <Code2 size={12} className="breadcrumb-icon breadcrumb-icon-symbol" />;
}

export function EditorBreadcrumbs({ segments, onSelectSibling }: EditorBreadcrumbsProps) {
  const [openDropdownId, setOpenDropdownId] = useState<string | null>(null);
  const [menuFocusIndex, setMenuFocusIndex] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const closeDropdown = (segmentId: string) => {
    setOpenDropdownId(null);
    document.getElementById(`breadcrumb-trigger-${segmentId}`)?.focus();
  };

  // Move focus into the menu when it opens so Arrow keys work immediately.
  useEffect(() => {
    if (!openDropdownId) return;
    const items = menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]');
    (items?.[menuFocusIndex] ?? items?.[0])?.focus();
    // menuFocusIndex is set in the same batch that opens the menu.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openDropdownId]);

  useEffect(() => {
    const handleOutsideClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpenDropdownId(null);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        const openId = openDropdownId;
        setOpenDropdownId(null);
        if (openId) {
          const trigger = document.getElementById(`breadcrumb-trigger-${openId}`);
          if (trigger && document.activeElement?.closest('.breadcrumb-dropdown')) {
            trigger.focus();
          }
        }
      }
    };
    window.addEventListener('mousedown', handleOutsideClick);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('mousedown', handleOutsideClick);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [openDropdownId]);

  const handleMenuKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>, segment: BreadcrumbSegment) => {
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [],
    );
    if (items.length === 0) return;

    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeDropdown(segment.id);
      return;
    }

    if (event.key === 'Enter' || event.key === ' ') {
      const active = document.activeElement;
      if (active instanceof HTMLElement && items.includes(active)) {
        event.preventDefault();
        active.click();
      }
      return;
    }

    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement as HTMLElement);
    let next = current;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = items.length - 1;
    else if (event.key === 'ArrowDown') next = current < 0 ? 0 : (current + 1) % items.length;
    else next = current < 0 ? items.length - 1 : (current - 1 + items.length) % items.length;
    setMenuFocusIndex(next);
    items[next]?.focus();
  };

  return (
    <nav className="editor-breadcrumbs" aria-label="Breadcrumb navigation" ref={containerRef}>
      <ol className="breadcrumbs-list">
        {segments.map((segment, index) => {
          const isLast = index === segments.length - 1;
          const isDropdownOpen = openDropdownId === segment.id;
          const hasSiblings = segment.siblings && segment.siblings.length > 0;

          return (
            <li key={segment.id} className="breadcrumb-item">
              <div className="breadcrumb-segment-wrapper">
                <button
                  id={`breadcrumb-trigger-${segment.id}`}
                  type="button"
                  className={`breadcrumb-button${isLast ? ' is-current' : ''}`}
                  onClick={() => {
                    if (hasSiblings) {
                      if (isDropdownOpen) {
                        closeDropdown(segment.id);
                      } else {
                        const selectedIndex = segment.siblings!.indexOf(segment.label);
                        setMenuFocusIndex(selectedIndex >= 0 ? selectedIndex : 0);
                        setOpenDropdownId(segment.id);
                      }
                    }
                  }}
                  aria-expanded={isDropdownOpen}
                  aria-haspopup={hasSiblings ? 'menu' : undefined}
                  aria-controls={isDropdownOpen ? `breadcrumb-menu-${segment.id}` : undefined}
                >
                  <SegmentKindIcon kind={segment.kind} />
                  <span className="breadcrumb-label">{segment.label}</span>
                  {hasSiblings && (
                    <span className="breadcrumb-chevron-btn" aria-hidden="true">
                      {isDropdownOpen ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                    </span>
                  )}
                </button>

                {isDropdownOpen && hasSiblings && (
                  <div
                    ref={menuRef}
                    id={`breadcrumb-menu-${segment.id}`}
                    className="breadcrumb-dropdown"
                    role="menu"
                    aria-label={`${segment.label} siblings`}
                    onKeyDown={(event) => handleMenuKeyDown(event, segment)}
                  >
                    <div className="breadcrumb-dropdown-title">
                      <span>{segment.kind.toUpperCase()} SIBLINGS</span>
                    </div>
                    {segment.siblings!.map((sibling, siblingIndex) => (
                      <button
                        key={sibling}
                        type="button"
                        role="menuitem"
                        tabIndex={siblingIndex === menuFocusIndex ? 0 : -1}
                        className={`breadcrumb-dropdown-item${sibling === segment.label ? ' is-selected' : ''}`}
                        aria-current={sibling === segment.label ? 'true' : undefined}
                        onClick={() => {
                          onSelectSibling?.(segment.id, sibling);
                          closeDropdown(segment.id);
                        }}
                      >
                        <SegmentKindIcon kind={segment.kind} />
                        <span>{sibling}</span>
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {!isLast && (
                <span className="breadcrumb-separator" aria-hidden="true">
                  <ChevronRight size={10} />
                </span>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
