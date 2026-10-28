// Selora IDE — Interactive Settings Panel (DESIGN §3.3)
import { useEffect, useState, useMemo, type FC } from 'react';
import {
  updateSettings,
  restartLsp,
  type BootstrapState,
  type EnvelopeContext,
  type SettingsPatch,
} from '../../bridge/bridge';
import {
  Key,
  Code2,
  Eye,
  Terminal,
  Search,
} from '../../components/icons';
import { buildCommandList, commandCategory } from '../keymap/commands';
import { canonicalKeybind, parseKeybind } from '../keymap/keybind';
import { THEME_OPTIONS, normalizeThemeValue } from '../theme/theme';

export interface SettingsPanelViewProps {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}

type SettingsTab = 'gateway' | 'editor' | 'appearance' | 'keybindings';

interface KeybindingEntry {
  id: string;
  command: string;
  keys: string;
  category: string;
}

/**
 * The select offers the palettes users can choose. Rust still accepts the
 * legacy `space-black` spelling but resolves it to dark; a stored alias is not
 * silently rewritten in the display.
 */

/** Mirrors selora-settings' EDITOR_* range constants. */
const FONT_SIZE_MIN = 8;
const FONT_SIZE_MAX = 32;
const TAB_SIZE_MIN = 1;
const TAB_SIZE_MAX = 8;

type KeybindDraftKey =
  | 'keybind_command_palette'
  | 'keybind_fuzzy_find'
  | 'keybind_toggle_chat'
  | 'keybind_save';

const KEYBIND_FIELDS: Array<{ key: KeybindDraftKey; label: string; command: string }> = [
  { key: 'keybind_save', label: 'Save', command: 'Save' },
  { key: 'keybind_fuzzy_find', label: 'Find in Files', command: 'Find in Files' },
  { key: 'keybind_command_palette', label: 'Command Palette', command: 'Open Command Palette' },
  { key: 'keybind_toggle_chat', label: 'Toggle Agent Dock', command: 'Toggle Agent Dock' },
];

/** For values that do persist in settings.json but have no settings-wire exposure. */
const unexposedTitle = (label: string): string =>
  `${label} is stored in settings.json, but this build's settings protocol does not expose it. This value is read-only; changes are not saved.`;

function formatError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** Only for values with genuinely no writable backend field. */
const unsupportedTitle = (label: string): string =>
  `${label} has no writable backend settings field in this build. This control is disabled; changes are not saved.`;

export const SettingsPanelView: FC<SettingsPanelViewProps> = ({ bootstrap, onOpenConnect }) => {
  const settings = bootstrap.settings;
  const isConnected = bootstrap.connection === 'connected';

  const [activeTab, setActiveTab] = useState<SettingsTab>('gateway');
  const [keybindQuery, setKeybindQuery] = useState('');
  const [saving, setSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [fontSizeDraft, setFontSizeDraft] = useState(String(settings.editor_font_size));
  const [tabSizeDraft, setTabSizeDraft] = useState(String(settings.editor_tab_size));
  const [keybindDrafts, setKeybindDrafts] = useState<Record<KeybindDraftKey, string>>({
    keybind_command_palette: settings.keybind_command_palette,
    keybind_fuzzy_find: settings.keybind_fuzzy_find,
    keybind_toggle_chat: settings.keybind_toggle_chat,
    keybind_save: settings.keybind_save,
  });

  // The backend event is the source of truth: after a save (or an external
  // settings change) the fields show the persisted value, not a stale draft.
  useEffect(() => {
    setFontSizeDraft(String(settings.editor_font_size));
    setTabSizeDraft(String(settings.editor_tab_size));
    setKeybindDrafts({
      keybind_command_palette: settings.keybind_command_palette,
      keybind_fuzzy_find: settings.keybind_fuzzy_find,
      keybind_toggle_chat: settings.keybind_toggle_chat,
      keybind_save: settings.keybind_save,
    });
  }, [
    settings.editor_font_size,
    settings.editor_tab_size,
    settings.keybind_command_palette,
    settings.keybind_fuzzy_find,
    settings.keybind_toggle_chat,
    settings.keybind_save,
  ]);

  // settings_update is not session-scoped: Rust dispatch rejects any
  // envelope `session_id` on it, so settings always send null.
  const context: EnvelopeContext = {
    core_instance_id: bootstrap.core_instance_id,
    workspace_id: bootstrap.workspace.id,
    session_id: null,
  };

  const persist = (patch: SettingsPatch): void => {
    setSettingsError(null);
    setFieldError(null);
    setSaving(true);
    void updateSettings(patch, context)
      .catch((cause: unknown) => setSettingsError(formatError(cause)))
      .finally(() => setSaving(false));
  };

const restartServer = (language: string): void => {
    setSettingsError(null);
    void restartLsp(language, context).catch((cause: unknown) =>
      setSettingsError(formatError(cause)),
    );
  };

  const lspServers = bootstrap.lsp_servers ?? [];

  const commitFontSize = (): void => {
    const value = Number(fontSizeDraft);
    if (!Number.isInteger(value) || value < FONT_SIZE_MIN || value > FONT_SIZE_MAX) {
      setFieldError(`Font size must be a whole number from ${FONT_SIZE_MIN} to ${FONT_SIZE_MAX}.`);
      return;
    }
    if (value === settings.editor_font_size) return;
    persist({ editor_font_size: value });
  };

  const commitKeybind = (key: KeybindDraftKey): void => {
    const label = KEYBIND_FIELDS.find((field) => field.key === key)?.label ?? key;
    const raw = keybindDrafts[key].trim();
    const parsed = parseKeybind(raw);
    if (!parsed) {
      setFieldError(`${label} must be one chord such as Ctrl+S. "${raw}" is not valid.`);
      return;
    }
    const canonical = canonicalKeybind(parsed);
    if (canonical === settings[key]) return;
    persist({ [key]: canonical } as SettingsPatch);
  };

  // The cheat sheet is built from the real command registry, so it can never
  // advertise a command the shell does not dispatch. Commands without a
  // shortcut are listed with "—" instead of an invented chord.
  const keybindingList: KeybindingEntry[] = useMemo(() => {
    return buildCommandList(settings).map((command) => ({
      id: command.id,
      command: command.title,
      keys: command.shortcut ?? '—',
      category: commandCategory(command.id),
    }));
  }, [settings]);

  const filteredKeybindings = useMemo(() => {
    if (!keybindQuery.trim()) return keybindingList;
    const q = keybindQuery.toLowerCase();
    return keybindingList.filter(
      (k) =>
        k.command.toLowerCase().includes(q) ||
        k.keys.toLowerCase().includes(q) ||
        k.category.toLowerCase().includes(q)
    );
  }, [keybindingList, keybindQuery]);

  const themeValue = normalizeThemeValue(settings.theme);

  // The active model is the only model fact the bridge exposes. When the
  // bootstrap has one, show it read-only; never substitute a placeholder id.
  const activeModel = useMemo(() => {
    const id = bootstrap.active_model_id;
    if (!id) return null;
    const model = bootstrap.models.find((candidate) => candidate.id === id);
    return { id, label: model ? `${model.display_name} (${model.id})` : id };
  }, [bootstrap.active_model_id, bootstrap.models]);

  return (
    <div className="sidebar-settings-panel">
      {/* Settings Navigation Tabs */}
      <nav className="settings-nav-tabs" aria-label="Settings Categories">
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'gateway' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('gateway')}
          title="AI Gateway & Providers"
        >
          <Key size={12} />
          <span>AI Gateway</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'editor' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('editor')}
          title="Editor Preferences"
        >
          <Code2 size={12} />
          <span>Editor</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'appearance' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('appearance')}
          title="Appearance & Theme"
        >
          <Eye size={12} />
          <span>Theme</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'keybindings' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('keybindings')}
          title="Keyboard Shortcuts"
        >
          <Terminal size={12} />
          <span>Keymap</span>
        </button>
      </nav>

      {settingsError ? (
        <p className="settings-card-hint" role="alert">
          Settings update failed: {settingsError}
        </p>
      ) : null}
      {fieldError ? (
        <p className="settings-card-hint" role="alert">
          {fieldError}
        </p>
      ) : null}

      {/* Tab 1: AI Gateway & Models */}
      {activeTab === 'gateway' && (
        <div className="settings-section-content">
          {/* Status & Gateway Card */}
          <div className="settings-card ai-gateway-card">
            <div className="settings-card-header">
              <span className="settings-card-title">AI Provider & Key</span>
              <span
                className={`settings-status-pill ${
                  isConnected ? 'is-connected' : 'is-disconnected'
                }`}
              >
                {isConnected ? 'Connected' : 'Not Connected'}
              </span>
            </div>
            <p className="settings-card-hint">
              Connect GLM, DeepSeek, OpenAI, OpenRouter, or custom completions gateways.
            </p>
            <button
              type="button"
              className="settings-primary-btn"
              onClick={onOpenConnect}
            >
              {isConnected ? 'Manage API Key & Gateway' : 'Configure API Key'}
            </button>
          </div>

          {/* Model Selection Group */}
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-provider-select">
                Default Provider
              </label>
              <select
                id="settings-provider-select"
                className="settings-select"
                value=""
                disabled
                title={unsupportedTitle('Provider selection')}
              >
                <option value="">—</option>
              </select>
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-model-select">
                Chat Model
              </label>
              <select
                id="settings-model-select"
                className="settings-select"
                value={activeModel?.id ?? ''}
                disabled
                title={
                  activeModel
                    ? 'The active model is reported by the connected gateway and cannot be changed in this build.'
                    : 'No active model is reported by the gateway. This build has no model-selection control, so changes are not saved.'
                }
              >
                <option value="">—</option>
                {activeModel ? <option value={activeModel.id}>{activeModel.label}</option> : null}
              </select>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-streaming-label">
                Stream Completions
              </span>
              <label className="settings-switch" title={unsupportedTitle('Streaming')}>
                <input
                  type="checkbox"
                  checked={false}
                  disabled
                  aria-labelledby="settings-streaming-label"
                  title={unsupportedTitle('Streaming')}
                />
                <span className="switch-slider" />
              </label>
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-temp-input">
                Temperature
              </label>
              <input
                id="settings-temp-input"
                className="settings-number-input"
                type="number"
                value=""
                placeholder="—"
                disabled
                title={unsupportedTitle('Temperature')}
              />
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-max-tokens">
                Max Output Tokens
              </label>
              <input
                id="settings-max-tokens"
                className="settings-number-input"
                type="number"
                value=""
                placeholder="—"
                disabled
                title={unsupportedTitle('Max output tokens')}
              />
            </div>
          </div>
        </div>
      )}

      {/* Tab 2: Editor */}
      {activeTab === 'editor' && (
        <div className="settings-section-content">
          <div className="settings-card">
            <div className="settings-card-header">
              <span className="settings-card-title">Language Servers</span>
              <span className="settings-card-hint">
                {lspServers.length === 0 ? 'Not started' : String(lspServers.length) + ' tracked'}
              </span>
            </div>
            <p className="settings-card-hint">
              Servers start when a file of their language opens. Restart re-runs the
              server for this workspace; paths, arguments and the master switch come
              from the <code>lsp</code> section of settings.json.
            </p>
            {lspServers.length === 0 ? (
              <p className="settings-card-hint" role="status">
                No language server has started in this workspace yet.
              </p>
            ) : (
              <div className="settings-group">
                {lspServers.map((server) => (
                  <div className="settings-row" key={server.language}>
                    <span className="settings-label">
                      {server.server}
                      <span
                        className="settings-card-hint"
                        data-testid={`lsp-state-${server.language}`}
                        role="status"
                      >
                        {' '}
                        — {server.state}
                        {server.error ? `: ${server.error}` : ''}
                      </span>
                    </span>
                    <button
                      type="button"
                      className="settings-primary-btn"
                      onClick={() => restartServer(server.language)}
                      aria-label={`Restart ${server.server}`}
                    >
                      Restart
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
          <p className="settings-card-hint">
            These values save to settings.json and apply to the editor immediately.
          </p>
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-font-size">
                Font Size (px)
              </label>
              <input
                id="settings-font-size"
                className="settings-number-input"
                type="number"
                min={FONT_SIZE_MIN}
                max={FONT_SIZE_MAX}
                step={1}
                value={fontSizeDraft}
                disabled={saving}
                aria-busy={saving}
                onChange={(e) => setFontSizeDraft(e.target.value)}
                onBlur={commitFontSize}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') commitFontSize();
                }}
                title={`Editor font size, ${FONT_SIZE_MIN}–${FONT_SIZE_MAX} px.`}
              />
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-tab-size">
                Tab Size
              </label>
              <select
                id="settings-tab-size"
                className="settings-select"
                value={tabSizeDraft}
                disabled={saving}
                onChange={(e) => {
                  const value = Number(e.target.value);
                  setTabSizeDraft(e.target.value);
                  if (value !== settings.editor_tab_size) persist({ editor_tab_size: value });
                }}
              >
                {Array.from(
                  { length: TAB_SIZE_MAX - TAB_SIZE_MIN + 1 },
                  (_, index) => TAB_SIZE_MIN + index,
                ).map((size) => (
                  <option key={size} value={size}>
                    {size} {size === 1 ? 'space' : 'spaces'}
                  </option>
                ))}
              </select>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-word-wrap-label">
                Word Wrap
              </span>
              <label className="settings-switch">
                <input
                  type="checkbox"
                  checked={settings.editor_word_wrap}
                  disabled={saving}
                  aria-busy={saving}
                  aria-labelledby="settings-word-wrap-label"
                  onChange={(e) => persist({ editor_word_wrap: e.target.checked })}
                />
                <span className="switch-slider" />
              </label>
            </div>

            <div className="settings-row">
              <span className="settings-label">Font Family</span>
              <span
                className="settings-card-hint"
                aria-label="Font family value"
                title={unsupportedTitle('Font family')}
              >
                JetBrains Mono
              </span>
            </div>
          </div>
        </div>
      )}

      {/* Tab 3: Appearance & Theme */}
      {activeTab === 'appearance' && (
        <div className="settings-section-content">
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-theme-select">
                Color Theme
              </label>
              <select
                id="settings-theme-select"
                className="settings-select"
                value={themeValue}
                onChange={(e) => persist({ theme: e.target.value })}
              >
                {THEME_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-reduced-motion-label">
                Reduced Motion
              </span>
              <label className="settings-switch">
                <input
                  type="checkbox"
                  checked={settings.reduced_motion}
                  onChange={(e) => persist({ reduced_motion: e.target.checked })}
                  aria-labelledby="settings-reduced-motion-label"
                  aria-busy={saving}
                />
                <span className="switch-slider" />
              </label>
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-accent-select">
                Accent Tint
              </label>
              <select
                id="settings-accent-select"
                className="settings-select"
                value=""
                disabled
                title={unsupportedTitle('Accent tint')}
              >
                <option value="">—</option>
              </select>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-check-updates-label">
                Automatic Updates
              </span>
              <label
                className="settings-switch"
                title="Check GitHub Releases once per launch and offer the newer installer in a centered prompt. The About dialog's manual check always works."
              >
                <input
                  type="checkbox"
                  checked={settings.check_for_updates ?? true}
                  disabled={saving}
                  aria-busy={saving}
                  aria-labelledby="settings-check-updates-label"
                  onChange={(e) => persist({ check_for_updates: e.target.checked })}
                />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
        </div>
      )}

      {/* Tab 4: Keybindings */}
      {activeTab === 'keybindings' && (
        <div className="settings-section-content keybindings-section">
          <p className="settings-card-hint">
            Edit a chord, then press Enter or leave the field. Invalid chords are not saved.
          </p>
          <div className="settings-group">
            {KEYBIND_FIELDS.map((field) => (
              <div className="settings-row" key={field.key}>
                <label
                  className="settings-label"
                  htmlFor={`settings-keybind-${field.key}`}
                >
                  {field.label}
                </label>
                <input
                  id={`settings-keybind-${field.key}`}
                  className="settings-text-input"
                  type="text"
                  value={keybindDrafts[field.key]}
                  disabled={saving}
                  aria-busy={saving}
                  spellCheck={false}
                  onChange={(e) =>
                    setKeybindDrafts((current) => ({
                      ...current,
                      [field.key]: e.target.value,
                    }))
                  }
                  onBlur={() => commitKeybind(field.key)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitKeybind(field.key);
                  }}
                />
              </div>
            ))}
          </div>

          {/* Filter Input */}
          <div className="keybind-search-wrapper">
            <Search size={12} className="keybind-search-icon" />
            <input
              className="keybind-search-input"
              type="text"
              placeholder="Search keybindings (e.g. palette, agent, Ctrl)..."
              value={keybindQuery}
              onChange={(e) => setKeybindQuery(e.target.value)}
              aria-label="Filter keybindings"
            />
          </div>

          {/* Keybindings Cheat-Sheet Table */}
          <div className="keybindings-table-wrapper">
            <table className="keybindings-table">
              <thead>
                <tr>
                  <th>Command</th>
                  <th>Keybinding</th>
                </tr>
              </thead>
              <tbody>
                {filteredKeybindings.map((item) => (
                  <tr key={item.id} className="keybinding-row">
                    <td className="keybinding-cmd-cell">
                      <span className="keybinding-cmd-name">{item.command}</span>
                      <span className="keybinding-category">{item.category}</span>
                    </td>
                    <td className="keybinding-key-cell">
                      <kbd className="settings-kbd">{item.keys}</kbd>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Quick Settings & Keybind Echo (Required for Z2 spec parity & ShellGrid.test.tsx) */}
      {activeTab !== 'keybindings' && (
        <dl className="settings-echo">
          <div className="settings-echo-row">
            <dt>Command palette</dt>
            <dd>{settings.keybind_command_palette}</dd>
          </div>
          <div className="settings-echo-row">
            <dt>Toggle agent dock</dt>
            <dd>{settings.keybind_toggle_chat}</dd>
          </div>
          <div className="settings-echo-row">
            <dt>Save</dt>
            <dd>{settings.keybind_save}</dd>
          </div>
        </dl>
      )}

      <div className="settings-footer-info">
        <p className="settings-later">
          Theme, reduced motion, editor preferences, and the four keybinds save automatically
          through Rust.
        </p>
        <p className="settings-later">
          Disabled fields have no backend setting in this build.
        </p>
      </div>
    </div>
  );
};
