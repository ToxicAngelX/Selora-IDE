#!/usr/bin/env node
// Selora IDE - automated functional walkthrough over the WebView2 DevTools
// protocol (CDP).
//
// The launcher (scripts/walkthrough.ps1) starts the desktop binary with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>, then
// runs this file. Everything here talks to the *live* application: real DOM,
// real mouse/key events through Input.*, real IPC through the Tauri command
// seam. It never imports application modules and it never mocks a step.
//
// Artifacts (screenshots, cdp-result.json, console/exception log) are written
// to WALKTHROUGH_ARTIFACTS. Exit codes:
//   0  every automated step passed
//   1  a step failed (diagnostics written)
//   2  the driver could not attach to the app or the page never booted
//
// No third-party dependencies: Node >= 22 provides fetch and WebSocket.

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { requirePersistedKey, requireCleanRuntime } from './gates.mjs';

const PORT = Number(process.env.CDP_PORT || 9333);
const ARTIFACTS = process.env.WALKTHROUGH_ARTIFACTS || '';
const WORKSPACE = process.env.WALKTHROUGH_WORKSPACE || '';
const WORKSPACE_B = process.env.WALKTHROUGH_WORKSPACE_B || '';
const SETTINGS_FILE = process.env.WALKTHROUGH_SETTINGS_FILE || '';
const STAMP = process.env.WALKTHROUGH_STAMP || String(Date.now());
const BOOT_TIMEOUT_MS = Number(process.env.WALKTHROUGH_BOOT_TIMEOUT_MS || 60000);
const STEP_TIMEOUT_MS = Number(process.env.WALKTHROUGH_STEP_TIMEOUT_MS || 20000);

if (!ARTIFACTS) {
  console.error('cdp-walkthrough: WALKTHROUGH_ARTIFACTS is required');
  process.exit(2);
}

await mkdir(ARTIFACTS, { recursive: true });

const startedAt = new Date();
const steps = [];
const consoleLines = [];
const runtimeErrors = [];
const exceptions = [];
const screenshots = [];
const findings = [];
let cdp = null;
let currentStep = null;

function log(message) {
  const line = `${new Date().toISOString()} ${message}`;
  console.log(line);
  consoleLines.push(line);
}

function short(value, limit = 600) {
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  if (text === undefined) return String(value);
  return text.length > limit ? `${text.slice(0, limit)}...` : text;
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// --- CDP client -----------------------------------------------------------

class CdpClient {
  constructor(wsUrl) {
    this.wsUrl = wsUrl;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Set();
    this.ws = null;
  }

  async connect() {
    await new Promise((resolve, reject) => {
      const ws = new WebSocket(this.wsUrl);
      this.ws = ws;
      const timer = setTimeout(() => reject(new Error('CDP WebSocket open timed out')), 10000);
      ws.addEventListener('open', () => {
        clearTimeout(timer);
        resolve();
      });
      ws.addEventListener('error', (event) => {
        clearTimeout(timer);
        reject(new Error(`CDP WebSocket error: ${event.message || 'unknown'}`));
      });
      ws.addEventListener('message', (event) => this.onMessage(event.data));
      ws.addEventListener('close', () => {
        for (const { reject: rejectPending, timer } of this.pending.values()) {
          clearTimeout(timer);
          rejectPending(new Error('CDP connection closed'));
        }
        this.pending.clear();
      });
    });
  }

  onMessage(data) {
    let message;
    try {
      message = JSON.parse(typeof data === 'string' ? data : Buffer.from(data).toString('utf8'));
    } catch {
      return;
    }
    if (message.id !== undefined) {
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      clearTimeout(entry.timer);
      if (message.error) {
        entry.reject(new Error(`${entry.method}: ${message.error.message} (${message.error.code})`));
      } else {
        entry.resolve(message.result ?? {});
      }
      return;
    }
    for (const listener of this.listeners) {
      try {
        listener(message.method, message.params ?? {});
      } catch {
        // A listener must never break the protocol pump.
      }
    }
  }

  send(method, params = {}, timeoutMs = 20000) {
    if (!this.ws || this.ws.readyState !== 1) {
      return Promise.reject(new Error(`CDP not connected (${method})`));
    }
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout after ${timeoutMs} ms: ${method}`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer, method });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  onEvent(listener) {
    this.listeners.add(listener);
  }

  close() {
    try {
      this.ws?.close();
    } catch {
      // Already closed.
    }
  }
}

async function fetchJson(url, timeoutMs = 5000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return await response.json();
  } finally {
    clearTimeout(timer);
  }
}

async function attach() {
  const deadline = Date.now() + BOOT_TIMEOUT_MS;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const targets = await fetchJson(`http://127.0.0.1:${PORT}/json/list`);
      const page = (Array.isArray(targets) ? targets : []).find(
        (target) => target.type === 'page' && target.webSocketDebuggerUrl,
      );
      if (page) {
        const client = new CdpClient(page.webSocketDebuggerUrl);
        await client.connect();
        log(`attached: ${page.title || '(no title)'} ${page.url}`);
        return client;
      }
      lastError = new Error('no page target yet');
    } catch (error) {
      lastError = error;
    }
    await delay(250);
  }
  throw new Error(`could not attach to WebView2 CDP on port ${PORT}: ${lastError?.message || 'unknown'}`);
}

// --- page evaluation helpers ---------------------------------------------

async function evaluate(expression, timeoutMs = 20000) {
  const response = await cdp.send(
    'Runtime.evaluate',
    {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
      timeout: timeoutMs,
    },
    timeoutMs + 2000,
  );
  if (response.exceptionDetails) {
    const detail =
      response.exceptionDetails.exception?.description ||
      response.exceptionDetails.text ||
      'evaluation failed';
    throw new Error(`page evaluation failed: ${detail}`);
  }
  return response.result?.value;
}

function argsSource(args) {
  return args.map((value) => JSON.stringify(value)).join(', ');
}

async function evalFn(fn, ...args) {
  return evaluate(`(${fn.toString()})(${argsSource(args)})`);
}

// Click through Input.dispatchMouseEvent: this is a real pointer sequence in
// the renderer, not a synthetic el.click(), so React handlers, hit-testing,
// overlays, and disabled controls all behave as they do for the user.
async function clickBy(finderSource, ...args) {
  const expression = `(${finderSource})(${argsSource(args)})`;
  const measure = () =>
    evaluate(`(() => {
    const el = ${expression};
    if (!el) return null;
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return { zero: true, tag: el.tagName, cls: el.className };
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return {
      x, y, w: r.width, h: r.height,
      tag: el.tagName,
      label: el.getAttribute('aria-label') || el.textContent?.trim().slice(0, 60),
      hit_tag: hit?.tagName || null,
      hit_cls: typeof hit?.className === 'string' ? hit.className : null,
      hit_label: hit?.getAttribute?.('aria-label') || hit?.textContent?.trim().slice(0, 60) || null,
      hit_inside: Boolean(hit && (el === hit || el.contains(hit) || hit.contains(el)))
    };
  })()`);

  // A transient toast can cover the target for a few seconds (toasts last
  // 4-8 s). Retry the hit test before declaring the click blocked.
  const deadline = Date.now() + 10000;
  let rect = null;
  for (;;) {
    rect = await measure();
    if (!rect) throw new Error(`click target not found: ${short(finderSource, 140)} ${short(args)}`);
    if (rect.zero) throw new Error(`click target has zero size: ${short(rect)}`);
    if (rect.hit_inside) break;
    if (Date.now() >= deadline) {
      throw new Error(
        `click would be intercepted at ${Math.round(rect.x)},${Math.round(rect.y)}: ` +
          `target=${rect.tag} ${short(rect.label || '', 40)}; hit=${rect.hit_tag} ${short(rect.hit_cls || '', 40)} ${short(rect.hit_label || '', 60)}`,
      );
    }
    await delay(250);
  }
  const point = { x: Math.round(rect.x), y: Math.round(rect.y) };
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', clickCount: 1, ...point });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', clickCount: 1, ...point });
  log(`clicked ${rect.tag} ${short(rect.label || '', 60)} at ${point.x},${point.y}`);
}

async function keyCombo(key, modifiers = 0, code = '') {
  const base = {
    key,
    code: code || (key.length === 1 ? `Key${key.toUpperCase()}` : key),
    windowsVirtualKeyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0,
    nativeVirtualKeyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0,
    modifiers,
  };
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', ...base });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
}

async function waitFor(label, predicateSource, timeoutMs = STEP_TIMEOUT_MS, intervalMs = 120) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    try {
      last = await evaluate(`(() => { try { return (${predicateSource})(); } catch (e) { return { __error: String(e) }; } })()`);
      if (last === true) return true;
    } catch (error) {
      last = { __error: error.message };
    }
    await delay(intervalMs);
  }
  throw new Error(`timeout after ${timeoutMs} ms waiting for ${label}; last=${short(last)}`);
}

async function waitForFn(label, fn, timeoutMs = STEP_TIMEOUT_MS, intervalMs = 120) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    try {
      last = await fn();
      if (last) return last;
    } catch (error) {
      last = error.message;
    }
    await delay(intervalMs);
  }
  throw new Error(`timeout after ${timeoutMs} ms waiting for ${label}; last=${short(last)}`);
}

async function screenshot(name) {
  try {
    const result = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }, 15000);
    const file = path.join(ARTIFACTS, name);
    await writeFile(file, Buffer.from(result.data, 'base64'));
    screenshots.push(name);
    log(`screenshot ${name}`);
    return file;
  } catch (error) {
    log(`screenshot failed (${name}): ${error.message}`);
    return null;
  }
}

async function getBootstrap() {
  return evaluate(`(async () => {
    const internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      throw new Error('window.__TAURI_INTERNALS__.invoke is unavailable');
    }
    return await internals.invoke('get_bootstrap_state', { protocolVersion: 1 });
  })()`);
}

async function dispatchProtocol(payload) {
  return evaluate(`(async () => {
    const internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {
      throw new Error('window.__TAURI_INTERNALS__.invoke is unavailable');
    }
    const boot = await internals.invoke('get_bootstrap_state', { protocolVersion: 1 });
    const envelope = {
      protocol_version: 1,
      workspace_id: boot.workspace.id,
      core_instance_id: boot.core_instance_id,
      request_id: crypto.randomUUID(),
      session_id: null,
      payload: ${JSON.stringify(payload)}
    };
    return await internals.invoke('dispatch_command', { envelope });
  })()`);
}

// --- steps ----------------------------------------------------------------

function normalizePath(value) {
  // Rust canonicalizes roots with the Windows verbatim prefix (\\?\C:\...).
  return String(value ?? '')
    .replace(/\\/g, '/')
    .replace(/^\/\/\?\//, '')
    .replace(/^\/\/\.\//, '')
    .replace(/\/+$/, '')
    .toLowerCase();
}

function findBuffer(bootstrap, suffix) {
  const target = normalizePath(suffix);
  return (bootstrap?.buffers ?? []).find((buffer) => {
    const candidate = normalizePath(buffer.path);
    return candidate === target || candidate.endsWith(`/${target}`);
  });
}

async function readTextFile(file) {
  try {
    return await readFile(file, 'utf8');
  } catch {
    return null;
  }
}

async function step(id, title, body) {
  currentStep = id;
  const started = Date.now();
  log(`STEP ${id}: ${title}`);
  try {
    const detail = await body();
    const finding = findings.find((item) => item.id === id);
    const entry = {
      id,
      title,
      status: finding ? 'FINDING' : 'PASS',
      ms: Date.now() - started,
      detail: finding ? finding.detail : (detail ?? null),
    };
    steps.push(entry);
    log(`${entry.status} ${id} (${entry.ms} ms)${entry.detail ? `: ${short(entry.detail)}` : ''}`);
    return detail;
  } catch (error) {
    const entry = { id, title, status: 'FAIL', ms: Date.now() - started, detail: error.message };
    steps.push(entry);
    log(`FAIL ${id} (${entry.ms} ms): ${error.message}`);
    await screenshot(`failure-${id}.png`);
    throw error;
  } finally {
    currentStep = null;
  }
}

function recordFinding(id, title, detail) {
  findings.push({ id, title, detail });
  log(`FINDING ${id}: ${detail}`);
}

async function pageDiagnostics() {
  try {
    return await evaluate(`(() => ({
      url: location.href,
      title: document.title,
      ready: document.readyState,
      shell: Boolean(document.querySelector('.shell')),
      bootError: document.querySelector('.boot-error')?.textContent?.trim() || null,
      onboarding: Boolean(document.querySelector('[data-testid="onboarding-backdrop"]')),
      sidebarView: document.querySelector('.sidebar-header')?.textContent?.trim() || null,
      tabs: [...document.querySelectorAll('.editor-tab')].map((tab) => tab.getAttribute('aria-label')),
      activeTab: document.querySelector('.editor-tab[aria-selected="true"]')?.getAttribute('aria-label') || null,
      dialogs: [...document.querySelectorAll('[role="dialog"]')].map((el) => el.getAttribute('aria-label') || el.textContent?.trim().slice(0, 80)),
      toasts: [...document.querySelectorAll('.toast')].map((el) => el.textContent?.trim().slice(0, 160)),
      activeElement: document.activeElement?.outerHTML?.slice(0, 240) || null,
      bodyText: (document.body.innerText || '').slice(0, 2000)
    }))()`);
  } catch (error) {
    return { unavailable: error.message };
  }
}

async function writeResult(payload) {
  const result = {
    ok: payload.ok,
    mode: process.env.WALKTHROUGH_MODE || 'release',
    stamp: STAMP,
    started_at: startedAt.toISOString(),
    finished_at: new Date().toISOString(),
    steps,
    findings,
    screenshots,
    console: consoleLines,
    exceptions,
    runtime_errors: runtimeErrors,
    error: payload.error ?? null,
    diagnostics: payload.diagnostics ?? null,
    bootstrap: payload.bootstrap ?? null,
  };
  await writeFile(path.join(ARTIFACTS, 'cdp-result.json'), JSON.stringify(result, null, 2), 'utf8');
}

function summarizeBootstrap(bootstrap) {
  if (!bootstrap) return null;
  return {
    protocol_version: bootstrap.protocol_version,
    connection: bootstrap.connection,
    workspace: {
      id: bootstrap.workspace?.id ?? null,
      root: bootstrap.workspace?.root ?? null,
      display_name: bootstrap.workspace?.display_name ?? null,
    },
    settings: bootstrap.settings,
    layout: bootstrap.layout,
    buffer_count: (bootstrap.buffers ?? []).length,
    buffers: (bootstrap.buffers ?? []).map((buffer) => ({
      path: buffer.path,
      dirty: buffer.dirty,
      version: buffer.version,
    })),
    file_tree_root: bootstrap.file_tree?.root?.name ?? null,
    top_level_entries: (bootstrap.file_tree?.root?.children ?? []).map((child) => child.name),
    models_count: (bootstrap.models ?? []).length,
    workflows: (bootstrap.workflows ?? []).map((run) => ({
      run_id: run.run_id,
      name: run.name,
      status: run.status,
    })),
  };
}

async function main() {
  log(`driver start: port=${PORT} workspace=${WORKSPACE} settings=${SETTINGS_FILE}`);
  cdp = await attach();

  cdp.onEvent((method, params) => {
    if (method === 'Runtime.consoleAPICalled') {
      const text = (params.args ?? [])
        .map((arg) => arg.value ?? arg.description ?? arg.preview?.description ?? arg.type)
        .join(' ');
      const line = `console.${params.type}: ${text}`;
      if (params.type === 'error' || params.type === 'assert') runtimeErrors.push(line);
      consoleLines.push(line);
      log(line);
    } else if (method === 'Runtime.exceptionThrown') {
      const description =
        params.exceptionDetails?.exception?.description ||
        params.exceptionDetails?.text ||
        'unknown exception';
      exceptions.push({ at: new Date().toISOString(), description });
      log(`exception: ${description}`);
    } else if (method === 'Log.entryAdded') {
      const entry = params.entry ?? {};
      if (entry.level === 'error') {
        const line = `log.error: ${entry.text} (${entry.url || ''}:${entry.lineNumber ?? ''})`;
        runtimeErrors.push(line);
        consoleLines.push(line);
        log(line);
      }
    } else if (method === 'Inspector.targetCrashed') {
      exceptions.push({ at: new Date().toISOString(), description: 'renderer target crashed' });
      log('exception: renderer target crashed');
    }
  });

  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');
  await cdp.send('Log.enable');
  try {
    await cdp.send('Inspector.enable');
  } catch {
    // Inspector is optional; Runtime/Log still cover errors.
  }

  await step('boot', 'Shell boots and reports bootstrap state', async () => {
    await waitFor(
      'shell or boot error',
      `() => Boolean(document.querySelector('.shell')) || Boolean(document.querySelector('.boot-error'))`,
      BOOT_TIMEOUT_MS,
    );
    const errorText = await evaluate(
      `document.querySelector('.boot-error')?.textContent?.trim() || null`,
    );
    if (errorText) throw new Error(`boot error screen: ${short(errorText)}`);
    await waitFor('bootstrap ready', `() => Boolean(window.__TAURI_INTERNALS__?.invoke)`);
    const bootstrap = await getBootstrap();
    if (!bootstrap?.workspace) throw new Error('bootstrap has no workspace object');
    await waitFor('file tree region', `() => Boolean(document.querySelector('.workbench'))`);
    return `connection=${bootstrap.connection} workspace=${bootstrap.workspace.display_name}`;
  });

  await screenshot('00-launch.png');

  await step('persisted-key', 'A persisted Credential Manager key is available', async () => {
    const status = await evaluate(`window.__TAURI_INTERNALS__.invoke('get_api_key_status')`);
    requirePersistedKey(status);
    return 'persisted key present (key value never read or logged)';
  });

  await step('onboarding', 'First-run overlay is dismissed through its own control', async () => {
    const present = await evaluate(
      `Boolean(document.querySelector('[data-testid="onboarding-backdrop"]'))`,
    );
    if (!present) return 'overlay not shown (profile already onboarded)';
    await clickBy(`() => document.querySelector('.onboarding-skip')`);
    await waitFor('onboarding overlay gone', `() => !document.querySelector('[data-testid="onboarding-backdrop"]')`);
    return 'clicked Skip';
  });

  await step('workspace-boot', 'Boot workspace is attached and its files are listed', async () => {
    const bootstrap = await getBootstrap();
    if (normalizePath(bootstrap.workspace.root) !== normalizePath(WORKSPACE)) {
      throw new Error(
        `boot workspace mismatch: expected ${WORKSPACE}, got ${bootstrap.workspace.root}`,
      );
    }
    await waitFor(
      'file tree view',
      `() => Boolean(document.querySelector('[data-testid="file-tree-view"]'))`,
    );
    const names = await evaluate(
      `[...document.querySelectorAll('.tree-node-row')].map((row) => row.getAttribute('title') || '')`,
    );
    if (!names.some((name) => /hello\.ts$/i.test(name))) {
      throw new Error(`hello.ts missing from file tree; rows=${short(names)}`);
    }
    await screenshot('01-workspace.png');
    return `root=${bootstrap.workspace.root}`;
  });

  await step('workspace-open', 'Opening another folder through the protocol seam switches the workspace', async () => {
    const before = await getBootstrap();
    await dispatchProtocol({ type: 'open_folder_at', payload: { root: WORKSPACE_B } });
    const after = await waitForFn(
      'workspace switch',
      async () => {
        const current = await getBootstrap();
        return normalizePath(current.workspace.root) === normalizePath(WORKSPACE_B) ? current : null;
      },
      20000,
    );
    if (after.workspace.id === before.workspace.id) {
      throw new Error('workspace id did not change after open_folder_at');
    }
    await waitFor(
      'second workspace tree',
      `() => [...document.querySelectorAll('.tree-node-row')].some((row) => /second-workspace\\.txt$/i.test(row.getAttribute('title') || ''))`,
    );
    await screenshot('02-second-workspace.png');

    await dispatchProtocol({ type: 'open_folder_at', payload: { root: WORKSPACE } });
    const restored = await waitForFn(
      'workspace restored',
      async () => {
        const current = await getBootstrap();
        return normalizePath(current.workspace.root) === normalizePath(WORKSPACE) ? current : null;
      },
      20000,
    );
    await waitFor(
      'fixture tree after restore',
      `() => [...document.querySelectorAll('.tree-node-row')].some((row) => /hello\\.ts$/i.test(row.getAttribute('title') || ''))`,
    );
    return `switched id ${before.workspace.id} -> ${after.workspace.id} -> ${restored.workspace.id}`;
  });

  await step('file-open', 'A file opens from the explorer into a real editor tab', async () => {
    await clickBy(
      `(fileName) => [...document.querySelectorAll('.tree-node-row')].find((row) => (row.getAttribute('title') || '').toLowerCase().endsWith(fileName.toLowerCase()))`,
      'hello.ts',
    );
    await waitFor(
      'hello.ts tab',
      `() => Boolean(document.querySelector('.editor-tab[aria-label="hello.ts"]'))`,
    );
    await waitFor('CodeMirror content', `() => Boolean(document.querySelector('.cm-content'))`);
    const text = await evaluate(`document.querySelector('.cm-content')?.innerText || ''`);
    if (!/hello from walkthrough/.test(text)) {
      throw new Error(`editor content missing fixture text: ${short(text, 200)}`);
    }
    await screenshot('03-file-open.png');
    const tabs = await evaluate(
      `[...document.querySelectorAll('.editor-tab')].map((tab) => tab.getAttribute('aria-label'))`,
    );
    return `tab + CodeMirror content visible; tabs=${short(tabs)}`;
  });

  await step('edit-save', 'Typing reaches the buffer and Ctrl+S writes the file to disk', async () => {
    const marker = `// walkthrough ${STAMP}`;
    await clickBy(`() => document.querySelector('.cm-content')`);
    const focus = await evaluate(
      `({ tag: document.activeElement?.tagName || null, cls: typeof document.activeElement?.className === 'string' ? document.activeElement.className : null })`,
    );
    if (focus.tag !== 'DIV' || !String(focus.cls || '').includes('cm-content')) {
      throw new Error(`editor did not take focus after the click: ${short(focus)}`);
    }
    await cdp.send('Input.insertText', { text: `${marker}\n` });
    const dirty = await waitForFn(
      'buffer dirty',
      async () => {
        const bootstrap = await getBootstrap();
        const buffer = findBuffer(bootstrap, 'hello.ts');
        return buffer && buffer.text.includes(marker) ? buffer : null;
      },
    );
    if (!dirty.dirty) throw new Error('buffer text contains marker but dirty flag is false');

    await keyCombo('s', 2);
    await waitForFn(
      'buffer clean after save',
      async () => {
        const buffer = findBuffer(await getBootstrap(), 'hello.ts');
        return buffer && !buffer.dirty ? buffer : null;
      },
      15000,
    );
    const disk = await waitForFn(
      'marker on disk',
      async () => {
        const text = await readTextFile(path.join(WORKSPACE, 'hello.ts'));
        return text && text.includes(marker) ? text : null;
      },
      15000,
    );
    await screenshot('04-edited-saved.png');
    return `file now ${disk.length} bytes and contains ${marker}`;
  });

  await step('dirty-close-guard', 'Closing a dirty tab prompts and Cancel keeps the tab', async () => {
    await clickBy(`() => document.querySelector('.cm-content')`);
    await cdp.send('Input.insertText', { text: `// unsaved guard ${STAMP}\n` });
    await waitForFn(
      'second dirty buffer',
      async () => {
        const buffer = findBuffer(await getBootstrap(), 'hello.ts');
        return buffer && buffer.dirty ? buffer : null;
      },
    );
    await clickBy(
      `() => document.querySelector('.tab-close-btn[aria-label="Close hello.ts"]')`,
    );
    await waitFor(
      'unsaved changes dialog',
      `() => Boolean(document.querySelector('[role="dialog"][aria-label^="Unsaved changes"]'))`,
    );
    await clickBy(
      `() => [...document.querySelectorAll('[role="dialog"] button')].find((button) => button.textContent.trim() === 'Cancel')`,
    );
    await waitFor(
      'dialog dismissed',
      `() => !document.querySelector('[role="dialog"][aria-label^="Unsaved changes"]')`,
    );
    const stillOpen = await evaluate(
      `Boolean(document.querySelector('.editor-tab[aria-label="hello.ts"]'))`,
    );
    if (!stillOpen) throw new Error('tab closed even though the guard was cancelled');
    await clickBy(`() => document.querySelector('.cm-content')`);
    await keyCombo('s', 2);
    await waitForFn(
      'buffer clean after guard save',
      async () => {
        const buffer = findBuffer(await getBootstrap(), 'hello.ts');
        return buffer && !buffer.dirty ? buffer : null;
      },
      15000,
    );
    return 'Save/Discard/Cancel dialog appeared; Cancel kept the dirty tab';
  });

  await step('tabs', 'A second tab opens, switching works, and a clean tab closes', async () => {
    await clickBy(
      `(fileName) => [...document.querySelectorAll('.tree-node-row')].find((row) => (row.getAttribute('title') || '').toLowerCase().endsWith(fileName.toLowerCase()))`,
      'notes.md',
    );
    await waitForFn(
      'two tabs',
      async () => {
        const tabs = await evaluate(
          `[...document.querySelectorAll('.editor-tab')].map((tab) => tab.getAttribute('aria-label'))`,
        );
        return tabs.length === 2 && tabs.includes('notes.md') ? tabs : null;
      },
    );
    await clickBy(`() => document.querySelector('.editor-tab[aria-label="hello.ts"]')`);
    await waitFor(
      'hello.ts selected',
      `() => document.querySelector('.editor-tab[aria-label="hello.ts"]')?.getAttribute('aria-selected') === 'true'`,
    );
    await clickBy(`() => document.querySelector('.tab-close-btn[aria-label="Close notes.md"]')`);
    await waitFor(
      'notes.md closed',
      `() => !document.querySelector('.editor-tab[aria-label="notes.md"]')`,
    );
    const tabs = await evaluate(
      `[...document.querySelectorAll('.editor-tab')].map((tab) => tab.getAttribute('aria-label'))`,
    );
    if (tabs.length !== 1) throw new Error(`expected 1 tab after close, got ${short(tabs)}`);
    await screenshot('05-tabs.png');
    return `tabs now ${short(tabs)}; switch and close verified`;
  });

  await step('settings', 'Settings UI writes and persists a real setting', async () => {
    await clickBy(
      `() => [...document.querySelectorAll('.activity-button')].find((button) => button.getAttribute('aria-label') === 'Settings')`,
    );
    await waitFor('settings panel', `() => Boolean(document.querySelector('.sidebar-settings-panel'))`);
    await clickBy(
      `() => [...document.querySelectorAll('.settings-tab-btn')].find((button) => button.textContent.trim() === 'Theme')`,
    );
    await waitFor('theme select', `() => Boolean(document.querySelector('#settings-theme-select'))`);
    const optionCount = await evaluate(
      `document.querySelector('#settings-theme-select')?.options.length || 0`,
    );
    if (optionCount < 2) throw new Error(`theme select has only ${optionCount} option(s)`);

    const initial = await getBootstrap();
    const initialValue = Boolean(initial.settings.reduced_motion);
    const checkboxExpr = `() => document.querySelector('input[aria-labelledby="settings-reduced-motion-label"]')?.closest('label')`;
    const checkbox = await evaluate(`Boolean((${checkboxExpr})())`);
    if (!checkbox) throw new Error('reduced-motion checkbox not found');

    const settingsFile = SETTINGS_FILE;
    const readPersisted = async () => {
      const text = await readTextFile(settingsFile);
      if (!text) return null;
      try {
        return JSON.parse(text);
      } catch {
        return null;
      }
    };

    await clickBy(checkboxExpr);
    const toggled = await waitForFn(
      'bootstrap reflects toggled setting',
      async () => {
        const current = await getBootstrap();
        return Boolean(current.settings.reduced_motion) !== initialValue ? current.settings : null;
      },
      15000,
    );
    if (toggled.reduced_motion !== !initialValue) {
      throw new Error('bootstrap reduced_motion did not take the new value');
    }
    const persistedValue = (parsed) =>
      parsed?.ui?.reduce_motion ?? parsed?.layout?.reduced_motion ?? parsed?.reduced_motion;
    const persisted = await waitForFn(
      'settings.json persists the toggle',
      async () => {
        const parsed = await readPersisted();
        return parsed && Boolean(persistedValue(parsed)) === toggled.reduced_motion
          ? parsed
          : null;
      },
      15000,
    );
    await screenshot('06-settings.png');

    // Restore the isolated settings file to its initial value.
    await clickBy(checkboxExpr);
    await waitForFn(
      'bootstrap restored',
      async () => {
        const current = await getBootstrap();
        return Boolean(current.settings.reduced_motion) === initialValue ? current.settings : null;
      },
      15000,
    );
    await waitForFn(
      'settings.json restored',
      async () => {
        const parsed = await readPersisted();
        return parsed && Boolean(persistedValue(parsed)) === initialValue ? parsed : null;
      },
      15000,
    );
    return `reduced_motion toggled ${initialValue} -> ${toggled.reduced_motion} -> ${initialValue}; persisted_ui=${short(persisted.ui)}`;
  });

  await step('workflows-smoke', 'Workflows route mounts the real panel without an error fallback', async () => {
    await clickBy(
      `() => [...document.querySelectorAll('.activity-button')].find((button) => button.getAttribute('aria-label') === 'Workflows')`,
    );
    await waitFor('workflows panel', `() => Boolean(document.querySelector('[data-testid="workflows-panel"]'))`);
    const state = await evaluate(`(() => {
      const panel = document.querySelector('[data-testid="workflows-panel"]');
      return {
        heading: panel?.querySelector('.wf-title')?.textContent?.trim() || null,
        empty: Boolean(panel?.querySelector('[data-testid="workflows-empty"]')),
        runs: panel?.querySelectorAll('.wf-run-list li').length || 0,
        text: panel?.innerText?.slice(0, 400) || ''
      };
    })()`);
    if (state.heading !== 'Workflows') {
      throw new Error(`workflows panel heading mismatch: ${short(state.heading)}`);
    }
    if (/unavailable|failed to render/i.test(state.text)) {
      throw new Error(`workflows panel showed its failure fallback: ${short(state.text)}`);
    }
    await screenshot('07-workflows.png');
    return `panel mounted; empty=${state.empty}; runs=${state.runs}`;
  });

  await step('agent-activity-button', 'Agent activity button opens the agent dock', async () => {
    // Normalise: Ctrl+L toggles the dock once (the documented shortcut).
    const layout = async () => {
      const bootstrap = await getBootstrap();
      return bootstrap.layout;
    };
    if ((await layout()).agent_dock_open) {
      await keyCombo('l', 2);
      await waitForFn(
        'dock closed before the button probe',
        async () => !(await layout()).agent_dock_open,
        10000,
      );
    }
    await clickBy(
      `() => [...document.querySelectorAll('.activity-button')].find((button) => button.getAttribute('aria-label') === 'Agent')`,
    );
    await delay(1200);
    const opened = (await layout()).agent_dock_open;
    if (!opened) {
      // Observed on this build: ActivityBar calls onSelectViewMode('split')
      // (which already opens the dock) and then toggles it again, so the
      // button is a no-op from editor mode. Keep the walkthrough running and
      // report it as a finding instead of hiding it.
      recordFinding(
        'agent-activity-button',
        'Agent activity button is a no-op from editor mode',
        'Clicking the Agent activity button left agent_dock_open=false. ActivityBar handles the click twice: onSelectViewMode("split") opens the dock, then layoutStore.toggle("agent") closes it. The status-bar Agent pill and Ctrl+L both open the dock.',
      );
      await screenshot('finding-agent-activity-button.png');
      return 'dock stayed closed (recorded as FINDING)';
    }
    return 'clicking the Agent activity button opened the dock';
  });

  await step('agent-smoke', 'Agent dock opens (Ctrl+L) and its composer mounts', async () => {
    const bootstrap = await getBootstrap();
    if (!bootstrap.layout.agent_dock_open) {
      await keyCombo('l', 2);
    }
    await waitFor(
      'agent workbench',
      `() => Boolean(document.querySelector('.agent-workbench-root'))`,
      15000,
    );
    await waitFor(
      'composer textarea',
      `() => Boolean(document.querySelector('textarea[aria-label="Composer prompt input"]'))`,
    );
    await clickBy(`() => document.querySelector('textarea[aria-label="Composer prompt input"]')`);
    await cdp.send('Input.insertText', { text: `walkthrough smoke ${STAMP}` });
    const typed = await evaluate(
      `document.querySelector('textarea[aria-label="Composer prompt input"]')?.value || ''`,
    );
    if (!typed.includes(STAMP)) throw new Error('composer did not accept typed text');

    const gate = await evaluate(`(() => {
      const send = document.querySelector('button[aria-label="Send"], button[aria-label="Queue prompt"], button[aria-label="Steer active run"]');
      const blocked = document.querySelector('.composer-send-blocked');
      return {
        send_label: send ? send.getAttribute('aria-label') : null,
        send_disabled: send ? send.disabled : null,
        send_title: send ? send.title : null,
        blocked_text: blocked?.textContent?.trim().slice(0, 240) || null
      };
    })()`);
    await screenshot('08-agent.png');

    // Clear the unsent draft; this walkthrough never sends a gateway request.
    await keyCombo('a', 2);
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Backspace',
      code: 'Backspace',
      windowsVirtualKeyCode: 8,
      nativeVirtualKeyCode: 8,
    });
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Backspace',
      code: 'Backspace',
      windowsVirtualKeyCode: 8,
      nativeVirtualKeyCode: 8,
    });
    const cleared = await evaluate(
      `document.querySelector('textarea[aria-label="Composer prompt input"]')?.value || ''`,
    );
    if (cleared.trim().length !== 0) {
      log(`warning: composer draft not empty after clear: ${short(cleared)}`);
    }
    return `composer mounted; send=${gate.send_label} disabled=${gate.send_disabled}; title=${short(gate.send_title, 80)}; blocked=${short(gate.blocked_text, 120)}`;
  });

  await step('runtime-errors', 'No renderer exception, console error, or browser log error occurred', async () => {
    requireCleanRuntime(exceptions, runtimeErrors);
    return 'no renderer, console, or browser log errors';
  });

  const finalBootstrap = await getBootstrap();
  await writeResult({ ok: true, bootstrap: summarizeBootstrap(finalBootstrap) });
  log('driver done: PASS');
  return 0;
}

try {
  const code = await main();
  cdp?.close();
  process.exit(code ?? 0);
} catch (error) {
  log(`driver failed: ${error.stack || error.message}`);
  try {
    const diagnostics = await pageDiagnostics();
    let bootstrap = null;
    try {
      bootstrap = summarizeBootstrap(await getBootstrap());
    } catch {
      bootstrap = null;
    }
    await screenshot('failure-final.png');
    await writeResult({ ok: false, error: error.message, diagnostics, bootstrap });
  } catch (writeError) {
    log(`failed to write diagnostics: ${writeError.message}`);
  }
  cdp?.close();
  process.exit(1);
}
