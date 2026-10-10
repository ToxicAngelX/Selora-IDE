// Selora IDE — Phase Z2 entry point.
//
// Style imports follow the single convention (ARCHITECTURE §8.3, DESIGN
// §13.1): base.css (which @imports tokens.css) first, then component
// stylesheets. The Z1 one-file styles.css is deleted — its hardcoded
// literals are exactly what the token layer replaces.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import './styles/base.css';
import './styles/shell.css';
import './styles/agent.css';

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
