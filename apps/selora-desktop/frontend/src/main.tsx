// Selora IDE — Phase Z2 entry point.
//
// Style imports follow the single convention (ARCHITECTURE §8.3, DESIGN
// §13.1): base.css (which @imports tokens.css) first, then component
// stylesheets. The Z1 one-file styles.css is deleted — its hardcoded
// literals are exactly what the token layer replaces.
//
// Region stylesheets belong to their (lazily loaded) region component:
// sidebar.css is imported by PrimarySidebar and dock.css by BottomDock, so a
// closed region never ships that CSS in first paint. agent.css stays eager:
// the sidebar's agent view renders AgentSessionRail, which can be on screen
// from the first frame.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import './styles/base.css';
import './styles/animations.css';
import './styles/shell.css';
import './styles/agent.css';
import './styles/editor.css';

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
