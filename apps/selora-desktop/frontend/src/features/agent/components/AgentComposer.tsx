// Selora IDE — ZCode Parity Hero Composer Card (REVAMP_SPEC §2.1, PHASE_Z §8.1, ZCode v4 parity)
//
// Sacred behavior:
// - Enter submits (when trimmed is non-empty).
// - Shift+Enter inserts newline.
// - Up/Down recalls prompt history on a single-line or empty buffer.
// - Auto-expanding multiline input up to 8 rows.
// - BorderBeam rides perimeter during in-flight runs.
// - Toolbar with Lucide icons (ZCode V4ComposerToolbar & V4ComposerModeControls):
//   Context (+), Permission mode dropdown, Context count badge, Model switcher, Reasoning effort, Send/Stop.
// - No emoji slop anywhere: 100% clean vector Lucide iconography and tokens.
import React, { useState, useRef, useEffect, useCallback } from 'react';
import {
  Plus,
  ShieldCheck,
  Lightbulb,
  ShieldAlert,
  ChevronDown,
  Paperclip,
  Sparkles,
  Zap,
  ArrowUp,
  Square,
  X,
  FileText,
  Code2,
  ImageIcon,
  Check,
  Key,
  DirectSend,
} from '../../../components/icons';
import { BorderBeam } from '../presence/BorderBeam';
import { VoiceBeam } from '../presence/VoiceBeam';
import type { PermissionMode, ReasoningEffort } from '../types';
import { agentStore } from '../agent-store';

export type AgentComposerProps = {
  inFlight: boolean;
  streamingActivityLevel: number;
  permissionMode: PermissionMode;
  reasoningEffort: ReasoningEffort;
  selectedModel: string;
  promptHistory: string[];
};

type MenuType = 'mode' | 'model' | 'effort' | 'context' | null;

interface ComposerAttachment {
  id: string;
  name: string;
  type: 'file' | 'code' | 'image';
}

const MODE_OPTIONS: Array<{
  id: PermissionMode;
  label: string;
  description: string;
  icon: typeof ShieldCheck;
  colorClass: string;
}> = [
  {
    id: 'full',
    label: 'Full access',
    description: 'Agent executes tools and applies file edits directly',
    icon: ShieldCheck,
    colorClass: 'mode-icon-full',
  },
  {
    id: 'plan_only',
    label: 'Plan only',
    description: 'Generates plans before modifying any project files',
    icon: Lightbulb,
    colorClass: 'mode-icon-plan',
  },
  {
    id: 'ask_before_change',
    label: 'Ask before changes',
    description: 'Requires explicit user approval for each file write',
    icon: ShieldAlert,
    colorClass: 'mode-icon-ask',
  },
];

const MODEL_OPTIONS = [
  { id: 'GLM-5.3 CodeFast', provider: 'Zhipu / Local Rust', badge: 'Fast' },
  { id: 'Claude 3.7 Sonnet', provider: 'Anthropic', badge: 'Hybrid' },
  { id: 'DeepSeek V3', provider: 'DeepSeek', badge: 'Reasoning' },
  { id: 'GPT-4o', provider: 'OpenAI', badge: 'General' },
];

const EFFORT_OPTIONS: Array<{ id: ReasoningEffort; label: string; desc: string }> = [
  { id: 'max', label: 'Max', desc: 'Thorough, multi-step chain of thought' },
  { id: 'standard', label: 'Standard', desc: 'Balanced speed and analysis' },
  { id: 'quick', label: 'Quick', desc: 'Fast, minimal thought overhead' },
];

export const AgentComposer: React.FC<AgentComposerProps> = ({
  inFlight,
  streamingActivityLevel,
  permissionMode,
  reasoningEffort,
  selectedModel,
  promptHistory,
}) => {
  const [text, setText] = useState('');
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedDraft, setSavedDraft] = useState('');
  const [isFocused, setIsFocused] = useState(false);
  const [activeMenu, setActiveMenu] = useState<MenuType>(null);
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([
    { id: 'att-1', name: 'src/main.rs', type: 'file' },
  ]);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  // Click outside to close any open floating menus
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setActiveMenu(null);
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

  const handleQueue = useCallback(() => {
    const trimmed = text.trim();
    if (!trimmed) return;
    agentStore.queuePrompt(trimmed, permissionMode);
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
  }, [text, permissionMode]);

  const handleSteer = useCallback(() => {
    const trimmed = text.trim();
    if (!trimmed) return;
    agentStore.steerActiveTurn(trimmed);
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
  }, [text]);

  const handleSubmit = useCallback(() => {
    if (inFlight) {
      const trimmed = text.trim();
      if (trimmed) {
        handleQueue();
      } else {
        agentStore.stopActiveTurn();
      }
      return;
    }
    const trimmed = text.trim();
    if (!trimmed) return;
    agentStore.sendUserMessage(trimmed);
    setText('');
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
  }, [inFlight, text, handleQueue]);

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
        if (text.trim()) {
          handleQueue();
        } else {
          agentStore.stopActiveTurn();
        }
      } else {
        handleSubmit();
      }
      return;
    }

    if (e.key === 'Escape' && inFlight) {
      e.preventDefault();
      agentStore.stopActiveTurn();
      return;
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

  const customModelId = agentStore.getSnapshot().modelSettings.customModelId.trim();
  const availableModels = agentStore.getSnapshot().availableModels;
  const matchedModel = availableModels.find((m) => m.id === selectedModel);
  const displayModelName = customModelId ? `Custom: ${customModelId}` : matchedModel ? (matchedModel.display_name || matchedModel.id) : selectedModel;

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  };

  const addAttachment = (type: 'file' | 'code' | 'image', name: string) => {
    setAttachments((prev) => [
      ...prev,
      { id: `att-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`, name, type },
    ]);
    setActiveMenu(null);
  };

  return (
    <div
      ref={containerRef}
      className={`agent-composer hero-composer-card ${isFocused ? 'is-focused' : ''} ${inFlight ? 'is-in-flight' : ''}`}
    >
      {/* 1. Border Beam (ZCode animated glow that rides perimeter during runs) */}
      <BorderBeam active={inFlight} durationMs={2400} borderRadius={8} />

      {/* 2. Top Context Attachment Chips (if any exist) */}
      {attachments.length > 0 && (
        <div className="composer-attachments-row" aria-label="Attached context">
          {attachments.map((att) => (
            <span key={att.id} className="composer-attachment-chip">
              {att.type === 'file' ? (
                <FileText size={12} className="chip-icon text-muted" />
              ) : att.type === 'code' ? (
                <Code2 size={12} className="chip-icon text-accent" />
              ) : (
                <ImageIcon size={12} className="chip-icon text-info" />
              )}
              <span className="chip-name">{att.name}</span>
              <button
                type="button"
                className="chip-remove-btn"
                onClick={() => removeAttachment(att.id)}
                title={`Remove ${att.name}`}
                aria-label={`Remove ${att.name}`}
              >
                <X size={10} />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 3. Text Input Area */}
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
            Ask anything…
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

      {/* 4. Floating Menus for ZCode Controls */}
      {activeMenu === 'mode' && (
        <div className="composer-floating-menu mode-menu" role="menu">
          <div className="floating-menu-header">Permission Mode</div>
          {MODE_OPTIONS.map((opt) => {
            const Icon = opt.icon;
            const isSelected = opt.id === permissionMode;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                onClick={() => {
                  agentStore.setPermissionMode(opt.id);
                  setActiveMenu(null);
                }}
              >
                <Icon size={14} className={opt.colorClass} />
                <div className="menu-item-text">
                  <div className="item-title">{opt.label}</div>
                  <div className="item-desc">{opt.description}</div>
                </div>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
        </div>
      )}

      {activeMenu === 'model' && (
        <div className="composer-floating-menu model-menu" role="menu">
          <div className="floating-menu-header">Inference Model</div>
          {(agentStore.getSnapshot().availableModels.length > 0
            ? agentStore.getSnapshot().availableModels.map((m) => ({
                id: m.id,
                name: m.display_name || m.id,
                provider: m.available ? 'Connected' : 'Unavailable',
                badge: m.id.toLowerCase().includes('flash') || m.id.toLowerCase().includes('mini')
                  ? 'Fast'
                  : m.id.toLowerCase().includes('sonnet') || m.id.toLowerCase().includes('gpt-4') || m.id.toLowerCase().includes('pro')
                  ? 'Smart'
                  : 'Ready',
              }))
            : MODEL_OPTIONS.map((m) => ({
                id: m.id,
                name: m.id,
                provider: m.provider,
                badge: m.badge,
              }))
          ).map((m) => {
            const isSelected = selectedModel === m.id || selectedModel === m.name;
            return (
              <button
                key={m.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                onClick={() => {
                  agentStore.setSelectedModel(m.id);
                  setActiveMenu(null);
                }}
              >
                <Sparkles size={13} className="text-emerald-400" />
                <div className="menu-item-text">
                  <div className="item-title">{m.name}</div>
                  <div className="item-desc">{m.provider}</div>
                </div>
                <span className="menu-model-badge">{m.badge}</span>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
          {customModelId && (
            <div style={{ padding: '6px 12px', fontSize: '11px', color: '#10b981', background: 'rgba(16, 185, 129, 0.08)', borderRadius: '6px', margin: '4px 0' }}>
              Custom override: <code>{customModelId}</code>
            </div>
          )}
          <div style={{ borderTop: '1px solid rgba(255, 255, 255, 0.08)', margin: '4px 0' }} />
          <button
            type="button"
            className="floating-menu-item"
            onClick={() => {
              window.dispatchEvent(new CustomEvent('selora:open-connect'));
              setActiveMenu(null);
            }}
          >
            <Key size={13} className="text-accent" />
            <div className="menu-item-text">
              <div className="item-title">Model & Reasoning Settings…</div>
              <div className="item-desc">Change model, temperature, reasoning depth & API key</div>
            </div>
          </button>
        </div>
      )}

      {activeMenu === 'effort' && (
        <div className="composer-floating-menu effort-menu" role="menu">
          <div className="floating-menu-header">Reasoning Depth</div>
          {EFFORT_OPTIONS.map((opt) => {
            const isSelected = opt.id === reasoningEffort;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                onClick={() => {
                  agentStore.setReasoningEffort(opt.id);
                  setActiveMenu(null);
                }}
              >
                <Zap size={13} className="text-amber-400" />
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

      {activeMenu === 'context' && (
        <div className="composer-floating-menu context-menu" role="menu">
          <div className="floating-menu-header">Attach Context</div>
          <button
            type="button"
            className="floating-menu-item"
            onClick={() => addAttachment('file', 'Active Buffer')}
          >
            <FileText size={13} className="text-muted" />
            <div className="menu-item-text">
              <div className="item-title">Active Buffer</div>
              <div className="item-desc">Attach current file in editor</div>
            </div>
          </button>
          <button
            type="button"
            className="floating-menu-item"
            onClick={() => addAttachment('code', 'Git Working Diff')}
          >
            <Code2 size={13} className="text-accent" />
            <div className="menu-item-text">
              <div className="item-title">Git Working Diff</div>
              <div className="item-desc">Attach unstaged git modifications</div>
            </div>
          </button>
          <button
            type="button"
            className="floating-menu-item"
            onClick={() => addAttachment('image', 'Screenshot')}
          >
            <ImageIcon size={13} className="text-info" />
            <div className="menu-item-text">
              <div className="item-title">Attach Screenshot</div>
              <div className="item-desc">Add image from clipboard / file</div>
            </div>
          </button>
        </div>
      )}

      {/* 5. Bottom Toolbar (ZCode V4 Parity) */}
      <div className="composer-toolbar">
        <div className="toolbar-left">
          {/* Add context button */}
          <button
            type="button"
            className="composer-btn-icon"
            onClick={() => setActiveMenu((cur) => (cur === 'context' ? null : 'context'))}
            title="Attach file, image, or workspace context (+)"
            aria-label="Add context"
          >
            <Plus size={14} />
          </button>

          {/* Permission Mode pill (ZCode V4ComposerModeControls) */}
          <button
            type="button"
            className="composer-pill mode-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'mode' ? null : 'mode'))}
            title="Toggle permission policy"
          >
            <CurrentModeIcon size={13} className={currentMode.colorClass} />
            <span className="pill-text">{currentMode.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Context attachment count indicator */}
          <span
            className="composer-context-badge"
            title="Active workspace context attachments"
            onClick={() => setActiveMenu((cur) => (cur === 'context' ? null : 'context'))}
            style={{ cursor: 'pointer' }}
          >
            <Paperclip size={12} className="badge-icon" />
            <span className="badge-count">{attachments.length + 2}</span>
          </span>

          {/* Prompt word & estimated token pill */}
          {text.trim() && (
            <span
              className="composer-token-pill"
              title="Estimated word and token usage for prompt"
            >
              {text.trim().split(/\s+/).length}w · ~{Math.max(1, Math.round(text.trim().length / 3.8))}t
            </span>
          )}
        </div>

        <div className="toolbar-right">
          {/* Model Selector Pill (ZCode ModelConfigSelect) */}
          <button
            type="button"
            className="composer-pill model-pill"
            title="Active Model: click to switch"
            onClick={() => setActiveMenu((cur) => (cur === 'model' ? null : 'model'))}
          >
            <span className="model-dot" />
            <span className="pill-text">{displayModelName}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Reasoning Effort Pill (ZCode ThoughtLevelCycleControl) */}
          <button
            type="button"
            className="composer-pill effort-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'effort' ? null : 'effort'))}
            title="Reasoning effort"
          >
            <Zap size={12} className="effort-icon" />
            <span className="pill-text">{currentEffort.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* When in-flight with typed text: offer Steer, Queue, and Stop! */}
          {inFlight && text.trim() ? (
            <div className="composer-inflight-actions-group">
              {/* Steer button (Ctrl+Enter) */}
              <button
                type="button"
                className="composer-action-pill composer-steer-btn"
                onClick={handleSteer}
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
            /* Send / Stop Action Button (ZCode Signature Button) */
            <button
              type="button"
              className={`composer-send-btn ${inFlight ? 'is-stop' : ''} ${text.trim() ? 'has-text' : ''}`}
              onClick={handleSubmit}
              title={inFlight ? 'Stop execution (Esc)' : 'Send prompt (Enter)'}
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
