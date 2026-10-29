// Selora IDE — ZCode Parity Hero Composer Card (REVAMP_SPEC §2.1, PHASE_Z §8.1, ZCode v4 parity)
//
// Sacred behavior:
// - Enter submits (when trimmed is non-empty).
// - Shift+Enter inserts newline.
// - Up/Down recalls prompt history on a single-line or empty buffer.
// - Smooth auto-expanding multiline input up to 8 rows.
// - BorderBeam rides perimeter during in-flight runs with dual-beam aurora glow.
// - Toolbar with Iconsax icons: permission mode dropdown, model switcher,
//   reasoning effort, Send/Stop.
//
// Honesty rules:
// - The model list is exactly the backend catalog (bootstrap.models /
//   models_changed). No fallback catalog, no invented context windows or
//   latencies. With no catalog we show a "Connect and refresh models" empty
//   state and disable send.
// - No workspace from Rust means the agent cannot read/write; the composer
//   explains that and disables send instead of inventing a folder.
// - Permission mode labels describe real session behavior: full = auto-apply
//   writes (bash still asks), ask = every write/bash asks, plan = read-only.
import React, { useState, useRef, useEffect, useCallback, useMemo, useSyncExternalStore } from 'react';
import {
  Plus,
  ShieldCheck,
  Lightbulb,
  ShieldAlert,
  ChevronDown,
  Sparkles,
  Zap,
  ArrowUp,
  Square,
  X,
  Check,
  Key,
  DirectSend,
  Search,
  Paperclip,
  FileText,
} from '../../../components/icons';
import { BorderBeam } from '../presence/BorderBeam';
import { VoiceBeam } from '../presence/VoiceBeam';
import type { FileTreeNode, Model } from '../../../bridge/bridge';
import type { PermissionMode, ReasoningEffort } from '../types';
import { agentStore, getComposerGate, resolveModelId } from '../agent-store';
import { activeEditorContext, editorStore } from '../../editor/editor-store';
import './agent-attachments.css';

export type AgentComposerProps = {
  inFlight: boolean;
  streamingActivityLevel: number;
  permissionMode: PermissionMode;
  reasoningEffort: ReasoningEffort;
  promptHistory: string[];
};

type MenuType = 'mode' | 'model' | 'effort' | null;

type ModelItem = {
  id: string;
  name: string;
  provider: string;
  colorClass: string;
};

const MODE_OPTIONS: Array<{
  id: PermissionMode;
  label: string;
  badge: string;
  description: string;
  icon: typeof ShieldCheck;
  colorClass: string;
}> = [
  {
    id: 'full',
    label: 'Full auto-apply',
    badge: 'Armed',
    description:
      'File writes, edits and deletes apply immediately for this session. Bash commands still ask for approval.',
    icon: ShieldCheck,
    colorClass: 'mode-icon-full',
  },
  {
    id: 'plan_only',
    label: 'Plan only',
    badge: 'Read-only',
    description: 'Explores and plans without writing any files.',
    icon: Lightbulb,
    colorClass: 'mode-icon-plan',
  },
  {
    id: 'ask_before_change',
    label: 'Ask before changes',
    badge: 'Guarded',
    description: 'Every file write and bash command pauses for your approval.',
    icon: ShieldAlert,
    colorClass: 'mode-icon-ask',
  },
];

const EFFORT_OPTIONS: Array<{ id: ReasoningEffort; label: string; desc: string }> = [
  { id: 'max', label: 'Max', desc: 'Thorough, multi-step chain of thought' },
  { id: 'standard', label: 'Standard', desc: 'Balanced speed and analysis' },
  { id: 'quick', label: 'Quick', desc: 'Fast, minimal thought overhead' },
];

/**
 * Phase 9a composer mode row. Labels are ZCode-class; each maps to a real
 * existing session policy rather than inventing a fourth backend mode.
 */
const COMPOSER_MODES: Array<{
  id: 'ask' | 'agent' | 'plan';
  label: string;
  mode: PermissionMode;
  description: string;
}> = [
  {
    id: 'ask',
    label: 'Ask',
    mode: 'ask_before_change',
    description: 'Ask before changes: every file write and command pauses for your approval.',
  },
  {
    id: 'agent',
    label: 'Agent',
    mode: 'full',
    description: 'Agent: file writes apply immediately for this session; commands still ask.',
  },
  {
    id: 'plan',
    label: 'Plan',
    mode: 'plan_only',
    description: 'Plan: explore read-only and propose a plan without writing files.',
  },
];

function composerModeId(mode: PermissionMode): 'ask' | 'agent' | 'plan' {
  if (mode === 'plan_only') return 'plan';
  if (mode === 'ask_before_change') return 'ask';
  return 'agent';
}

/**
 * Attachments are workspace-relative paths, not uploaded bytes. The backend
 * contract is explicit: `editor_context` never carries file contents and the
 * agent reads files itself with `read_file` (selora-protocol lib.rs). So the
 * prompt names the files and the agent reads the same bytes its tools see.
 */
const MAX_ATTACHMENTS = 8;
const ATTACHMENT_HEADER = 'Attached workspace files (read them with read_file before answering):';

function flattenFileTree(node: FileTreeNode | null, out: string[] = []): string[] {
  if (!node || out.length >= 5000) return out;
  if (node.kind === 'file') out.push(node.path);
  for (const child of node.children ?? []) flattenFileTree(child, out);
  return out;
}

const ATTACHMENTS_ONLY_PROMPT = 'Please review the attached workspace files.';

function composePrompt(text: string, attachments: readonly string[]): string {
  if (attachments.length === 0) return text;
  const intro = text.trim() ? text : ATTACHMENTS_ONLY_PROMPT;
  return `${intro}\n\n${ATTACHMENT_HEADER}\n${attachments.map((path) => `- ${path}`).join('\n')}`;
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** Provider label derived from the real backend model id (no fake metadata). */
function describeModel(model: Model): ModelItem {
  const idLower = `${model.id} ${model.display_name}`.toLowerCase();
  if (idLower.includes('claude') || idLower.includes('anthropic')) {
    return {
      id: model.id,
      name: model.display_name || model.id,
      provider: 'Anthropic',
      colorClass: 'provider-anthropic',
    };
  }
  if (
    idLower.includes('gpt') ||
    idLower.includes('openai') ||
    idLower.includes('o1') ||
    idLower.includes('o3')
  ) {
    return {
      id: model.id,
      name: model.display_name || model.id,
      provider: 'OpenAI',
      colorClass: 'provider-openai',
    };
  }
  if (idLower.includes('deepseek')) {
    return {
      id: model.id,
      name: model.display_name || model.id,
      provider: 'DeepSeek',
      colorClass: 'provider-deepseek',
    };
  }
  if (idLower.includes('glm') || idLower.includes('zhipu')) {
    return {
      id: model.id,
      name: model.display_name || model.id,
      provider: 'Zhipu',
      colorClass: 'provider-zhipu',
    };
  }
  if (idLower.includes('ollama') || idLower.includes('llama') || idLower.includes('qwen')) {
    return {
      id: model.id,
      name: model.display_name || model.id,
      provider: 'Local',
      colorClass: 'provider-custom',
    };
  }
  return {
    id: model.id,
    name: model.display_name || model.id,
    provider: 'Provider',
    colorClass: 'provider-custom',
  };
}

export const AgentComposer: React.FC<AgentComposerProps> = ({
  inFlight,
  streamingActivityLevel,
  permissionMode,
  reasoningEffort,
  promptHistory,
}) => {
  const storeState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const availableModels = storeState.availableModels;
  const selectedModelId = storeState.selectedModel;
  const gate = getComposerGate(storeState);

  const [text, setText] = useState('');
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedDraft, setSavedDraft] = useState('');
  const [isFocused, setIsFocused] = useState(false);
  const [activeMenu, setActiveMenu] = useState<MenuType>(null);
  const [modelSearch, setModelSearch] = useState('');
  const [modelError, setModelError] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<string[]>([]);
  const [attachOpen, setAttachOpen] = useState(false);
  const [attachQuery, setAttachQuery] = useState('');

  const editor = useSyncExternalStore(editorStore.subscribe, editorStore.getSnapshot);

  const workspaceFiles = useMemo(() => {
    const seen = new Set<string>();
    const options: Array<{ path: string; name: string; open: boolean }> = [];
    for (const tab of editor.tabs ?? []) {
      if (!tab.path || seen.has(tab.path)) continue;
      seen.add(tab.path);
      options.push({ path: tab.path, name: tab.filename, open: true });
    }
    for (const path of flattenFileTree(editor.fileTree ?? null)) {
      if (seen.has(path)) continue;
      seen.add(path);
      options.push({ path, name: baseName(path), open: false });
    }
    return options;
  }, [editor.tabs, editor.fileTree]);

  const filteredFiles = useMemo(() => {
    const query = attachQuery.trim().toLowerCase();
    const matches = query
      ? workspaceFiles.filter((file) => file.path.toLowerCase().includes(query))
      : workspaceFiles;
    return matches.slice(0, 80);
  }, [workspaceFiles, attachQuery]);

  const toggleAttachment = useCallback((path: string) => {
    setAttachments((current) => {
      if (current.includes(path)) return current.filter((entry) => entry !== path);
      if (current.length >= MAX_ATTACHMENTS) return current;
      return [...current, path];
    });
  }, []);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const modelSearchInputRef = useRef<HTMLInputElement>(null);
  const modelTriggerRef = useRef<HTMLButtonElement>(null);

  // Auto-resize textarea with smooth height transition
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    const computedHeight = Math.min(el.scrollHeight, 180);
    el.style.height = `${Math.max(computedHeight, 26)}px`;
  }, [text]);

  // Focus search input when model menu opens
  useEffect(() => {
    if (activeMenu !== 'model') {
      setModelSearch('');
      setModelError(null);
      return;
    }
    const timer = window.setTimeout(() => {
      modelSearchInputRef.current?.focus();
    }, 50);
    return () => window.clearTimeout(timer);
  }, [activeMenu]);

  // Click outside to close any open floating menus
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setActiveMenu(null);
        setAttachOpen(false);
      }
    };
    document.addEventListener('pointerdown', handleClickOutside);
    return () => document.removeEventListener('pointerdown', handleClickOutside);
  }, []);

  // Listen for draft restore events (e.g. from Rewind or Queue edit)
  useEffect(() => {
    const handleSetDraft = (e: Event) => {
      const customEvent = e as CustomEvent<{ text: string }>;
      if (customEvent.detail && typeof customEvent.detail.text === 'string') {
        setText(customEvent.detail.text);
        setTimeout(() => {
          if (textareaRef.current) {
            textareaRef.current.focus();
            textareaRef.current.selectionStart = textareaRef.current.value.length;
            textareaRef.current.selectionEnd = textareaRef.current.value.length;
          }
        }, 50);
      }
    };
    window.addEventListener('selora:set-composer-draft', handleSetDraft);
    return () => {
      window.removeEventListener('selora:set-composer-draft', handleSetDraft);
    };
  }, []);

  const handleRefreshModels = useCallback(() => {
    setModelError(null);
    void agentStore.refreshModels().catch((err: unknown) => {
      const message = err instanceof Error ? err.message : String(err ?? '');
      setModelError(message || 'Could not refresh models. Check the provider connection.');
    });
  }, []);

  const handleQueue = useCallback(() => {
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    agentStore.queuePrompt(composePrompt(trimmed, attachments), permissionMode);
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setAttachments([]);
  }, [gate.ready, text, permissionMode, attachments]);

  const handleSteer = useCallback(() => {
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    agentStore.steerActiveTurn(composePrompt(trimmed, attachments));
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setAttachments([]);
  }, [gate.ready, text, attachments]);

  const handleSubmit = useCallback(() => {
    if (inFlight) {
      if (!gate.ready) return;
      const trimmed = text.trim();
      if (trimmed || attachments.length > 0) {
        handleQueue();
      } else {
        agentStore.stopActiveTurn();
      }
      return;
    }
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    const prompt = composePrompt(trimmed, attachments);
    const context = activeEditorContext();
    if (context) agentStore.sendUserMessage(prompt, false, context);
    else agentStore.sendUserMessage(prompt);
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setAttachments([]);
  }, [inFlight, gate.ready, text, attachments, handleQueue]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter') {
      if (e.shiftKey) {
        // Shift+Enter inserts newline
        return;
      }
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) {
        // Ctrl+Enter or Cmd+Enter: Steer immediately!
        if (text.trim()) {
          handleSteer();
        }
        return;
      }
      if (inFlight) {
        if (!gate.ready) return;
        if (text.trim() || attachments.length > 0) {
          handleQueue();
        } else {
          agentStore.stopActiveTurn();
        }
      } else {
        handleSubmit();
      }
      return;
    }

    if (e.key === 'Escape') {
      if (activeMenu) {
        setActiveMenu(null);
        return;
      }
      if (inFlight) {
        e.preventDefault();
        agentStore.stopActiveTurn();
        return;
      }
    }

    // Up / Down arrow prompt history recall on single-line or empty input
    if (e.key === 'ArrowUp' && !text.includes('\n')) {
      if (promptHistory.length === 0) return;
      e.preventDefault();
      if (historyIndex === -1) {
        setSavedDraft(text);
        const nextIndex = 0;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      } else if (historyIndex < promptHistory.length - 1) {
        const nextIndex = historyIndex + 1;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      }
    } else if (e.key === 'ArrowDown' && !text.includes('\n')) {
      if (historyIndex > 0) {
        e.preventDefault();
        const nextIndex = historyIndex - 1;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      } else if (historyIndex === 0) {
        e.preventDefault();
        setHistoryIndex(-1);
        setText(savedDraft);
      }
    }
  };

  const currentMode =
    MODE_OPTIONS.find((m) => m.id === permissionMode) || MODE_OPTIONS[0];
  const CurrentModeIcon = currentMode.icon;

  const currentEffort =
    EFFORT_OPTIONS.find((e) => e.id === reasoningEffort) || EFFORT_OPTIONS[1];

  const autoApplyArmed = permissionMode === 'full';

  const resolvedModels: ModelItem[] = useMemo(
    () => availableModels.map(describeModel),
    [availableModels],
  );

  const filteredModels = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    if (!q) return resolvedModels;
    return resolvedModels.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider.toLowerCase().includes(q),
    );
  }, [resolvedModels, modelSearch]);

  const matchedModel = resolvedModels.find(
    (m) => m.id === selectedModelId || m.name === selectedModelId,
  );
  const sendableModelId = resolveModelId(storeState);
  const displayModelName = matchedModel
    ? matchedModel.name
    : sendableModelId ?? (availableModels.length === 0 ? 'No model' : 'Select a model');

  const sendBlocked = !inFlight && !gate.ready;
  const wordCount = text.trim() ? text.trim().split(/\s+/).length : 0;

  return (
    <div
      ref={containerRef}
      className={`agent-composer hero-composer-card ${isFocused ? 'is-focused' : ''} ${
        inFlight ? 'is-in-flight' : ''
      }`}
    >
      {/* 1. Border Beam (Animated perimeter aura during in-flight runs) */}
      <BorderBeam active={inFlight} durationMs={2400} borderRadius={8} colorPreset="emerald" />

      {/* 2. Composer mode row (Ask / Agent / Plan). Each entry is a real
          session permission policy — not a cosmetic toggle. */}
      <div className="composer-mode-row" role="group" aria-label="Composer mode">
        {COMPOSER_MODES.map((option) => {
          const active = composerModeId(permissionMode) === option.id;
          return (
            <button
              key={option.id}
              type="button"
              className={`composer-mode-btn${active ? ' is-active' : ''}`}
              aria-pressed={active}
              title={option.description}
              onClick={() => agentStore.setPermissionMode(option.mode)}
            >
              {option.label}
            </button>
          );
        })}
      </div>

      {/* 2b. Attached workspace files. Paths only: the agent reads them. */}
      {attachments.length > 0 && (
        <div className="agent-attach-chips" role="list" aria-label="Attached files">
          {attachments.map((path) => (
            <span className="agent-attach-chip" role="listitem" key={path} title={path}>
              <FileText size={11} aria-hidden="true" />
              <span className="agent-attach-chip-name">{baseName(path)}</span>
              <button
                type="button"
                className="agent-attach-chip-remove"
                aria-label={`Remove attachment ${baseName(path)}`}
                onClick={() => toggleAttachment(path)}
              >
                <X size={10} aria-hidden="true" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 3. Smooth Auto-Expanding Text Input Area */}
      <div className="composer-input-row" style={{ position: 'relative' }}>
        {!isFocused && !text && (
          <p
            className="agent-composer-placeholder"
            onClick={() => textareaRef.current?.focus()}
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              margin: 0,
              pointerEvents: 'none',
              cursor: 'text',
            }}
          >
            {gate.reason === 'no_workspace'
              ? 'Open a folder to let the agent read and write…'
              : gate.reason === 'no_model'
              ? 'Connect and refresh models to start…'
              : 'Ask anything…'}
          </p>
        )}
        <textarea
          ref={textareaRef}
          className="composer-textarea"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={() => setIsFocused(true)}
          onBlur={() => setIsFocused(false)}
          rows={1}
          aria-label="Composer prompt input"
          placeholder=""
        />
      </div>

      {/* 3. Blocked-send explanation (no workspace / no model) */}
      {!gate.ready && (
        <div
          className="composer-send-blocked"
          role="status"
          aria-live="polite"
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            gap: 6,
            margin: '0 10px 6px',
            padding: '6px 8px',
            fontSize: 11,
            lineHeight: 1.4,
            color: 'var(--fg-secondary, var(--c-a1a1aa))',
            background: 'var(--c-rgba-251-191-36-0p08)',
            border: '1px solid var(--c-rgba-251-191-36-0p28)',
            borderRadius: 6,
          }}
        >
          <ShieldAlert size={12} className="text-amber-400" aria-hidden="true" />
          <span>{gate.message}</span>
        </div>
      )}

      {/* 4. Floating Dropdowns with Smooth Scale-In Animation */}

      {/* Permission Mode Selector */}
      {activeMenu === 'mode' && (
        <div id="composer-mode-menu" className="composer-floating-menu mode-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">Permission Mode</div>
          {MODE_OPTIONS.map((opt) => {
            const Icon = opt.icon;
            const isSelected = opt.id === permissionMode;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                role="menuitem"
                onClick={() => {
                  agentStore.setPermissionMode(opt.id);
                  setActiveMenu(null);
                }}
              >
                <div className={`menu-icon-wrap ${opt.colorClass}`}>
                  <Icon size={14} />
                </div>
                <div className="menu-item-text">
                  <div className="item-title-row">
                    <span className="item-title">{opt.label}</span>
                    <span className="menu-mode-badge">{opt.badge}</span>
                  </div>
                  <div className="item-desc">{opt.description}</div>
                </div>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
        </div>
      )}

      {/* Model Selector Dropdown — backend catalog only */}
      {activeMenu === 'model' && (
        <div id="composer-model-menu" className="composer-floating-menu model-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">Inference Model &amp; Provider</div>

          {availableModels.length === 0 ? (
            <div className="model-no-results" role="status">
              <div className="item-title" style={{ color: 'var(--fg-default, var(--c-e4e4e7))' }}>
                Connect and refresh models
              </div>
              <div className="item-desc" style={{ marginTop: 4 }}>
                No models were reported by the backend. Connect a provider, then refresh
                the catalog.
              </div>
              <button
                type="button"
                className="floating-menu-item"
                role="menuitem"
                onClick={handleRefreshModels}
                style={{ justifyContent: 'center', marginTop: 6 }}
              >
                <Zap size={13} className="text-amber-400 flex-shrink-0" />
                <span className="item-title">Refresh models</span>
              </button>
              {modelError && (
                <div className="item-desc" style={{ marginTop: 6, color: 'var(--c-f87171)' }}>
                  {modelError}
                </div>
              )}
            </div>
          ) : (
            <>
              {/* Real-time Search Filter Input */}
              <div className="model-search-box">
                <Search size={13} className="model-search-icon" />
                <input
                  ref={modelSearchInputRef}
                  type="text"
                  className="model-search-input"
                  placeholder="Search models or providers..."
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key !== 'Escape') return;
                    e.preventDefault();
                    e.stopPropagation();
                    setActiveMenu(null);
                    modelTriggerRef.current?.focus();
                  }}
                  aria-label="Search models"
                />
                {modelSearch && (
                  <button
                    type="button"
                    className="model-search-clear"
                    onClick={() => setModelSearch('')}
                    title="Clear search"
                    aria-label="Clear search"
                  >
                    <X size={11} />
                  </button>
                )}
              </div>

              <div className="model-items-list">
                {filteredModels.length === 0 ? (
                  <div className="model-no-results">No matching models found</div>
                ) : (
                  filteredModels.map((m) => {
                    const isSelected =
                      selectedModelId === m.id ||
                      selectedModelId === m.name ||
                      sendableModelId === m.id;
                    return (
                      <button
                        key={m.id}
                        type="button"
                        className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                        role="menuitem"
                        onClick={() => {
                          agentStore.setSelectedModel(m.id);
                          setActiveMenu(null);
                        }}
                      >
                        <Sparkles size={13} className="text-emerald-400 flex-shrink-0" />
                        <div className="menu-item-text">
                          <div className="item-title-row">
                            <span className="item-title">{m.name}</span>
                            <span className={`provider-badge ${m.colorClass}`}>
                              {m.provider}
                            </span>
                          </div>
                          {m.id !== m.name && <div className="item-desc">{m.id}</div>}
                        </div>
                        {isSelected && <Check size={13} className="menu-check text-ok" />}
                      </button>
                    );
                  })
                )}
              </div>
            </>
          )}

          <div className="menu-divider" />
          <button
            type="button"
            className="floating-menu-item model-settings-item"
            role="menuitem"
            onClick={() => {
              window.dispatchEvent(new CustomEvent('selora:open-connect'));
              setActiveMenu(null);
            }}
          >
            <Key size={13} className="text-accent flex-shrink-0" />
            <div className="menu-item-text">
              <div className="item-title">Model &amp; Reasoning Settings…</div>
              <div className="item-desc">Configure keys, local Ollama endpoints, &amp; depth</div>
            </div>
          </button>
        </div>
      )}

      {/* Reasoning Depth Dropdown */}
      {activeMenu === 'effort' && (
        <div id="composer-effort-menu" className="composer-floating-menu effort-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">Reasoning Depth</div>
          {EFFORT_OPTIONS.map((opt) => {
            const isSelected = opt.id === reasoningEffort;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                role="menuitem"
                onClick={() => {
                  agentStore.setReasoningEffort(opt.id);
                  setActiveMenu(null);
                }}
              >
                <Zap size={13} className="text-amber-400 flex-shrink-0" />
                <div className="menu-item-text">
                  <div className="item-title">{opt.label}</div>
                  <div className="item-desc">{opt.desc}</div>
                </div>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
        </div>
      )}

      {/* 4b. Workspace file picker. Only real tree entries and open tabs. */}
      {attachOpen && (
        <div
          className="agent-attach-menu"
          role="dialog"
          aria-label="Attach workspace files"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation();
              setAttachOpen(false);
            }
          }}
        >
          <div className="agent-attach-menu-head">
            <span className="agent-attach-menu-title">Attach workspace files</span>
            <span className="agent-attach-menu-count">
              {attachments.length}/{MAX_ATTACHMENTS}
            </span>
          </div>
          <input
            className="agent-attach-search"
            value={attachQuery}
            onChange={(event) => setAttachQuery(event.target.value)}
            placeholder="Search workspace files…"
            aria-label="Search workspace files"
            autoFocus
          />
          <div className="agent-attach-list" role="listbox" aria-label="Workspace files" aria-multiselectable="true">
            {workspaceFiles.length === 0 ? (
              <p className="agent-attach-empty">Open a folder to attach workspace files.</p>
            ) : filteredFiles.length === 0 ? (
              <p className="agent-attach-empty">No files match “{attachQuery.trim()}”.</p>
            ) : (
              filteredFiles.map((file) => {
                const selected = attachments.includes(file.path);
                const atLimit = !selected && attachments.length >= MAX_ATTACHMENTS;
                return (
                  <button
                    key={file.path}
                    type="button"
                    role="option"
                    aria-selected={selected}
                    className={`agent-attach-file${selected ? ' is-selected' : ''}`}
                    title={file.path}
                    disabled={atLimit}
                    onClick={() => toggleAttachment(file.path)}
                  >
                    <FileText size={12} className="agent-attach-file-icon" aria-hidden="true" />
                    <span className="agent-attach-file-name">{file.name}</span>
                    <span className="agent-attach-file-path">{file.path}</span>
                    {file.open && <span className="agent-attach-file-open">open</span>}
                    {selected && <Check size={12} className="agent-attach-file-check" aria-hidden="true" />}
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}

      {/* 5. Bottom Toolbar */}
      <div className="composer-toolbar">
        <div className="toolbar-left">
          {/* Attach workspace files. Paths only, read by the agent's tools. */}
          <button
            type="button"
            className={`composer-pill attach-pill${attachOpen ? ' is-open' : ''}`}
            onClick={() => setAttachOpen((open) => !open)}
            disabled={workspaceFiles.length === 0}
            title={
              workspaceFiles.length === 0
                ? 'Open a folder to attach workspace files'
                : 'Attach workspace files for this prompt'
            }
            aria-label="Attach workspace files"
            aria-haspopup="dialog"
            aria-expanded={attachOpen}
          >
            <Paperclip size={13} />
            <span className="pill-text">Attach</span>
            {attachments.length > 0 && (
              <span className="agent-attach-count">{attachments.length}</span>
            )}
          </button>

          {/* Permission Mode pill (ZCode V4ComposerModeControls) */}
          <button
            type="button"
            className="composer-pill mode-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'mode' ? null : 'mode'))}
            title="Toggle permission policy"
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'mode'}
            aria-controls={activeMenu === 'mode' ? 'composer-mode-menu' : undefined}
          >
            <CurrentModeIcon size={13} className={currentMode.colorClass} />
            <span className="pill-text">{currentMode.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Armed auto-apply indicator — writes land without cards, bash asks */}
          {autoApplyArmed && (
            <span
              className="composer-auto-apply-armed"
              role="status"
              title="Full auto-apply is active: file writes, edits and deletes apply immediately for this session. Bash commands still require approval."
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 4,
                padding: '2px 6px',
                fontSize: 10,
                fontWeight: 600,
                letterSpacing: '0.02em',
                color: 'var(--c-34d399)',
                background: 'var(--c-rgba-16-185-129-0p12)',
                border: '1px solid var(--c-rgba-16-185-129-0p35)',
                borderRadius: 4,
              }}
            >
              <ShieldCheck size={11} aria-hidden="true" />
              Auto-apply armed
            </span>
          )}

          {/* Prompt word count (measured, not estimated) */}
          {text.trim() && (
            <span className="composer-token-pill" title="Word count for this prompt">
              {wordCount}w
            </span>
          )}
        </div>

        <div className="toolbar-right">
          {/* Model Selector Pill with Provider Indicator */}
          <button
            ref={modelTriggerRef}
            type="button"
            className="composer-pill model-pill"
            title="Active Model: click to switch or search"
            onClick={() => setActiveMenu((cur) => (cur === 'model' ? null : 'model'))}
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'model'}
            aria-controls={activeMenu === 'model' ? 'composer-model-menu' : undefined}
          >
            <span className="model-dot" />
            <span className="pill-text">{displayModelName}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Reasoning Effort Pill */}
          <button
            type="button"
            className="composer-pill effort-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'effort' ? null : 'effort'))}
            title="Reasoning effort"
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'effort'}
            aria-controls={activeMenu === 'effort' ? 'composer-effort-menu' : undefined}
          >
            <Zap size={12} className="effort-icon" />
            <span className="pill-text">{currentEffort.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* In-Flight Actions vs Send/Stop Button */}
          {inFlight && (text.trim() || attachments.length > 0) ? (
            <div className="composer-inflight-actions-group">
              {/* Steer button (Ctrl+Enter) */}
              <button
                type="button"
                className="composer-action-pill composer-steer-btn"
                onClick={handleSteer}
                disabled={!gate.ready}
                title="Steer active run (Ctrl+Enter): inject urgent course correction directive"
                aria-label="Steer active run"
              >
                <DirectSend size={12} className="btn-icon" />
                <span className="pill-text">Steer</span>
              </button>

              {/* Queue button (Enter) */}
              <button
                type="button"
                className="composer-action-pill composer-queue-btn"
                onClick={handleQueue}
                disabled={!gate.ready}
                title="Queue prompt (Enter): runs automatically when active turn completes"
                aria-label="Queue prompt"
              >
                <Plus size={12} className="btn-icon" />
                <span className="pill-text">Queue</span>
              </button>

              {/* Stop button (Esc) */}
              <button
                type="button"
                className="composer-send-btn is-stop"
                onClick={() => agentStore.stopActiveTurn()}
                title="Stop execution (Esc)"
                aria-label="Stop"
              >
                <Square size={12} />
              </button>
            </div>
          ) : (
            /* Functional Glowing Green Send / Morphing Stop Action Button */
            <button
              type="button"
              className={`composer-send-btn ${inFlight ? 'is-stop' : ''} ${
                (text.trim() || attachments.length > 0) && gate.ready ? 'has-text' : ''
              } ${sendBlocked ? 'is-blocked' : ''}`}
              onClick={handleSubmit}
              disabled={sendBlocked}
              aria-disabled={sendBlocked}
              title={
                sendBlocked
                  ? gate.message ?? 'Sending is unavailable'
                  : inFlight
                  ? 'Stop execution (Esc)'
                  : 'Send prompt (Enter)'
              }
              aria-label={inFlight ? 'Stop' : 'Send'}
            >
              {inFlight ? <Square size={12} /> : <ArrowUp size={14} />}
            </button>
          )}
        </div>
      </div>

      {/* 6. VoiceBeam AI activity meter at the bottom edge */}
      <VoiceBeam active={inFlight} level={streamingActivityLevel} />
    </div>
  );
};