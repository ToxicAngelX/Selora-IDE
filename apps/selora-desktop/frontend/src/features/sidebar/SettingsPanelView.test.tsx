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
      'light',
    ]);
    // The fixture stores "dark"; the select reports it instead of rewriting it.
    expect(theme.value).toBe('dark');
    expect(screen.getByRole('option', { name: 'Dark (alias)' })).toBeTruthy();
  });

  it('states the real persistence scope and drops the stale later-slice copy', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    expect(
      screen.getByText(
        'Theme, reduced motion, editor preferences, and the four keybinds save automatically through Rust.',
      ),
    ).toBeTruthy();
    expect(screen.getByText('Disabled fields have no backend setting in this build.')).toBeTruthy();
    expect(screen.queryByText('The other controls are read-only in this build.')).toBeNull();
    expect(screen.queryByText('Settings editing lands in a later slice.')).toBeNull();
  });

  it('renders gateway values with no backend field as unavailable, not as invented state', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    const provider = screen.getByLabelText('Default Provider') as HTMLSelectElement;
    expect(provider.disabled).toBe(true);
    expect(provider.value).toBe('');
    expect(provider.options).toHaveLength(1);
    expect(provider.options[0].textContent).toBe('—');
    expect(provider.title).toContain('no writable backend settings field');

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
    expect(temperature.title).toContain('no writable backend settings field');

    const maxTokens = screen.getByLabelText('Max Output Tokens') as HTMLInputElement;
    expect(maxTokens.disabled).toBe(true);
    expect(maxTokens.value).toBe('');
    expect(maxTokens.placeholder).toBe('—');
    expect(maxTokens.title).toContain('no writable backend settings field');

    const streaming = screen.getByRole('checkbox', { name: 'Stream Completions' }) as HTMLInputElement;
    expect(streaming.disabled).toBe(true);
    expect(streaming.checked).toBe(false);
    expect(streaming.title).toContain('no writable backend settings field');
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

  it('renders real editor controls and persists font size, tab size, and word wrap', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));

    const font = screen.getByLabelText('Font Size (px)') as HTMLInputElement;
    expect(font.disabled).toBe(false);
    expect(font.value).toBe('14');
    expect(font.min).toBe('8');
    expect(font.max).toBe('32');

    fireEvent.change(font, { target: { value: '18' } });
    fireEvent.blur(font);
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { editor_font_size: 18 },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );

    const tab = screen.getByLabelText('Tab Size') as HTMLSelectElement;
    expect(tab.value).toBe('4');
    expect(Array.from(tab.options).map((option) => option.value)).toEqual([
      '1', '2', '3', '4', '5', '6', '7', '8',
    ]);
    fireEvent.change(tab, { target: { value: '2' } });
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { editor_tab_size: 2 },
        expect.anything(),
      ),
    );

    const wrap = screen.getByRole('checkbox', { name: 'Word Wrap' }) as HTMLInputElement;
    expect(wrap.checked).toBe(false);
    fireEvent.click(wrap);
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { editor_word_wrap: true },
        expect.anything(),
      ),
    );

    // The old placeholder rows that lied about a read-only editor are gone.
    expect(screen.queryByLabelText('Font size value')).toBeNull();
    expect(screen.queryByText('—')).toBeNull();
  });

  it('keeps an invalid font size local and shows an error instead of saving it', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));
    const font = screen.getByLabelText('Font Size (px)');
    fireEvent.change(font, { target: { value: '99' } });
    fireEvent.blur(font);

    expect(
      screen.getByText('Font size must be a whole number from 8 to 32.'),
    ).toBeTruthy();
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });

  it('persists a valid keybind rebind and rejects garbage without sending it', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Keymap' }));

    const save = screen.getByLabelText('Save') as HTMLInputElement;
    expect(save.value).toBe('Ctrl+S');
    fireEvent.change(save, { target: { value: 'ctrl + alt + s' } });
    fireEvent.blur(save);
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { keybind_save: 'Ctrl+Alt+S' },
        expect.anything(),
      ),
    );

    harness.updateSettings.mockClear();
    const palette = screen.getByLabelText('Command Palette') as HTMLInputElement;
    fireEvent.change(palette, { target: { value: 'Ctrl+Shift+P+L' } });
    fireEvent.blur(palette);

    expect(
      screen.getByText(
        'Command Palette must be one chord such as Ctrl+S. "Ctrl+Shift+P+L" is not valid.',
      ),
    ).toBeTruthy();
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });

  it('builds the keymap from the command registry and advertises no unhandled chords', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Keymap' }));

    // Real registry commands, including all four rebindable chords read from settings.
    expect(screen.getAllByText('Toggle Agent Dock').length).toBeGreaterThan(0);
    expect(screen.getByText('Toggle Primary Sidebar')).toBeTruthy();
    expect(screen.getByText('Ctrl+Shift+P')).toBeTruthy();
    expect(screen.getByText('Ctrl+L')).toBeTruthy();
    expect(screen.getByText('Ctrl+B')).toBeTruthy();
    expect(screen.getByText('Ctrl+S')).toBeTruthy();
    expect(screen.getByText('Ctrl+P')).toBeTruthy();
    // Commands with no shortcut render an em dash instead of an invented chord.
    expect(screen.getByText('Save Layout')).toBeTruthy();

    // Fictional entries from the old hard-coded table are gone.
    expect(screen.queryByText('Go to file')).toBeNull();
    expect(screen.queryByText('Format document')).toBeNull();
    expect(screen.queryByText('Git: Commit staged changes')).toBeNull();
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
