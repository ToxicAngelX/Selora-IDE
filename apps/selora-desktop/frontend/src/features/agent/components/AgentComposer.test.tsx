// Selora IDE — AgentComposer Component Tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import React from 'react';
import type { BootstrapState } from '../../../bridge/bridge';
import { AgentComposer } from './AgentComposer';
import { agentStore } from '../agent-store';

const MODELS = [
  { id: 'claude-3-7-sonnet', display_name: 'Claude 3.7 Sonnet', available: true },
  { id: 'gpt-4o', display_name: 'GPT-4o', available: true },
  { id: 'deepseek-chat', display_name: 'DeepSeek Chat', available: true },
];

function makeBootstrap(overrides: Partial<BootstrapState> = {}): BootstrapState {
  return {
    protocol_version: 1,
    core_instance_id: 'core-1',
    snapshot_sequence: 1,
    workspace: { id: 'ws-1', core_instance_id: 'core-1', root: '/repo', display_name: 'repo' },
    buffers: [],
    active_buffer_id: null,
    file_tree: { root: null },
    models: MODELS,
    active_model_id: 'claude-3-7-sonnet',
    connection: 'connected',
    chat: {
      sessions: [],
      messages: [],
      active_session_id: null,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
    },
    settings: {
      reduced_motion: false,
      theme: 'selora-dark',
      keybind_command_palette: 'Ctrl+Shift+P',
      keybind_toggle_chat: 'Ctrl+L',
      keybind_fuzzy_find: 'Ctrl+P',
      keybind_save: 'Ctrl+S',
      editor_font_size: 14,
      editor_tab_size: 4,
      editor_word_wrap: false,
    },
    layout: {} as BootstrapState['layout'],
    layout_constraints: {} as BootstrapState['layout_constraints'],
    git: { branch: 'main', changed_paths: [], staged_paths: [] },
    diagnostics: [],
    lsp_servers: [],
    feature_flags: { agent: true, git: true, search: true, editor: true, later_slices: [] },
    ...overrides,
  };
}

beforeEach(() => {
  agentStore.reset();
  agentStore.updateModelSettings({ customModelId: '' });
  agentStore.hydrate(makeBootstrap());
  agentStore.setPermissionMode('full');
  agentStore.setReasoningEffort('standard');
});

function renderComposer(props: Partial<React.ComponentProps<typeof AgentComposer>> = {}) {
  return render(
    <AgentComposer
      inFlight={false}
      streamingActivityLevel={0}
      permissionMode="full"
      reasoningEffort="standard"
      promptHistory={[]}
      {...props}
    />,
  );
}

describe('AgentComposer Component', () => {
  it('submits user message on Enter when a workspace and model are available', () => {
    const sendSpy = vi.spyOn(agentStore, 'sendUserMessage');

    const { getByLabelText } = renderComposer();

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Fix compiler errors in main.rs' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });

    expect(sendSpy).toHaveBeenCalledWith('Fix compiler errors in main.rs');
    sendSpy.mockRestore();
  });

  it('sends no editor context when no buffer is active', () => {
    const sendSpy = vi.spyOn(agentStore, 'sendUserMessage');

    const { getByLabelText } = renderComposer();

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Fix compiler errors in main.rs' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });

    // Never send an empty editor DTO: one argument only when no buffer is active.
    expect(sendSpy.mock.calls[0]).toEqual(['Fix compiler errors in main.rs']);
    sendSpy.mockRestore();
  });

  it('exposes Steer and Queue buttons when typing while in-flight', () => {
    const queueSpy = vi.spyOn(agentStore, 'queuePrompt');
    const steerSpy = vi.spyOn(agentStore, 'steerActiveTurn');

    const { getByLabelText, getByText } = renderComposer({
      inFlight: true,
      streamingActivityLevel: 0.4,
    });

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Only modify frontend files' } });

    // Both Steer and Queue buttons are now exposed in the toolbar
    const steerBtn = getByLabelText('Steer active run');
    const queueBtn = getByLabelText('Queue prompt');
    expect(steerBtn).toBeTruthy();
    expect(queueBtn).toBeTruthy();

    // Measured word count (no fabricated token estimate)
    expect(getByText('4w')).toBeTruthy();

    // Clicking Queue invokes queuePrompt
    fireEvent.click(queueBtn);
    expect(queueSpy).toHaveBeenCalledWith('Only modify frontend files', 'full');

    // Re-type and re-query Steer button (re-rendered with non-empty text)
    fireEvent.change(textarea, { target: { value: 'Urgent stop directive' } });
    const steerBtnAfterReType = getByLabelText('Steer active run');
    fireEvent.click(steerBtnAfterReType);
    expect(steerSpy).toHaveBeenCalledWith('Urgent stop directive');

    queueSpy.mockRestore();
    steerSpy.mockRestore();
  });

  it('steers immediately on Ctrl+Enter while typing in-flight', () => {
    const steerSpy = vi.spyOn(agentStore, 'steerActiveTurn');

    const { getByLabelText } = renderComposer({ inFlight: true, streamingActivityLevel: 0.2 });

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Skip unit test generation' } });
    fireEvent.keyDown(textarea, { key: 'Enter', ctrlKey: true });

    expect(steerSpy).toHaveBeenCalledWith('Skip unit test generation');
    steerSpy.mockRestore();
  });

  it('restores draft into composer when selora:set-composer-draft event is dispatched', () => {
    const { getByDisplayValue } = renderComposer();

    act(() => {
      window.dispatchEvent(
        new CustomEvent('selora:set-composer-draft', {
          detail: { text: 'Restored prompt from rewind' },
        })
      );
    });

    expect(getByDisplayValue('Restored prompt from rewind')).toBeTruthy();
  });

  it('opens model selector, searches with filter, and changes selected model', () => {
    const modelSpy = vi.spyOn(agentStore, 'setSelectedModel');

    const { getByTitle, getByLabelText, getAllByText, getByRole, queryByText } = renderComposer();

    // Open model selector dropdown
    const modelPill = getByTitle('Active Model: click to switch or search');
    fireEvent.click(modelPill);

    // Verify search input is present
    const searchInput = getByLabelText('Search models');
    expect(searchInput).toBeTruthy();

    // Type query "Claude"
    fireEvent.change(searchInput, { target: { value: 'Claude' } });

    // "Claude 3.7 Sonnet" should be visible, and "GPT-4o" should be filtered out
    expect(getAllByText('Claude 3.7 Sonnet').length).toBeGreaterThan(0);
    expect(getAllByText('Anthropic').length).toBeGreaterThan(0);
    expect(queryByText('GPT-4o')).toBeNull();

    // Select Claude
    fireEvent.click(getByRole('menuitem', { name: /Claude 3.7 Sonnet/ }));
    expect(modelSpy).toHaveBeenCalledWith('claude-3-7-sonnet');

    modelSpy.mockRestore();
  });

  it('exposes menu state and closes the model search with Escape returning focus', () => {
    const { getByTitle, getByLabelText, queryByLabelText, getAllByRole } = renderComposer();

    const modelPill = getByTitle('Active Model: click to switch or search');
    expect(modelPill.getAttribute('aria-haspopup')).toBe('menu');
    expect(modelPill.getAttribute('aria-expanded')).toBe('false');
    expect(modelPill.getAttribute('aria-controls')).toBeNull();

    fireEvent.click(modelPill);

    expect(modelPill.getAttribute('aria-expanded')).toBe('true');
    expect(modelPill.getAttribute('aria-controls')).toBe('composer-model-menu');
    expect(getAllByRole('menuitem').length).toBeGreaterThan(0);

    const searchInput = getByLabelText('Search models');
    fireEvent.keyDown(searchInput, { key: 'Escape' });

    expect(queryByLabelText('Search models')).toBeNull();
    expect(modelPill.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(modelPill);
  });

  it('opens permission mode selector and switches policy', () => {
    const modeSpy = vi.spyOn(agentStore, 'setPermissionMode');

    const { getByTitle, getByText, getByRole } = renderComposer();

    const modePill = getByTitle('Toggle permission policy');
    fireEvent.click(modePill);

    expect(getByText('Permission Mode')).toBeTruthy();
    expect(getByRole('menuitem', { name: /Plan only/i })).toBeTruthy();
    expect(getByRole('menuitem', { name: /Ask before changes/i })).toBeTruthy();
    expect(getByRole('menuitem', { name: /Full auto-apply/i })).toBeTruthy();
    // Honest description of what auto-apply really does this session.
    expect(getByText(/apply immediately for this session/i)).toBeTruthy();
    expect(getByText(/still ask for approval/i)).toBeTruthy();

    fireEvent.click(getByText('Plan only'));
    expect(modeSpy).toHaveBeenCalledWith('plan_only');

    modeSpy.mockRestore();
  });

  it('shows an armed auto-apply indicator only in full mode', () => {
    const { getByText, unmount } = renderComposer({ permissionMode: 'full' });
    expect(getByText('Auto-apply armed')).toBeTruthy();
    unmount();

    const plan = renderComposer({ permissionMode: 'plan_only' });
    expect(plan.queryByText('Auto-apply armed')).toBeNull();
  });

  it('shows the "Connect and refresh models" empty state and blocks send with no catalog model', () => {
    agentStore.reset();
    agentStore.hydrate(makeBootstrap({ models: [], active_model_id: null }));
    const sendSpy = vi.spyOn(agentStore, 'sendUserMessage');

    const { getByLabelText, getByTitle, getAllByText, queryByLabelText } = renderComposer();

    // Composer-level explanation is visible without opening a menu.
    expect(getAllByText(/Connect and refresh models/).length).toBeGreaterThan(0);

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Do not send me' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    fireEvent.click(getByLabelText('Send'));

    expect(sendSpy).not.toHaveBeenCalled();

    // The catalog menu has no fabricated fallback models, just the empty state.
    fireEvent.click(getByTitle('Active Model: click to switch or search'));
    expect(queryByLabelText('Search models')).toBeNull();
    expect(document.body.textContent).not.toContain('GLM-5.3 CodeFast');
    expect(document.body.textContent).not.toContain('Claude 3.7 Sonnet');
    expect(document.body.textContent).not.toContain('GPT-4o');

    sendSpy.mockRestore();
  });

  it('explains that a folder is needed and blocks send when no workspace is open', () => {
    agentStore.reset();
    agentStore.hydrate(
      makeBootstrap({ workspace: null as unknown as BootstrapState['workspace'] }),
    );
    const sendSpy = vi.spyOn(agentStore, 'sendUserMessage');

    const { getByLabelText, getByText } = renderComposer();

    expect(getByText(/needs an open folder to read and write files/i)).toBeTruthy();

    const textarea = getByLabelText('Composer prompt input');
    fireEvent.change(textarea, { target: { value: 'Read my files' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    fireEvent.click(getByLabelText('Send'));

    expect(sendSpy).not.toHaveBeenCalled();
    sendSpy.mockRestore();
  });

  it('renders no fake attachments, model metadata, or invented values', () => {
    const { queryByText, queryByLabelText } = renderComposer();

    expect(queryByText('src/main.rs')).toBeNull();
    expect(queryByText('2.4 KB')).toBeNull();
    expect(queryByText('GLM-5.3 CodeFast')).toBeNull();
    expect(queryByText('128k ctx')).toBeNull();
    expect(queryByText('1.2s avg')).toBeNull();
    expect(queryByLabelText('Add context')).toBeNull();
    expect(queryByLabelText('Toggle workspace context menu')).toBeNull();
  });
});