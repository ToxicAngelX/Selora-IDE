import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { getBootstrapState, listenProtocolEvent, type BootstrapState } from './bridge';
import './styles.css';

function App() {
  const [state, setState] = useState<BootstrapState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(true);
  const load = () => { setConnecting(true); setError(null); void getBootstrapState().then(setState).catch((cause: unknown) => setError(cause instanceof Error ? cause.message : 'Bootstrap failed')).finally(() => setConnecting(false)); };
  useEffect(() => { load(); let unlisten: (() => void) | undefined; void listenProtocolEvent(() => load()).then(fn => { unlisten = fn; }); return () => unlisten?.(); }, []);
  if (connecting) return <main className="state" aria-live="polite"><span className="orb" aria-hidden="true" />Connecting to Rust core…</main>;
  if (error) return <main className="state error" role="alert"><h1>Selora IDE</h1><p>{error}</p><button onClick={load}>Reconnect</button></main>;
  if (!state) return null;
  return <main className="shell"><header><span className="mark" aria-hidden="true" /> <strong>Selora IDE</strong><span className="muted">diagnostic shell · protocol v{state.protocol_version}</span></header><section className="grid"><aside><h2>Workspace</h2><p>{state.workspace.display_name}</p><p className="muted">{state.workspace.root ?? 'No folder open'}</p><h2>Rust status</h2><p><span className={`dot ${state.connection}`} /> {state.connection}</p><p>Core epoch <code>{state.core_instance_id.slice(0, 8)}</code></p></aside><article><h1>Shell foundation ready</h1><p>Rust returned a fresh bootstrap snapshot. Editing, agent, Git, and LSP surfaces are scaffolded for later vertical slices.</p><div className="cards"><div><b>Models</b><span>{state.models.length || 'none loaded'}</span></div><div><b>Feature flags</b><span>{state.feature_flags.later_slices.join(' · ')}</span></div><div><b>Snapshot</b><span>sequence {state.snapshot_sequence}</span></div></div></article></section><footer>WebView2 target · reconnect-safe bootstrap · no secrets in frontend state</footer></main>;
}

createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);

