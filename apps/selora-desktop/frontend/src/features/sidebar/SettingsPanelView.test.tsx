import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  updateSettings: vi.fn(),
  restartLsp: vi.fn(),
}));

vi.mock('../../bridge/bridge', () => ({
  updateSettings: harness.updateSettings,
  restartLsp: harness.restartLsp,
}));

import { SettingsPanelView } from './SettingsPanelView';

describe('SettingsPanelView persistence wiring', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.updateSettings.mockResolvedValue(undefined);
    harness.restartLsp.mockResolvedValue(undefined);
  });

  it('sends supported toggle changes through the bridge with the bootstrap context', async () => {
    const bootstrap = makeBootstrap();
    render(<SettingsPanelView bootstrap={bootstrap} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Reduced Motion' }));

    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { reduced_motion: true },
        {
          core_instance_id: 'core-0001',
          workspace_id: 'no-workspace',
          session_id: null,
        },
      ),
    );
  });

  it('sends a supported theme change through the bridge', async () => {
    const bootstrap = makeBootstrap();
    render(<SettingsPanelView bootstrap={bootstrap} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    fireEvent.change(screen.getByLabelText('Color Theme'), { target: { value: 'light' } });

    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { theme: 'light' },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });

  it('reflects the persisted light theme after a bootstrap round-trip', () => {
    const bootstrap = makeBootstrap();
    const { rerender } = render(<SettingsPanelView bootstrap={bootstrap} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    fireEvent.change(screen.getByLabelText('Color Theme'), { target: { value: 'light' } });

    rerender(
      <SettingsPanelView
        bootstrap={{
          ...bootstrap,
          settings: { ...bootstrap.settings, theme: 'light' },
        }}
      />,
    );

    expect((screen.getByLabelText('Color Theme') as HTMLSelectElement).value).toBe('light');
    expect(screen.getByRole('option', { name: 'Selora Moonlight (Light)' })).toBeTruthy();
  });

  it('offers exactly the theme values Rust accepts, including the stored "dark" alias', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    const theme = screen.getByLabelText('Color Theme') as HTMLSelectElement;

    expect(Array.from(theme.options).map((option) => option.value)).toEqual([
      'selora-dark',
      'dark',
      'space-black',
      'light',
    ]);
    // The fixture stores "dark"; the select reports it instead of rewriting it.
    expect(theme.value).toBe('dark');
    expect(screen.getByRole('option', { name: 'Dark (alias)' })).toBeTruthy();
  });

  it('states the real persistence scope and drops the stale later-slice copy', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    expect(
      screen.getByText('Theme and reduced motion save automatically through Rust.'),
    ).toBeTruthy();
    expect(screen.getByText('The other controls are read-only in this build.')).toBeTruthy();
    expect(screen.queryByText('Settings editing lands in a later slice.')).toBeNull();
  });

  it('renders gateway values with no backend field as unavailable, not as invented state', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    const provider = screen.getByLabelText('Default Provider') as HTMLSelectElement;
    expect(provider.disabled).toBe(true);
    expect(provider.value).toBe('');
    expect(provider.options).toHaveLength(1);
    expect(provider.options[0].textContent).toBe('—');
    expect(provider.title).toContain('no backend settings field');

    const model = screen.getByLabelText('Chat Model') as HTMLSelectElement;
    expect(model.disabled).toBe(true);
    expect(model.value).toBe('');
    expect(model.options[0].textContent).toBe('—');
    expect(screen.queryByText(/glm-4-plus/)).toBeNull();
    expect(screen.queryByText(/GLM-4 \(Zhipu AI\)/)).toBeNull();

    const temperature = screen.getByLabelText('Temperature') as HTMLInputElement;
    expect(temperature.disabled).toBe(true);
    expect(temperature.value).toBe('');
    expect(temperature.placeholder).toBe('—');
    expect(temperature.title).toContain('no backend settings field');

    const maxTokens = screen.getByLabelText('Max Output Tokens') as HTMLInputElement;
    expect(maxTokens.disabled).toBe(true);
    expect(maxTokens.value).toBe('');
    expect(maxTokens.placeholder).toBe('—');
    expect(maxTokens.title).toContain('no backend settings field');

    const streaming = screen.getByRole('checkbox', { name: 'Stream Completions' }) as HTMLInputElement;
    expect(streaming.disabled).toBe(true);
    expect(streaming.checked).toBe(false);
    expect(streaming.title).toContain('no backend settings field');
  });

  it('shows the active model read-only when the bootstrap reports one', () => {
    const bootstrap = makeBootstrap({
      active_model_id: 'glm-4-plus',
      models: [{ id: 'glm-4-plus', display_name: 'GLM-4 Plus', available: true }],
    });
    render(<SettingsPanelView bootstrap={bootstrap} />);

    const model = screen.getByLabelText('Chat Model') as HTMLSelectElement;
    expect(model.disabled).toBe(true);
    expect(model.value).toBe('glm-4-plus');
    expect(screen.getByRole('option', { name: 'GLM-4 Plus (glm-4-plus)' })).toBeTruthy();
    expect(model.title).toContain('reported by the connected gateway');
  });

  it('does not present editor defaults that the settings protocol never exposes', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));

    for (const label of [
      'Font family value',
      'Font size value',
      'Tab size value',
      'Word wrap value',
      'Minimap value',
      'Line numbers value',
    ]) {
      const value = screen.getByLabelText(label);
      expect(value.textContent).toBe('—');
      // These fields persist in settings.json; only the settings wire omits them.
      expect(value.getAttribute('title')).toContain('stored in settings.json');
      expect(value.getAttribute('title')).toContain('does not expose it');
    }

    // Format on save is not a settings.json field at all, so it must not
    // borrow the "stored but unexposed" explanation.
    const formatOnSave = screen.getByLabelText('Format on save value');
    expect(formatOnSave.textContent).toBe('—');
    expect(formatOnSave.getAttribute('title')).toContain('no backend settings field');
    expect(formatOnSave.getAttribute('title')).not.toContain('stored in settings.json');

    expect(screen.queryByText('13.5')).toBeNull();
    expect(screen.queryByText('2 spaces')).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'Word Wrap' })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'Format On Save' })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'Minimap' })).toBeNull();
  });

  it('builds the keymap from the command registry and advertises no unhandled chords', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Keymap' }));

    // Real registry commands, including the rebindable pair read from settings.
    expect(screen.getByText('Toggle Agent Dock')).toBeTruthy();
    expect(screen.getByText('Toggle Primary Sidebar')).toBeTruthy();
    expect(screen.getByText('Ctrl+Shift+P')).toBeTruthy();
    expect(screen.getByText('Ctrl+L')).toBeTruthy();
    expect(screen.getByText('Ctrl+B')).toBeTruthy();
    // Commands with no shortcut render an em dash instead of an invented chord.
    expect(screen.getByText('Save Layout')).toBeTruthy();

    // Fictional entries from the old hard-coded table are gone.
    expect(screen.queryByText('Go to file')).toBeNull();
    expect(screen.queryByText('Format document')).toBeNull();
    expect(screen.queryByText('Git: Commit staged changes')).toBeNull();
    expect(screen.queryByText('Ctrl+P')).toBeNull();
    expect(screen.queryByText('Shift+Alt+F')).toBeNull();
    expect(screen.queryByText('Ctrl+Enter')).toBeNull();
  });

  it('does not invent an accent tint', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    const accent = screen.getByLabelText('Accent Tint') as HTMLSelectElement;

    expect(accent.disabled).toBe(true);
    expect(accent.value).toBe('');
    expect(Array.from(accent.options).map((option) => option.value)).toEqual(['']);
    expect(screen.queryByText(/emerald/i)).toBeNull();
    expect(screen.queryByText(/22c55e/i)).toBeNull();
  });

  it('shows the truthful LSP state and restarts a server through the bridge', async () => {
    const bootstrap = makeBootstrap({
      lsp_servers: [
        { language: 'rust', server: 'rust-analyzer', state: 'ready' },
        {
          language: 'typescript',
          server: 'typescript-language-server',
          state: 'failed',
          error: 'spawn failed: not found',
        },
      ],
    });
    render(<SettingsPanelView bootstrap={bootstrap} />);
    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));

    expect(screen.getByTestId('lsp-state-rust').textContent).toContain('ready');
    expect(screen.getByTestId('lsp-state-typescript').textContent).toContain(
      'failed: spawn failed: not found',
    );

    fireEvent.click(screen.getByRole('button', { name: 'Restart rust-analyzer' }));
    await waitFor(() =>
      expect(harness.restartLsp).toHaveBeenCalledWith(
        'rust',
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });
});