import { useEffect, useState, useSyncExternalStore } from 'react';
import {
  Key,
  ShieldCheck,
  Check,
  AlertCircle,
  Eye,
  EyeOff,
  Server,
  Loader2,
  X,
  ExternalLink,
  Trash2,
  RefreshCw,
  CpuIcon,
  Sparkles,
} from '../../components/icons';
import {
  connectApiKey,
  disconnectApiKey,
  getApiKeyStatus,
  type ApiKeyStatus,
  type BootstrapState,
} from '../../bridge/bridge';
import { refreshBootstrap } from '../../app/app-store';
import { agentStore } from '../agent/agent-store';
import '../../styles/connect-modal.css';

type ProviderPreset = {
  id: string;
  name: string;
  baseUrl: string;
  placeholder: string;
  docsUrl?: string;
  description: string;
};

const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'selora',
    name: 'Selora Gateway',
    baseUrl: 'https://api.selora.lol',
    placeholder: 'sk-selora-...',
    description: 'Official Selora managed endpoint with built-in model routing',
  },
  {
    id: 'glm',
    name: 'GLM / Zhipu AI',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    placeholder: 'glm-...',
    docsUrl: 'https://open.bigmodel.cn',
    description: 'Zhipu GLM-4 and GLM-Zero models',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com',
    placeholder: 'sk-...',
    docsUrl: 'https://platform.deepseek.com',
    description: 'DeepSeek-V3 and DeepSeek-R1 reasoning models',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    placeholder: 'sk-or-v1-...',
    docsUrl: 'https://openrouter.ai/keys',
    description: 'Unified multi-provider access to Claude, GPT-4, Llama 3',
  },
  {
    id: 'openai',
    name: 'OpenAI Direct',
    baseUrl: 'https://api.openai.com/v1',
    placeholder: 'sk-proj-...',
    docsUrl: 'https://platform.openai.com/api-keys',
    description: 'OpenAI official API (GPT-4o, o1, o3-mini)',
  },
  {
    id: 'ollama',
    name: 'Ollama (Local)',
    baseUrl: 'http://localhost:11434/v1',
    placeholder: 'ollama (any text)',
    docsUrl: 'https://ollama.com',
    description: 'Run completely offline on your local GPU',
  },
  {
    id: 'custom',
    name: 'Custom Provider',
    baseUrl: '',
    placeholder: 'Enter API key...',
    description: 'Any OpenAI-compatible completions endpoint',
  },
];

export type ConnectModalProps = {
  open: boolean;
  onClose: () => void;
  bootstrap: BootstrapState;
};

export function ConnectModal({ open, onClose, bootstrap }: ConnectModalProps) {
  const [activeTab, setActiveTab] = useState<'credentials' | 'settings'>('credentials');
  const [selectedPreset, setSelectedPreset] = useState<string>('selora');
  const [baseUrl, setBaseUrl] = useState<string>('https://api.selora.lol');
  const [apiKey, setApiKey] = useState<string>('');
  const [showKey, setShowKey] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(false);
  const [refreshingModels, setRefreshingModels] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [keyStatus, setKeyStatus] = useState<ApiKeyStatus | null>(null);

  const agentSnapshot = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);

  // Fetch status on open
  useEffect(() => {
    if (!open) return;
    setError(null);
    setSuccessMsg(null);
    let mounted = true;

    void (async () => {
      try {
        const status = await getApiKeyStatus();
        if (!mounted) return;
        setKeyStatus(status);
        if (status.base_url) {
          setBaseUrl(status.base_url);
          const matched = PROVIDER_PRESETS.find((p) => p.baseUrl === status.base_url);
          if (matched) {
            setSelectedPreset(matched.id);
          } else {
            setSelectedPreset('custom');
          }
        }
      } catch (err) {
        console.error('Failed to get API key status:', err);
      }
    })();

    return () => {
      mounted = false;
    };
  }, [open, bootstrap.connection]);

  // Handle ESC key to close
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const currentPreset = PROVIDER_PRESETS.find((p) => p.id === selectedPreset) || PROVIDER_PRESETS[0];

  const handlePresetChange = (presetId: string) => {
    setSelectedPreset(presetId);
    setError(null);
    setSuccessMsg(null);
    const preset = PROVIDER_PRESETS.find((p) => p.id === presetId);
    if (preset && preset.id !== 'custom') {
      setBaseUrl(preset.baseUrl);
    }
  };

  const handleConnect = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!apiKey.trim() && !keyStatus?.has_key) {
      setError('Please enter a valid API key.');
      return;
    }

    setLoading(true);
    setError(null);
    setSuccessMsg(null);

    try {
      const keyToUse = apiKey.trim();
      const result = await connectApiKey(keyToUse, baseUrl.trim() || undefined);
      setSuccessMsg(
        `Successfully connected! Found ${result.models_count} model${result.models_count === 1 ? '' : 's'}.${
          result.active_model_id ? ` Active: ${result.active_model_id}` : ''
        }`
      );
      const updatedStatus = await getApiKeyStatus();
      setKeyStatus(updatedStatus);
      await refreshBootstrap();
      if (result.active_model_id) {
        agentStore.setSelectedModel(result.active_model_id);
      }
      setTimeout(() => {
        onClose();
      }, 1200);
    } catch (err: unknown) {
      const msg = err && typeof err === 'object' && 'message' in err
        ? String((err as { message: string }).message)
        : 'Failed to connect. Check your key and network connection.';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const handleDisconnect = async () => {
    if (!confirm('Are you sure you want to disconnect and remove the stored API key?')) {
      return;
    }
    setLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      await disconnectApiKey();
      setKeyStatus(null);
      setApiKey('');
      setSuccessMsg('Disconnected. API key was removed from credential store.');
      await refreshBootstrap();
    } catch (err: unknown) {
      const msg = err && typeof err === 'object' && 'message' in err
        ? String((err as { message: string }).message)
        : 'Failed to disconnect.';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const handleRefreshModels = async () => {
    setRefreshingModels(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const models = await agentStore.refreshModels();
      setSuccessMsg(`Refreshed catalog: found ${models.length} model${models.length === 1 ? '' : 's'}.`);
    } catch (err: unknown) {
      const msg = err && typeof err === 'object' && 'message' in err
        ? String((err as { message: string }).message)
        : 'Failed to fetch models from provider.';
      setError(msg);
    } finally {
      setRefreshingModels(false);
    }
  };

  const isConnected = bootstrap.connection === 'connected' || keyStatus?.connected;

  return (
    <div className="connect-modal-backdrop" onClick={onClose} role="dialog" aria-modal="true" aria-labelledby="connect-modal-title">
      <div className="connect-modal-card" onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div className="connect-modal-header">
          <div className="connect-modal-header-info">
            <div className="connect-modal-icon-badge">
              <Key size={18} />
            </div>
            <div>
              <h2 id="connect-modal-title" className="connect-modal-title">
                Model Provider & API Key
              </h2>
              <p className="connect-modal-subtitle">
                Configure your AI gateway credentials securely stored in OS Credential Manager
              </p>
            </div>
          </div>
          <button
            type="button"
            className="connect-modal-close-btn"
            onClick={onClose}
            aria-label="Close dialog"
          >
            <X size={16} />
          </button>
        </div>

        {/* Current Status Pill */}
        <div className={`connect-status-banner ${isConnected ? 'is-connected' : 'is-disconnected'}`}>
          <div className="connect-status-left">
            <span className="connect-status-dot" />
            <span className="connect-status-label">
              {isConnected ? 'Gateway Connected' : 'Gateway Not Connected'}
            </span>
            {keyStatus?.key_hint && (
              <span className="connect-key-hint" title="Stored key mask">
                Key: {keyStatus.key_hint}
              </span>
            )}
          </div>
          {keyStatus?.active_model_id && (
            <span className="connect-active-model">
              Active: <code>{keyStatus.active_model_id}</code>
            </span>
          )}
        </div>

        {/* Tabs */}
        <div className="connect-tabs-nav" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === 'credentials'}
            className={`connect-tab-btn ${activeTab === 'credentials' ? 'is-active' : ''}`}
            onClick={() => {
              setActiveTab('credentials');
              setError(null);
              setSuccessMsg(null);
            }}
          >
            <Server size={14} />
            <span>Provider & Credentials</span>
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={activeTab === 'settings'}
            className={`connect-tab-btn ${activeTab === 'settings' ? 'is-active' : ''}`}
            onClick={() => {
              setActiveTab('settings');
              setError(null);
              setSuccessMsg(null);
            }}
          >
            <CpuIcon size={14} />
            <span>Model & Reasoning Settings</span>
          </button>
        </div>

        {activeTab === 'credentials' ? (
          /* Form Body - Credentials */
          <form onSubmit={handleConnect} className="connect-modal-form">
            {/* Provider Preset Selector */}
            <div className="connect-field">
              <label className="connect-label" htmlFor="provider-preset">
                Provider Preset
              </label>
              <div className="connect-preset-grid">
                {PROVIDER_PRESETS.map((preset) => {
                  const isSelected = selectedPreset === preset.id;
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      className={`connect-preset-card ${isSelected ? 'is-selected' : ''}`}
                      onClick={() => handlePresetChange(preset.id)}
                    >
                      <div className="connect-preset-name">
                        {preset.name}
                        {isSelected && <Check size={14} className="connect-preset-check" />}
                      </div>
                      <div className="connect-preset-desc">{preset.description}</div>
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Gateway Base URL */}
            <div className="connect-field">
              <div className="connect-field-header">
                <label className="connect-label" htmlFor="gateway-url">
                  Gateway Base URL
                </label>
                {currentPreset.docsUrl && (
                  <a
                    href={currentPreset.docsUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="connect-docs-link"
                  >
                    Provider Docs <ExternalLink size={11} />
                  </a>
                )}
              </div>
              <div className="connect-input-wrapper">
                <Server size={14} className="connect-input-icon" />
                <input
                  id="gateway-url"
                  type="text"
                  className="connect-input"
                  placeholder="https://api.openai.com/v1"
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  required
                />
              </div>
            </div>

            {/* API Key */}
            <div className="connect-field">
              <div className="connect-field-header">
                <label className="connect-label" htmlFor="api-key-input">
                  API Key
                </label>
                <span className="connect-field-hint">Never logged or committed</span>
              </div>
              <div className="connect-input-wrapper">
                <Key size={14} className="connect-input-icon" />
                <input
                  id="api-key-input"
                  type={showKey ? 'text' : 'password'}
                  className="connect-input"
                  placeholder={keyStatus?.key_hint ? `Stored: ${keyStatus.key_hint} (Enter new to replace)` : currentPreset.placeholder}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  autoComplete="off"
                  spellCheck="false"
                />
                <button
                  type="button"
                  className="connect-toggle-eye"
                  onClick={() => setShowKey(!showKey)}
                  aria-label={showKey ? 'Hide key' : 'Show key'}
                >
                  {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>

            {/* Alert messages */}
            {error && (
              <div className="connect-alert is-error" role="alert">
                <AlertCircle size={15} className="connect-alert-icon" />
                <span className="connect-alert-text">{error}</span>
              </div>
            )}

            {successMsg && (
              <div className="connect-alert is-success" role="status">
                <ShieldCheck size={15} className="connect-alert-icon" />
                <span className="connect-alert-text">{successMsg}</span>
              </div>
            )}

            {/* Action Footer */}
            <div className="connect-modal-footer">
              <div className="connect-footer-left">
                {keyStatus?.has_key && (
                  <button
                    type="button"
                    className="connect-btn-disconnect"
                    onClick={handleDisconnect}
                    disabled={loading}
                  >
                    <Trash2 size={13} />
                    <span>Disconnect</span>
                  </button>
                )}
              </div>
              <div className="connect-footer-right">
                <button
                  type="button"
                  className="connect-btn-cancel"
                  onClick={onClose}
                  disabled={loading}
                >
                  Close
                </button>
                <button
                  type="submit"
                  className="connect-btn-submit"
                  disabled={loading || (!apiKey.trim() && !keyStatus?.has_key)}
                >
                  {loading ? (
                    <>
                      <Loader2 size={14} className="connect-spinner" />
                      <span>Testing Gateway…</span>
                    </>
                  ) : (
                    <>
                      <ShieldCheck size={14} />
                      <span>Save & Connect</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          </form>
        ) : (
          /* Settings Tab Body */
          <div className="connect-settings-tab">
            {/* Active Model Picker & Refresh */}
            <div className="connect-field">
              <div className="connect-field-row">
                <label className="connect-label" htmlFor="model-select">
                  Active Model
                </label>
                <button
                  type="button"
                  className="connect-refresh-btn"
                  onClick={handleRefreshModels}
                  disabled={refreshingModels || !isConnected}
                  title="Fetch latest models list from the connected provider"
                >
                  <RefreshCw size={12} className={refreshingModels ? 'connect-spinner' : ''} />
                  <span>{refreshingModels ? 'Refreshing…' : 'Refresh Models'}</span>
                </button>
              </div>
              {agentSnapshot.availableModels.length > 0 ? (
                <select
                  id="model-select"
                  className="connect-select"
                  value={agentSnapshot.selectedModel}
                  onChange={(e) => agentStore.setSelectedModel(e.target.value)}
                >
                  {agentSnapshot.availableModels.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.display_name || m.id} {m.id !== m.display_name ? `(${m.id})` : ''}
                    </option>
                  ))}
                </select>
              ) : (
                <div className="connect-input-wrapper">
                  <Sparkles size={14} className="connect-input-icon text-muted" />
                  <input
                    id="model-select"
                    type="text"
                    className="connect-input"
                    placeholder="e.g. gpt-4o, claude-3-7-sonnet"
                    value={agentSnapshot.selectedModel}
                    onChange={(e) => agentStore.setSelectedModel(e.target.value)}
                  />
                </div>
              )}
              <span className="connect-field-hint">
                {agentSnapshot.availableModels.length > 0
                  ? `${agentSnapshot.availableModels.length} models available from connected provider.`
                  : isConnected
                  ? 'Click "Refresh Models" above to probe provider catalog.'
                  : 'Connect your provider in the Credentials tab to auto-discover models.'}
              </span>
            </div>

            {/* Custom Model ID Override */}
            <div className="connect-field">
              <label className="connect-label" htmlFor="custom-model-id">
                Custom Model ID (Optional Override)
              </label>
              <div className="connect-input-wrapper">
                <Sparkles size={14} className="connect-input-icon text-muted" />
                <input
                  id="custom-model-id"
                  type="text"
                  className="connect-input"
                  placeholder="e.g. anthropic/claude-3.7-sonnet, deepseek-reasoner"
                  value={agentSnapshot.modelSettings.customModelId}
                  onChange={(e) => agentStore.updateModelSettings({ customModelId: e.target.value })}
                />
              </div>
              <span className="connect-field-hint">
                If specified, this model ID will take precedence over catalog selection for agent turns.
              </span>
            </div>

            {/* Reasoning Depth */}
            <div className="connect-field">
              <label className="connect-label">Reasoning Depth</label>
              <div className="connect-effort-grid">
                {[
                  { id: 'quick' as const, label: 'Quick', desc: 'Minimal thought overhead, fast responses.' },
                  { id: 'standard' as const, label: 'Standard', desc: 'Balanced step-by-step verification.' },
                  { id: 'max' as const, label: 'Max', desc: 'Exhaustive chain of thought & edge-case checks.' },
                ].map((opt) => {
                  const isSel = agentSnapshot.reasoningEffort === opt.id;
                  return (
                    <button
                      key={opt.id}
                      type="button"
                      className={`connect-effort-card ${isSel ? 'is-selected' : ''}`}
                      onClick={() => agentStore.setReasoningEffort(opt.id)}
                    >
                      <div className="connect-effort-header">
                        <span>{opt.label}</span>
                        {isSel && <Check size={12} className="text-ok" />}
                      </div>
                      <div className="connect-effort-desc">{opt.desc}</div>
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Sampling Temperature */}
            <div className="connect-field">
              <div className="connect-slider-container">
                <div className="connect-slider-header">
                  <label className="connect-label" htmlFor="temperature-slider">
                    Temperature (Creativity / Randomness)
                  </label>
                  <span className="connect-slider-val">
                    {agentSnapshot.modelSettings.temperature.toFixed(2)}
                  </span>
                </div>
                <input
                  id="temperature-slider"
                  type="range"
                  min="0.0"
                  max="1.5"
                  step="0.05"
                  className="connect-slider"
                  value={agentSnapshot.modelSettings.temperature}
                  onChange={(e) => agentStore.updateModelSettings({ temperature: parseFloat(e.target.value) })}
                />
                <div className="connect-slider-ticks">
                  <span>0.0 (Deterministic / Code)</span>
                  <span>0.7 (Balanced)</span>
                  <span>1.5 (Creative)</span>
                </div>
              </div>
            </div>

            {/* Custom System Instructions */}
            <div className="connect-field">
              <label className="connect-label" htmlFor="system-instructions">
                Custom System Instructions
              </label>
              <textarea
                id="system-instructions"
                className="connect-textarea"
                placeholder="e.g. Always write clean TypeScript with strict types and zero any. Verify code changes before completing tasks."
                value={agentSnapshot.modelSettings.systemInstructions}
                onChange={(e) => agentStore.updateModelSettings({ systemInstructions: e.target.value })}
                rows={3}
              />
              <span className="connect-field-hint">
                Appended to Selora's system prompt on every agent turn.
              </span>
            </div>

            {/* Alert messages */}
            {error && (
              <div className="connect-alert is-error" role="alert">
                <AlertCircle size={15} className="connect-alert-icon" />
                <span className="connect-alert-text">{error}</span>
              </div>
            )}

            {successMsg && (
              <div className="connect-alert is-success" role="status">
                <ShieldCheck size={15} className="connect-alert-icon" />
                <span className="connect-alert-text">{successMsg}</span>
              </div>
            )}

            {/* Action Footer */}
            <div className="connect-modal-footer">
              <div className="connect-footer-left" />
              <div className="connect-footer-right">
                <button
                  type="button"
                  className="connect-btn-submit"
                  onClick={onClose}
                >
                  <Check size={14} />
                  <span>Done</span>
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
