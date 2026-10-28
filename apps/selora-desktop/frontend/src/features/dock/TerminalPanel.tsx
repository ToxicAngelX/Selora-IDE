// Selora IDE — integrated terminal (terminal slice).
//
// A real xterm.js surface wired to the TerminalCreate / TerminalInput /
// TerminalResize / TerminalClose commands and the TerminalCreated /
// TerminalOutput / TerminalExited events. Every row on screen comes from the
// backend PTY: there is no simulated prompt and no fabricated output. When the
// backend is missing, the panel says so and renders no terminal.
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { Icon } from '../../components/Icon';
import { currentEnvelopeContext } from '../../app/app-store';
import {
  terminalStore,
  terminalTabLabel,
  type TerminalSession,
  type TerminalState,
} from './terminal-store';
import {
  prefersReducedMotion,
  readTerminalTheme,
  terminalFontFamily,
  terminalFontSize,
} from './terminal-theme';

const INITIAL_COLS = 80;
const INITIAL_ROWS = 24;

export type TerminalHandle = {
  focus: () => void;
  copySelection: () => Promise<boolean>;
  pasteClipboard: () => Promise<void>;
};

function errorText(cause: unknown): string {
  if (typeof cause === 'object' && cause !== null) {
    const candidate = cause as { message?: unknown };
    if (typeof candidate.message === 'string') return candidate.message;
  }
  return cause instanceof Error ? cause.message : String(cause);
}

/** The first TerminalCreate grid. The view sends TerminalResize after fit. */
export function estimateTerminalGrid(element: HTMLElement | null): { cols: number; rows: number } {
  if (!element) return { cols: INITIAL_COLS, rows: INITIAL_ROWS };
  const rect = element.getBoundingClientRect();
  if (rect.width < 80 || rect.height < 40) {
    return { cols: INITIAL_COLS, rows: INITIAL_ROWS };
  }
  const cols = Math.max(20, Math.floor((rect.width - 24) / 7.2));
  const rows = Math.max(5, Math.floor((rect.height - 24) / 16));
  return { cols, rows };
}

function TerminalView({
  session,
  active,
  register,
  onNotice,
  onNew,
  onClose,
}: {
  session: TerminalSession;
  active: boolean;
  register: (terminalId: string, handle: TerminalHandle | null) => void;
  onNotice: (message: string) => void;
  onNew: () => void;
  onClose: () => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<() => void>(() => undefined);
  const activeRef = useRef(active);
  const [attempt, setAttempt] = useState(0);
  const [failure, setFailure] = useState<string | null>(null);
  activeRef.current = active;

  const copySelection = useCallback(async (): Promise<boolean> => {
    const terminal = terminalRef.current;
    if (!terminal) return false;
    const selection = terminal.getSelection();
    if (selection === '') {
      onNotice('Select terminal text to copy.');
      return false;
    }
    try {
      await navigator.clipboard.writeText(selection);
      onNotice('Copied the terminal selection.');
      return true;
    } catch {
      onNotice('The clipboard is unavailable; the selection was not copied.');
      return false;
    }
  }, [onNotice]);

  const pasteClipboard = useCallback(async (): Promise<void> => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    try {
      const text = await navigator.clipboard.readText();
      if (text === '') {
        onNotice('The clipboard is empty.');
        return;
      }
      terminal.paste(text);
      onNotice('Pasted from the clipboard.');
    } catch {
      onNotice('The clipboard is unavailable; nothing was pasted.');
    }
  }, [onNotice]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminalId = session.terminal_id;
    let terminal: Terminal | null = null;
    let fitAddon: FitAddon | null = null;
    let mutationObserver: MutationObserver | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let lastCols = 0;
    let lastRows = 0;

    setFailure(null);
    try {
      terminal = new Terminal({
        allowProposedApi: false,
        allowTransparency: false,
        convertEol: false,
        cursorBlink: !prefersReducedMotion(),
        cursorStyle: 'bar',
        fontFamily: terminalFontFamily(),
        fontSize: terminalFontSize(),
        lineHeight: 1.2,
        minimumContrastRatio: 1,
        rightClickSelectsWord: true,
        scrollback: 5000,
        theme: readTerminalTheme(),
      });
      fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);
      terminal.open(host);
      terminalRef.current = terminal;
    } catch (cause) {
      setFailure(errorText(cause));
      terminal?.dispose();
      terminalRef.current = null;
      return;
    }

    const live = terminal;
    const fit = fitAddon;

    const safeFit = () => {
      if (host.clientWidth <= 0 || host.clientHeight <= 0) return;
      try {
        fit.fit();
      } catch {
        // A hidden or zero-sized tab cannot be measured; the observer retries.
      }
    };
    fitRef.current = safeFit;

    const sendResize = (cols: number, rows: number) => {
      if (cols <= 0 || rows <= 0) return;
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;
      const ctx = currentEnvelopeContext();
      if (ctx) terminalStore.sendResize(terminalId, cols, rows, ctx);
    };

    const dataDisposable = live.onData((data) => {
      const ctx = currentEnvelopeContext();
      if (ctx) terminalStore.sendInput(terminalId, data, ctx);
    });
    const resizeDisposable = live.onResize(({ cols, rows }) => sendResize(cols, rows));
    const outputUnsubscribe = terminalStore.subscribeOutput(terminalId, (data) => live.write(data));

    live.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true;
      const modifier = event.ctrlKey || event.metaKey;
      if (modifier && event.shiftKey && event.code === 'KeyC') {
        void copySelection();
        return false;
      }
      if (modifier && event.shiftKey && event.code === 'KeyV') {
        void pasteClipboard();
        return false;
      }
      if (modifier && !event.shiftKey && event.code === 'KeyC' && live.hasSelection()) {
        void copySelection();
        return false;
      }
      return true;
    });

    const applyTheme = () => {
      live.options.theme = readTerminalTheme();
      live.options.fontFamily = terminalFontFamily();
      live.options.fontSize = terminalFontSize();
      live.options.cursorBlink = !prefersReducedMotion();
      safeFit();
    };

    lastCols = live.cols;
    lastRows = live.rows;
    safeFit();
    if (activeRef.current) live.focus();

    if (typeof MutationObserver !== 'undefined') {
      mutationObserver = new MutationObserver(applyTheme);
      mutationObserver.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ['data-theme', 'data-reduced-motion'],
      });
    }
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => safeFit());
      resizeObserver.observe(host);
    }

    register(terminalId, {
      focus: () => live.focus(),
      copySelection,
      pasteClipboard,
    });

    return () => {
      register(terminalId, null);
      dataDisposable.dispose();
      resizeDisposable.dispose();
      outputUnsubscribe();
      mutationObserver?.disconnect();
      resizeObserver?.disconnect();
      fitRef.current = () => undefined;
      terminalRef.current = null;
      live.dispose();
    };
  }, [session.terminal_id, attempt, copySelection, pasteClipboard, register]);

  useEffect(() => {
    if (!active || failure) return;
    fitRef.current();
    terminalRef.current?.focus();
  }, [active, failure]);

  return (
    <div
      className={`terminal-view${active ? ' is-active' : ''}`}
      id={`terminal-panel-${session.terminal_id}`}
      role="tabpanel"
      aria-labelledby={`terminal-tab-${session.terminal_id}`}
      aria-hidden={!active}
      data-terminal-id={session.terminal_id}
    >
      {session.status === 'exited' ? (
        <div className="terminal-exit-banner" role="status" data-testid="terminal-exit-banner">
          <span>
            Process exited with code {session.exit_code ?? 'unknown'}. The shell is no longer running.
          </span>
          <div className="terminal-exit-actions">
            <button type="button" onClick={onNew}>
              New terminal
            </button>
            <button type="button" onClick={onClose}>
              Close tab
            </button>
          </div>
        </div>
      ) : null}

      {failure ? (
        <div className="terminal-view-error" role="alert">
          <p>Terminal view failed to start: {failure}</p>
          <button
            type="button"
            onClick={() => {
              setFailure(null);
              setAttempt((value) => value + 1);
            }}
          >
            Retry view
          </button>
        </div>
      ) : (
        <div
          ref={hostRef}
          className="terminal-xterm"
          data-testid={`terminal-xterm-${session.terminal_id}`}
          onMouseDown={() => terminalRef.current?.focus()}
        />
      )}
    </div>
  );
}

function ActionButton({
  label,
  icon,
  onClick,
  disabled,
}: {
  label: string;
  icon: 'plus' | 'copy' | 'paste' | 'close';
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      className="terminal-action"
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
    >
      <Icon name={icon} size={12} />
    </button>
  );
}

export function TerminalPanel() {
  const state: TerminalState = useSyncExternalStore(terminalStore.subscribe, terminalStore.getSnapshot);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const handles = useRef(new Map<string, TerminalHandle>());
  const started = useRef(false);
  const [notice, setNotice] = useState<string | null>(null);

  const register = useCallback((terminalId: string, handle: TerminalHandle | null) => {
    if (handle) handles.current.set(terminalId, handle);
    else handles.current.delete(terminalId);
  }, []);

  const startTerminal = useCallback(() => {
    const ctx = currentEnvelopeContext();
    if (!ctx) {
      terminalStore.reportError(
        'Open a folder before starting a terminal. The terminal needs a workspace.',
      );
      return;
    }
    const { cols, rows } = estimateTerminalGrid(bodyRef.current);
    void terminalStore.requestCreate(cols, rows, ctx);
  }, []);

  // Opening the dock starts one shell. A create failure stays visible instead
  // of silently retrying, so a missing backend looks missing.
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (terminalStore.getSnapshot().sessions.length > 0) return;
    startTerminal();
  }, [startTerminal]);

  const activeSession = state.sessions.find((session) => session.terminal_id === state.active_id);

  const activate = (terminalId: string) => {
    terminalStore.activate(terminalId);
    handles.current.get(terminalId)?.focus();
  };

  const handleTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const last = state.sessions.length - 1;
    let nextIndex: number;
    if (event.key === 'ArrowRight') nextIndex = (index + 1) % state.sessions.length;
    else if (event.key === 'ArrowLeft') nextIndex = (index - 1 + state.sessions.length) % state.sessions.length;
    else if (event.key === 'Home') nextIndex = 0;
    else if (event.key === 'End') nextIndex = last;
    else return;
    event.preventDefault();
    const next = state.sessions[nextIndex];
    if (next) activate(next.terminal_id);
  };

  const closeSession = (terminalId: string) => {
    const ctx = currentEnvelopeContext();
    if (!ctx) {
      terminalStore.reportError('The terminal cannot be closed without a workspace context.');
      return;
    }
    void terminalStore.close(terminalId, ctx);
  };

  const copy = () => {
    const handle = state.active_id ? handles.current.get(state.active_id) : undefined;
    void (handle?.copySelection() ?? Promise.resolve(false));
  };
  const paste = () => {
    const handle = state.active_id ? handles.current.get(state.active_id) : undefined;
    void handle?.pasteClipboard();
  };

  let body: ReactNode;
  if (state.sessions.length > 0) {
    body = (
      <>
        {state.sessions.map((session) => (
          <TerminalView
            key={session.terminal_id}
            session={session}
            active={session.terminal_id === state.active_id}
            register={register}
            onNotice={setNotice}
            onNew={startTerminal}
            onClose={() => closeSession(session.terminal_id)}
          />
        ))}
      </>
    );
  } else if (state.error) {
    body = null;
  } else {
    body = (
      <div className="terminal-empty" role="status">
        <Icon name="terminal" size={20} />
        <h3>{state.creating ? 'Starting shell…' : 'No terminal session is running'}</h3>
        <p>
          {state.creating
            ? 'Waiting for the backend to create a real shell.'
            : 'Start a shell to use the integrated terminal.'}
        </p>
        {!state.creating ? (
          <button type="button" onClick={startTerminal}>
            Start terminal
          </button>
        ) : null}
      </div>
    );
  }

  return (
    <div className="dock-panel terminal-panel" data-testid="terminal-panel">
      <div className="terminal-tabstrip">
        <div className="terminal-tabs" role="tablist" aria-label="Terminal sessions">
          {state.sessions.map((session, index) => {
            const label = terminalTabLabel(session, index);
            const selected = session.terminal_id === state.active_id;
            return (
              <div
                key={session.terminal_id}
                className={`terminal-tab${selected ? ' is-active' : ''}`}
              >
                <button
                  type="button"
                  role="tab"
                  id={`terminal-tab-${session.terminal_id}`}
                  className="terminal-tab-select"
                  aria-selected={selected}
                  aria-controls={`terminal-panel-${session.terminal_id}`}
                  tabIndex={selected ? 0 : -1}
                  title={`${session.shell} — ${session.cwd}`}
                  onClick={() => activate(session.terminal_id)}
                  onKeyDown={(event) => handleTabKeyDown(event, index)}
                >
                  {label}
                </button>
                <button
                  type="button"
                  className="terminal-tab-close"
                  aria-label={`Close ${label}`}
                  title={`Close ${label}`}
                  onClick={() => closeSession(session.terminal_id)}
                >
                  <Icon name="close" size={10} />
                </button>
              </div>
            );
          })}
          <ActionButton
            label="New terminal"
            icon="plus"
            onClick={startTerminal}
            disabled={state.creating}
          />
        </div>
        <div className="dock-toolbar-right">
          <ActionButton label="Copy terminal selection" icon="copy" onClick={copy} disabled={!activeSession} />
          <ActionButton label="Paste into terminal" icon="paste" onClick={paste} disabled={!activeSession} />
        </div>
      </div>

      <div ref={bodyRef} className="terminal-body">
        {state.error ? (
          <div className="terminal-error-banner" role="alert" data-testid="terminal-error">
            <span>{state.error}</span>
            <div className="terminal-error-actions">
              {state.sessions.length === 0 ? (
                <button type="button" onClick={startTerminal}>
                  Retry
                </button>
              ) : null}
              <button type="button" onClick={() => terminalStore.dismissError()}>
                Dismiss
              </button>
            </div>
          </div>
        ) : null}
        {body}
        <span className="terminal-notice" role="status" aria-live="polite">
          {notice}
        </span>
      </div>
    </div>
  );
}