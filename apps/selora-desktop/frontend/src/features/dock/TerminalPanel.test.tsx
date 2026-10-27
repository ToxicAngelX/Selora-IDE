import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { TerminalPanel } from './TerminalPanel';

describe('TerminalPanel', () => {
  it('renders a clearly labeled terminal placeholder with no fake session', () => {
    render(<TerminalPanel />);

    expect(screen.getByTestId('terminal-panel')).toBeTruthy();
    expect(screen.getByText('Terminal placeholder')).toBeTruthy();
    expect(screen.getByText('No terminal session is running')).toBeTruthy();
    expect(screen.getByText('The integrated shell arrives with the terminal slice.')).toBeTruthy();
    expect(screen.queryByLabelText('Terminal command prompt')).toBeNull();
    expect(screen.queryByText(/cargo check/i)).toBeNull();
  });
});

