// Selora IDE — Provider & API Key Connection Modal (Elevated).
//
// Modern provider cards (Selora API Gateway, Anthropic Claude, OpenAI, DeepSeek,
// Local Ollama, OpenRouter, GLM, Custom) with glowing selection state, live key
// format validation badges, animated "Test Connection" button with spinning loader
// and success checkmark, and smooth enter/exit modal transitions.
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
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
  Globe,
  Zap,
  Bot,
} from '../../components/icons';
import {
  connectApiKey,
  disconnectApiKey,
  getApiKeyStatus,
  openExternal,
  type ApiKeyStatus,
  type BootstrapState,
} from '../../bridge/bridge';
import { currentEnvelopeContext, refreshBootstrap } from '../../app/app-store';
import { agentStore } from '../agent/agent-store';
import '../../styles/connect-modal.css';

export type ProviderPreset = {
  id: string;
  name: string;
  baseUrl: string;
  placeholder: string;
  docsUrl?: string;
  description: string;
  tag: 'Recommended' | 'Direct' | 'Local' | 'Gateway' | 'Custom';
  iconType: 'sparkles' | 'bot' | 'cpu' | 'zap' | 'server' | 'globe' | 'custom';
  keyPrefixHint: string;
};

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'selora',
    name: 'Selora API Gateway',
    baseUrl: 'https://api.selora.lol',
    placeholder: 'sk-selora-... or sk-gw-...',
    description: 'Official Selora managed endpoint with built-in model routing & speed',
    tag: 'Recommended',
    iconType: 'sparkles',
    keyPrefixHint: 'sk-gw-...',
  },
  {
    id: 'anthropic',
    name: 'Anthropic Claude',
    baseUrl: 'https://api.anthropic.com/v1',
    placeholder: 'sk-ant-api03-...',
    docsUrl: 'https://console.anthropic.com/settings/keys',
    description: 'Claude 3.7 Sonnet, Claude 3.5 Haiku & Opus reasoning models',
    tag: 'Direct',
    iconType: 'bot',
    keyPrefixHint: 'sk-ant-...',
  },
  {
    id: 'openai',
    name: 'OpenAI Direct',
    baseUrl: 'https://api.openai.com/v1',
    placeholder: 'sk-proj-...',
    docsUrl: 'https://platform.openai.com/api-keys',
    description: 'OpenAI official API (GPT-4o, o1, o3-mini)',
    tag: 'Direct',
    iconType: 'cpu',
    keyPrefixHint: 'sk-proj-...',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com',
    placeholder: 'sk-...',
    docsUrl: 'https://platform.deepseek.com',
    description: 'DeepSeek-V3 and DeepSeek-R1 reasoning & math models',
    tag: 'Direct',
    iconType: 'zap',
    keyPrefixHint: 'sk-...',
  },
  {
    id: 'ollama',
    name: 'Local Ollama',
    baseUrl: 'http://localhost:11434/v1',
    placeholder: 'No API key needed (offline)',
    docsUrl: 'https://ollama.com',
    description: 'Run completely offline on your local GPU (Llama 3, DeepSeek-R1)',
    tag: 'Local',
    iconType: 'server',
    keyPrefixHint: 'None (Local)',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    placeholder: 'sk-or-v1-...',
    docsUrl: 'https://openrouter.ai/keys',
    description: 'Unified multi-provider access to Claude, GPT-4, Llama 3',
    tag: 'Gateway',
    iconType: 'globe',
    keyPrefixHint: 'sk-or-...',
  },
  {
    id: 'glm',
    name: 'GLM / Zhipu AI',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    placeholder: 'glm-...',
    docsUrl: 'https://open.bigmodel.cn',
    description: 'Zhipu GLM-4 and GLM-Zero models',
    tag: 'Direct',
    iconType: 'cpu',
    keyPrefixHint: 'glm-...',
  },
  {
    id: 'custom',
    name: 'Custom Provider',
    baseUrl: '',
    placeholder: 'Enter API key or leave blank...',
    description: 'Any OpenAI-compatible completions endpoint',
    tag: 'Custom',
    iconType: 'custom',
    keyPrefixHint: 'Custom',
  },
];

const EXTERNAL_DOC_HOST_ALLOWLIST = new Set([
  'github.com',
  'api.github.com',
  'console.anthropic.com',
  'platform.openai.com',
  'openrouter.ai',
  'platform.deepseek.com',
  'open.bigmodel.cn',
  'ollama.com',
]);

function docsHost(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return null;
    return parsed.hostname.toLowerCase();
  } catch {
    return null;
  }
}

function isAllowedDocsUrl(url: string): boolean {
  const host = docsHost(url);
  return host !== null && EXTERNAL_DOC_HOST_ALLOWLIST.has(host);
}

function getKeyValidation(presetId: string, key: string, hasStoredKey: boolean) {
  const trimmed = key.trim();
  if (presetId === 'ollama') {
    return {
      status: 'valid' as const,
      badge: 'Local GPU · Key not required',
      isValid: true,
    };
  }
  if (!trimmed) {
    if (hasStoredKey) {
      return {
        status: 'stored' as const,
        badge: 'Stored key active',
        isValid: true,
      };
    }
    return {
      status: 'idle' as const,
      badge: 'Format required',
      isValid: false,
    };
  }
  if (presetId === 'selora') {
    const valid = trimmed.startsWith('sk-selora-') || trimmed.startsWith('sk-gw-') || trimmed.startsWith('sk-');
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid Selora format (sk-gw-...)' : 'Format hint: sk-gw-... or sk-selora-...',
      isValid: valid,
    };
  }
  if (presetId === 'anthropic') {
    const valid = trimmed.startsWith('sk-ant-');
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid Anthropic format (sk-ant-...)' : 'Format hint: sk-ant-...',
      isValid: valid,
    };
  }
  if (presetId === 'openai') {
    const valid = trimmed.startsWith('sk-proj-') || trimmed.startsWith('sk-');
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid OpenAI format (sk-proj-...)' : 'Format hint: sk-proj-... or sk-...',
      isValid: valid,
    };
  }
  if (presetId === 'deepseek') {
    const valid = trimmed.startsWith('sk-');
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid DeepSeek format (sk-...)' : 'Format hint: sk-...',
      isValid: valid,
    };
  }
  if (presetId === 'openrouter') {
    const valid = trimmed.startsWith('sk-or-');
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid OpenRouter format (sk-or-...)' : 'Format hint: sk-or-...',
      isValid: valid,
    };
  }
  if (presetId === 'glm') {
    const valid = trimmed.length > 5;
    return {
      status: valid ? ('valid' as const) : ('warning' as const),
      badge: valid ? 'Valid GLM format' : 'Format hint: glm-...',
      isValid: valid,
    };
  }
  return {
    status: trimmed.length > 0 ? ('valid' as const) : ('idle' as const),
    badge: trimmed.length > 0 ? 'Custom key provided' : 'Enter API key',
    isValid: trimmed.length > 0,
  };
}

export type ConnectModalProps = {
  open: boolean;
  onClose: () => void;
  bootstrap: BootstrapState;
};

export function ConnectModal({ open, onClose, bootstrap }: ConnectModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const [activeTab, setActiveTab] = useState<'credentials' | 'settings'>('credentials');
  const [selectedPreset, setSelectedPreset] = useState<string>('selora');
  const [baseUrl, setBaseUrl] = useState<string>('https://api.selora.lol');
  const [apiKey, setApiKey] = useState<string>('');
  const [showKey, setShowKey] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(false);
  const [testSuccess, setTestSuccess] = useState<boolean>(false);
  const [refreshingModels, setRefreshingModels] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [keyStatus, setKeyStatus] = useState<ApiKeyStatus | null>(null);

  const agentSnapshot = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);

  // Dialog contract (same as CheatSheet/AboutDialog): remember the opener,
  // move focus to the close control, and return focus when the dialog closes
  // or unmounts while still owning focus.
  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeRef.current?.focus();
    return () => {
      const restore = restoreFocusRef.current;
      restoreFocusRef.current = null;
      if (!restore || !restore.isConnected) return;
      const active = document.activeElement;
      const dialogHadFocus =
        active === document.body || active === null || (dialogRef.current?.contains(active) ?? false);
      if (dialogHadFocus) restore.focus();
    };
  }, [open]);

  // Fetch status on open
  useEffect(() => {
    if (!open) return;
    setError(null);
    setSuccessMsg(null);
    setTestSuccess(false);
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
  const docsUrl = currentPreset.docsUrl ?? '';
  const docsLinkHost = docsHost(docsUrl);
  const docsAllowed = isAllowedDocsUrl(docsUrl);
  const isConnected = bootstrap.connection === 'connected' || keyStatus?.connected;
  const keyValidation = getKeyValidation(selectedPreset, apiKey, Boolean(keyStatus?.has_key));

  const handleDocsClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    if (!docsUrl || !docsAllowed) return;
    const context = currentEnvelopeContext();
    if (!context) {
      setError('Cannot open documentation: no workspace context.');
      return;
    }
    void openExternal(docsUrl, context).catch(() => undefined);
  };

  const handlePresetChange = (presetId: string) => {
    setSelectedPreset(presetId);
    setError(null);
    setSuccessMsg(null);
    setTestSuccess(false);
    const preset = PROVIDER_PRESETS.find((p) => p.id === presetId);
    if (preset && preset.id !== 'custom') {
      setBaseUrl(preset.baseUrl);
    }
  };

  const handleConnect = async (e: React.FormEvent) => {
    e.preventDefault();
    if (selectedPreset !== 'ollama' && !apiKey.trim() && !keyStatus?.has_key) {
      setError('Please enter a valid API key.');
      return;
    }

    setLoading(true);
    setTestSuccess(false);
    setError(null);
    setSuccessMsg(null);

    try {
      const keyToUse = apiKey.trim() || (selectedPreset === 'ollama' ? 'ollama' : '');
      const result = await connectApiKey(keyToUse, baseUrl.trim() || undefined);
      setTestSuccess(true);
      setSuccessMsg(
        `Successfully connected! Discovered ${result.models_count} model${
          result.models_count === 1 ? '' : 's'
        }.${result.active_model_id ? ` Active: ${result.active_model_id}` : ''}`
      );
      const updatedStatus = await getApiKeyStatus();
      setKeyStatus(updatedStatus);
      await refreshBootstrap();
      if (result.active_model_id) {
        agentStore.setSelectedModel(result.active_model_id);
      }
      setTimeout(() => {
        onClose();
      }, 1300);
    } catch (err: unknown) {
      const msg =
        err && typeof err === 'object' && 'message' in err
          ? String((err as { message: string }).message)
          : 'Failed to connect. Check your gateway address, API key, and network status.';
      setError(msg);
      setTestSuccess(false);
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
    setTestSuccess(false);
    try {
      await disconnectApiKey();
      setKeyStatus(null);
      setApiKey('');
      setSuccessMsg('Disconnected. API key was securely removed from OS Credential Store.');
      await refreshBootstrap();
    } catch (err: unknown) {
      const msg =
        err && typeof err === 'object' && 'message' in err
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
      setSuccessMsg(`Refreshed catalog: discovered ${models.length} model${models.length === 1 ? '' : 's'}.`);
    } catch (err: unknown) {
      const msg =
        err && typeof err === 'object' && 'message' in err
          ? String((err as { message: string }).message)
          : 'Failed to fetch models from provider.';
      setError(msg);
    } finally {
      setRefreshingModels(false);
    }
  };

  const renderProviderIcon = (iconType: ProviderPreset['iconType']) => {
    switch (iconType) {
      case 'sparkles':
        return <Sparkles size={16} />;
      case 'bot':
        return <Bot size={16} />;
      case 'cpu':
        return <CpuIcon size={16} />;
      case 'zap':
        return <Zap size={16} />;
      case 'server':
        return <Server size={16} />;
      case 'globe':
        return <Globe size={16} />;
      default:
        return <Server size={16} />;
    }
  };

  const handleDialogKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = dialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="connect-modal-backdrop"
      onClick={onClose}
      data-testid="connect-modal-backdrop"
    >
      <div
        ref={dialogRef}
        className="connect-modal-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="connect-modal-title"
        aria-describedby="connect-modal-description"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleDialogKeyDown}
      >
        {/* Header */}
        <div className="connect-modal-header">
          <div className="connect-modal-header-info">
            <div className="connect-modal-icon-badge">
              <Key size={18} aria-hidden="true" />
            </div>
            <div>
              <h2 id="connect-modal-title" className="connect-modal-title">
                Model Provider & Gateway Connection
              </h2>
              <p id="connect-modal-description" className="connect-modal-subtitle">
                Configure your AI gateway credentials securely stored in OS Credential Manager
              </p>
            </div>
          </div>
          <button
            ref={closeRef}
            type="button"
            className="connect-modal-close-btn"
            onClick={onClose}
            aria-label="Close dialog"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        {/* Current Status Pill Banner */}
        <div
          className={`connect-status-banner ${isConnected ? 'is-connected' : 'is-disconnected'}`}
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          <div className="connect-status-left">
            <span className="connect-status-dot" aria-hidden="true" />
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

        {/* Navigation Tabs */}
        <div className="connect-tabs-nav" role="tablist" aria-label="Connection settings sections">
          <button
            id="connect-tab-credentials"
            type="button"
            role="tab"
            aria-selected={activeTab === 'credentials'}
            aria-controls="connect-panel-credentials"
            className={`connect-tab-btn ${activeTab === 'credentials' ? 'is-active' : ''}`}
            onClick={() => {
              setActiveTab('credentials');
              setError(null);
              setSuccessMsg(null);
            }}
          >
            <Server size={14} aria-hidden="true" />
            <span>Provider & Credentials</span>
          </button>
          <button
            id="connect-tab-settings"
            type="button"
            role="tab"
            aria-selected={activeTab === 'settings'}
            aria-controls="connect-panel-settings"
            className={`connect-tab-btn ${activeTab === 'settings' ? 'is-active' : ''}`}
            onClick={() => {
              setActiveTab('settings');
              setError(null);
              setSuccessMsg(null);
            }}
          >
            <CpuIcon size={14} aria-hidden="true" />
            <span>Model & Reasoning Settings</span>
          </button>
        </div>

        {activeTab === 'credentials' ? (
          /* Form Body - Credentials */
          <form
            id="connect-panel-credentials"
            role="tabpanel"
            aria-labelledby="connect-tab-credentials"
            onSubmit={handleConnect}
            className="connect-modal-form"
          >
            {/* Provider Preset Selector with Modern Cards */}
            <div className="connect-field">
              <span id="provider-preset-label" className="connect-label">
                Select Model Provider
              </span>
              <div
                className="connect-preset-grid"
                role="group"
                aria-labelledby="provider-preset-label"
              >
                {PROVIDER_PRESETS.map((preset) => {
                  const isSelected = selectedPreset === preset.id;
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      className={`connect-preset-card ${isSelected ? 'is-selected' : ''}`}
                      onClick={() => handlePresetChange(preset.id)}
                      aria-pressed={isSelected}
                      aria-label={`${preset.name} (${preset.tag})`}
                      aria-describedby={`provider-preset-desc-${preset.id}`}
                    >
                      <div className="connect-preset-top">
                        <div className="connect-preset-icon-wrapper" aria-hidden="true">
                          {renderProviderIcon(preset.iconType)}
                        </div>
                        <span className={`connect-preset-badge tag-${preset.tag.toLowerCase()}`}>
                          {preset.tag}
                        </span>
                      </div>
                      <div className="connect-preset-name">
                        <span>{preset.name}</span>
                        {isSelected && (
                          <Check size={14} className="connect-preset-check" aria-hidden="true" />
                        )}
                      </div>
                      <div id={`provider-preset-desc-${preset.id}`} className="connect-preset-desc">
                        {preset.description}
                      </div>
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
                {docsUrl && (
                  <a
                    href={docsUrl}
                    rel="noreferrer"
                    className="connect-docs-link"
                    aria-label={`${currentPreset.name} documentation (opens in a new tab)`}
                    aria-disabled={docsAllowed ? undefined : true}
                    tabIndex={docsAllowed ? undefined : -1}
                    title={
                      docsAllowed
                        ? undefined
                        : `Documentation unavailable: ${docsLinkHost ?? 'invalid URL'} is not on the native opener allowlist.`
                    }
                    style={docsAllowed ? undefined : { opacity: 0.5, cursor: 'not-allowed' }}
                    onClick={handleDocsClick}
                  >
                    Documentation <ExternalLink size={11} aria-hidden="true" />
                  </a>
                )}
              </div>
              <div className="connect-input-wrapper">
                <Server size={14} className="connect-input-icon" aria-hidden="true" />
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

            {/* API Key Input with Format Validation Badge */}
            <div className="connect-field">
              <div className="connect-field-header">
                <div className="connect-label-group">
                  <label className="connect-label" htmlFor="api-key-input">
                    API Key
                  </label>
                  {/* Validation Badge */}
                  <span
                    id="api-key-validation"
                    className={`connect-format-badge badge-${keyValidation.status}`}
                    role="status"
                    aria-live="polite"
                    aria-atomic="true"
                  >
                    {keyValidation.status === 'valid' && <Check size={10} aria-hidden="true" />}
                    {keyValidation.status === 'warning' && (
                      <AlertCircle size={10} aria-hidden="true" />
                    )}
                    <span>{keyValidation.badge}</span>
                  </span>
                </div>
                <span id="api-key-privacy-hint" className="connect-field-hint">
                  Never logged or committed
                </span>
              </div>
              <div
                className={`connect-input-wrapper${
                  keyValidation.status === 'warning' ? ' is-invalid' : ''
                }`}
              >
                <Key size={14} className="connect-input-icon" aria-hidden="true" />
                <input
                  id="api-key-input"
                  type={showKey ? 'text' : 'password'}
                  className="connect-input"
                  aria-describedby="api-key-validation api-key-privacy-hint"
                  aria-invalid={keyValidation.status === 'warning' ? true : undefined}
                  placeholder={
                    keyStatus?.key_hint
                      ? `Stored: ${keyStatus.key_hint} (Enter new key to replace)`
                      : currentPreset.placeholder
                  }
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
                  aria-pressed={showKey}
                >
                  {showKey ? <EyeOff size={14} aria-hidden="true" /> : <Eye size={14} aria-hidden="true" />}
                </button>
              </div>
            </div>

            {/* Alert Messages */}
            {error && (
              <div className="connect-alert is-error" role="alert">
                <AlertCircle size={15} className="connect-alert-icon" aria-hidden="true" />
                <span className="connect-alert-text">{error}</span>
              </div>
            )}

            {successMsg && (
              <div className="connect-alert is-success" role="status">
                <ShieldCheck size={15} className="connect-alert-icon" aria-hidden="true" />
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
                    <Trash2 size={13} aria-hidden="true" />
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
                  Cancel
                </button>
                {/* Animated "Test Connection" button */}
                <button
                  type="submit"
                  className={`connect-btn-submit${testSuccess ? ' is-success' : ''}`}
                  aria-busy={loading}
                  disabled={
                    loading ||
                    (selectedPreset !== 'ollama' && !apiKey.trim() && !keyStatus?.has_key)
                  }
                >
                  {loading ? (
                    <>
                      <Loader2 size={14} className="connect-spinner" aria-hidden="true" />
                      <span>Testing Gateway…</span>
                    </>
                  ) : testSuccess ? (
                    <>
                      <Check size={14} className="connect-btn-check" aria-hidden="true" />
                      <span>Connected & Verified!</span>
                    </>
                  ) : (
                    <>
                      <ShieldCheck size={14} aria-hidden="true" />
                      <span>Test & Connect</span>
                    </>
                  )}
                </button>
                <span
                  className="visually-hidden"
                  role="status"
                  aria-live="polite"
                  data-testid="connect-test-status"
                >
                  {loading ? 'Testing connection…' : ''}
                </span>
              </div>
            </div>
          </form>
        ) : (
          /* Settings Tab Body */
          <div
            id="connect-panel-settings"
            role="tabpanel"
            aria-labelledby="connect-tab-settings"
            className="connect-settings-tab"
          >
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
                  <RefreshCw
                    size={12}
                    className={refreshingModels ? 'connect-spinner' : ''}
                    aria-hidden="true"
                  />
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
                  <Sparkles size={14} className="connect-input-icon text-muted" aria-hidden="true" />
                  <input
                    id="model-select"
                    type="text"
                    className="connect-input"
                    placeholder="e.g. claude-3-7-sonnet, gpt-4o, deepseek-chat"
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
                <Sparkles size={14} className="connect-input-icon text-muted" aria-hidden="true" />
                <input
                  id="custom-model-id"
                  type="text"
                  className="connect-input"
                  placeholder="e.g. claude-3-7-sonnet-20250219, deepseek-reasoner"
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
              <span id="reasoning-depth-label" className="connect-label">
                Reasoning Depth
              </span>
              <div className="connect-effort-grid" role="group" aria-labelledby="reasoning-depth-label">
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
                      aria-pressed={isSel}
                      aria-label={`${opt.label} reasoning: ${opt.desc}`}
                    >
                      <div className="connect-effort-header">
                        <span>{opt.label}</span>
                        {isSel && <Check size={12} className="text-ok" aria-hidden="true" />}
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
                    Temperature (Creativity / Determinism)
                  </label>
                  <output htmlFor="temperature-slider" className="connect-slider-val">
                    {agentSnapshot.modelSettings.temperature.toFixed(2)}
                  </output>
                </div>
                <input
                  id="temperature-slider"
                  type="range"
                  min="0.0"
                  max="1.5"
                  step="0.05"
                  className="connect-slider"
                  value={agentSnapshot.modelSettings.temperature}
                  onChange={(e) =>
                    agentStore.updateModelSettings({ temperature: parseFloat(e.target.value) })
                  }
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
                onChange={(e) =>
                  agentStore.updateModelSettings({ systemInstructions: e.target.value })
                }
                rows={3}
              />
              <span className="connect-field-hint">
                Appended to Selora's system prompt on every agent turn.
              </span>
            </div>

            {/* Alert messages */}
            {error && (
              <div className="connect-alert is-error" role="alert">
                <AlertCircle size={15} className="connect-alert-icon" aria-hidden="true" />
                <span className="connect-alert-text">{error}</span>
              </div>
            )}

            {successMsg && (
              <div className="connect-alert is-success" role="status">
                <ShieldCheck size={15} className="connect-alert-icon" aria-hidden="true" />
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
                  <Check size={14} aria-hidden="true" />
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
