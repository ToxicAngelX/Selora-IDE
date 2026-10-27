// Selora IDE — Editor Tab Bar (DESIGN §3.4, ARCHITECTURE §8.1)
//
// Multi-file tabs with filetype icons, dirty-state indicators, close buttons,
// and the active tab's bottom hairline. Tabs mirror the protocol's open buffer
// list; creation and splitting stay out of the chrome until the core supports
// them.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { X } from '../icons';

export type EditorTabView = {
  id: string;
  filename: string;
  path: string;
  language: string;
  isDirty: boolean;
};

export type EditorTabBarProps = {
  tabs: EditorTabView[];
  activeTabId: string | null;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  /**
   * Save one buffer and resolve `false` when the save failed. Dirty tabs never
   * close until this resolves successfully.
   */
  onSaveTab: (id: string) => Promise<boolean | void> | boolean | void;
};

type ClosePrompt = {
  id: string;
  top: number;
  left: number;
};

function FileTypeIcon({ language, filename }: { language: string; filename: string }) {
  if (filename.endsWith('.rs') || language === 'rust') {
    return (
      <svg className="tab-file-icon tab-icon-rust" width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <circle cx="8" cy="8" r="6.5" stroke="currentColor" strokeWidth="1.2" strokeDasharray="2 1.5" />
        <path d="M6 5.5H8.5C9.3 5.5 10 6.1 10 7C10 7.8 9.4 8.4 8.6 8.5L10.2 11H8.6L7.2 8.7H6V11H4.8V5.5H6ZM6 6.6V7.7H8.4C8.8 7.7 9 7.4 9 7.1C9 6.8 8.7 6.6 8.4 6.6H6Z" fill="currentColor" />
      </svg>
    );
  }
  if (filename.endsWith('.tsx') || filename.endsWith('.ts') || language === 'typescript') {
    return (
      <svg className="tab-file-icon tab-icon-tsx" width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <rect x="2" y="2" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="1.2" />
        <path d="M4.5 6H7.5M6 6V11.5" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" />
        <path d="M8.5 10.5C9 11.2 9.8 11.5 10.7 11.1C11.5 10.7 11.4 9.8 10.6 9.4L9.8 9C9 8.6 8.9 7.8 9.6 7.3C10.3 6.9 11.2 7.2 11.5 7.8" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" />
      </svg>
    );
  }
  if (filename.endsWith('.css') || language === 'css') {
    return (
      <svg className="tab-file-icon tab-icon-css" width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path d="M3 2.5L4 13.5L8 14.5L12 13.5L13 2.5H3Z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
        <path d="M5.5 5.5H10.5L10 8.5H5.8L6.1 11.5L8 12.1L9.9 11.5L10 10.2" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  return (
    <svg className="tab-file-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path d="M3 2H9.5L13 5.5V14H3V2Z" stroke="currentColor" strokeWidth="1.2" />
      <path d="M9 2V6H13" stroke="currentColor" strokeWidth="1.2" />
    </svg>
  );
}

export function EditorTabBar({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onSaveTab,
}: EditorTabBarProps) {
  const [closePrompt, setClosePrompt] = useState<ClosePrompt | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);
  const promptRef = useRef<HTMLDivElement>(null);
  const saveButtonRef = useRef<HTMLButtonElement>(null);
  const promptTab = closePrompt ? tabs.find((tab) => tab.id === closePrompt.id) ?? null : null;

  const selectAndFocus = (id: string) => {
    onSelectTab(id);
    // Roving tabindex: move focus with the selection so the next arrow key
    // is handled by the newly active tab instead of the stale one.
    document.getElementById(`editor-tab-${id}`)?.focus();
  };

  const dismissClosePrompt = (focusTab: boolean) => {
    if (saving) return;
    const tabId = closePrompt?.id;
    setClosePrompt(null);
    setSaving(false);
    setSaveFailed(false);
    if (focusTab && tabId) selectAndFocus(tabId);
  };

  const requestClose = (id: string) => {
    const tab = tabs.find((candidate) => candidate.id === id);
    if (!tab) return;
    if (!tab.isDirty) {
      setClosePrompt(null);
      setSaving(false);
      setSaveFailed(false);
      onCloseTab(id);
      return;
    }

    // Anchor the confirmation under the tab's close target. `fixed` keeps it
    // outside the strip's overflow clipping.
    const anchor = document.getElementById(`editor-tab-${id}`)?.getBoundingClientRect();
    const viewportWidth = typeof window === 'undefined' ? 1024 : window.innerWidth;
    setSaving(false);
    setSaveFailed(false);
    setClosePrompt({
      id,
      top: anchor && anchor.height > 0 ? anchor.bottom + 4 : 40,
      left: anchor && anchor.width > 0 ? Math.max(8, Math.min(anchor.left, viewportWidth - 190)) : 8,
    });
  };

  const confirmDiscard = () => {
    if (!closePrompt || saving) return;
    const id = closePrompt.id;
    setClosePrompt(null);
    setSaving(false);
    setSaveFailed(false);
    onCloseTab(id);
  };

  const confirmSave = async () => {
    if (!closePrompt || saving) return;
    const id = closePrompt.id;
    setSaving(true);
    setSaveFailed(false);
    let saved = false;
    try {
      saved = (await onSaveTab(id)) !== false;
    } catch {
      saved = false;
    }
    if (!saved) {
      setSaving(false);
      setSaveFailed(true);
      return;
    }
    setSaving(false);
    setClosePrompt(null);
    onCloseTab(id);
  };

  // Move focus into the prompt so Escape and the actions are keyboard reachable.
  useEffect(() => {
    if (!closePrompt) return;
    saveButtonRef.current?.focus();
  }, [closePrompt]);

  useEffect(() => {
    if (!closePrompt) return;
    const handleOutsidePointer = (event: MouseEvent) => {
      if (saving) return;
      const target = event.target as Node | null;
      if (!target || promptRef.current?.contains(target)) return;
      setClosePrompt(null);
      setSaving(false);
      setSaveFailed(false);
    };
    window.addEventListener('mousedown', handleOutsidePointer);
    return () => window.removeEventListener('mousedown', handleOutsidePointer);
  }, [closePrompt, saving]);

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>, tabId: string, index: number) => {
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      const nextTab = tabs[(index + 1) % tabs.length];
      if (nextTab) selectAndFocus(nextTab.id);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      const prevTab = tabs[(index - 1 + tabs.length) % tabs.length];
      if (prevTab) selectAndFocus(prevTab.id);
    } else if (e.key === 'w' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      requestClose(tabId);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      selectAndFocus(tabId);
    } else if (e.key === 'Home') {
      e.preventDefault();
      const first = tabs[0];
      if (first) selectAndFocus(first.id);
    } else if (e.key === 'End') {
      e.preventDefault();
      const last = tabs[tabs.length - 1];
      if (last) selectAndFocus(last.id);
    }
  };

  return (
    <div className="editor-tabbar">
      <div className="editor-tab-strip" role="tablist" aria-label="Editor tabs">
        {tabs.map((tab, idx) => {
          const isActive = tab.id === activeTabId;
          return (
            <div
              key={tab.id}
              className={`editor-tab-shell${isActive ? ' is-active' : ''}`}
            >
              <button
                id={`editor-tab-${tab.id}`}
                type="button"
                role="tab"
                aria-selected={isActive}
                aria-label={tab.filename}
                aria-controls="editor-surface"
                tabIndex={isActive ? 0 : -1}
                className={`editor-tab${isActive ? ' is-active' : ''}${tab.isDirty ? ' is-dirty' : ''}`}
                onClick={() => selectAndFocus(tab.id)}
                onKeyDown={(e) => handleKeyDown(e, tab.id, idx)}
              >
                <FileTypeIcon language={tab.language} filename={tab.filename} />
                <span className="tab-title">{tab.filename}</span>
                {tab.isDirty ? (
                  <span className="tab-dirty-indicator" title="Unsaved changes" aria-hidden="true">
                    ●
                  </span>
                ) : null}
              </button>
              <button
                type="button"
                className="tab-close-btn"
                aria-label={`Close ${tab.filename}`}
                title={`Close ${tab.filename}`}
                onClick={(e) => {
                  e.stopPropagation();
                  requestClose(tab.id);
                }}
              >
                <X size={12} />
              </button>
              {isActive && <div className="tab-active-glow" aria-hidden="true" />}
            </div>
          );
        })}
      </div>
      {closePrompt && promptTab ? (
        <div
          ref={promptRef}
          className="tree-context-menu"
          role="dialog"
          aria-label={`Unsaved changes in ${promptTab.filename}`}
          tabIndex={-1}
          style={{ top: closePrompt.top, left: closePrompt.left }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              dismissClosePrompt(true);
            }
          }}
        >
          <div className="breadcrumb-dropdown-title">
            {saveFailed ? `${promptTab.filename} could not be saved` : 'Unsaved changes'}
          </div>
          <button
            ref={saveButtonRef}
            type="button"
            className="context-menu-item"
            onClick={() => void confirmSave()}
            disabled={saving}
          >
            Save
          </button>
          <button
            type="button"
            className="context-menu-item is-danger"
            onClick={confirmDiscard}
            disabled={saving}
          >
            Discard
          </button>
          <button
            type="button"
            className="context-menu-item"
            onClick={() => dismissClosePrompt(true)}
            disabled={saving}
          >
            Cancel
          </button>
        </div>
      ) : null}
    </div>
  );
}
