// Selora IDE — Interactive Settings Panel (DESIGN §3.3)
import { useState, useMemo, type FC } from 'react';
import {
  updateSettings,
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
 * The select mirrors Rust's accepted set (state.rs `apply_settings_update`):
 * "selora-dark" | "dark" | "space-black" | "light". "dark" is a legacy alias
 * kept visible so a stored alias is not silently rewritten in the display.
 */
const unsupportedTitle = (label: string): string =>
  `${label} has no backend settings field in this build. This control is read-only; changes are not saved.`;

function formatError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export const SettingsPanelView: FC<SettingsPanelViewProps> = ({ bootstrap, onOpenConnect }) => {
  const settings = bootstrap.settings;
  const isConnected = bootstrap.connection === 'connected';

  const [activeTab, setActiveTab] = useState<SettingsTab>('gateway');
  const [keybindQuery, setKeybindQuery] = useState('');
  const [saving, setSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);

  // settings_update is not session-scoped: Rust dispatch rejects any
  // envelope `session_id` on it, so settings always send null.
  const context: EnvelopeContext = {
    core_instance_id: bootstrap.core_instance_id,
    workspace_id: bootstrap.workspace.id,
    session_id: null,
  };

  const persist = (patch: SettingsPatch): void => {
    setSettingsError(null);
    setSaving(true);
    void updateSettings(patch, context)
      .catch((cause: unknown) => setSettingsError(formatError(cause)))
      .finally(() => setSaving(false));
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
                    : unsupportedTitle('Model selection')
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
          <p className="settings-card-hint">
            Editor preferences exist in settings.json but are not exposed by this build&apos;s
            settings protocol, so every value shows —.
          </p>
          <div className="settings-group">
            {[
              { label: 'Font Family', name: 'Font family' },
              { label: 'Font Size (px)', name: 'Font size' },
              { label: 'Tab Size', name: 'Tab size' },
              { label: 'Word Wrap', name: 'Word wrap' },
              { label: 'Format On Save', name: 'Format on save' },
              { label: 'Minimap', name: 'Minimap' },
              { label: 'Line Numbers', name: 'Line numbers' },
            ].map((row) => (
              <div className="settings-row" key={row.name}>
                <span className="settings-label">{row.label}</span>
                <span
                  className="settings-card-hint"
                  aria-label={`${row.name} value`}
                  title={unsupportedTitle(row.name)}
                >
                  —
                </span>
              </div>
            ))}
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
          </div>
        </div>
      )}

      {/* Tab 4: Keybindings Table */}
      {activeTab === 'keybindings' && (
        <div
          className="settings-section-content keybindings-section"
          title="Commands come from the shell's command registry. Keybinding editing has no backend field in this build, so this table is read-only."
        >
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
        </dl>
      )}

      {/* Honest persistence scope: only the two SettingsPatch fields write. */}
      <div className="settings-footer-info">
        <p className="settings-later">
          Theme and reduced motion save automatically through Rust.
        </p>
        <p className="settings-later">The other controls are read-only in this build.</p>
      </div>
    </div>
  );
};
