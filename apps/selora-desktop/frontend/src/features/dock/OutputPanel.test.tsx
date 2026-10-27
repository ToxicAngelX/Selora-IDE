import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { OutputPanel } from './OutputPanel';

describe('OutputPanel', () => {
  it('renders an honest empty state instead of fabricated log rows', () => {
    render(<OutputPanel />);

    expect(screen.getByTestId('output-panel')).toBeTruthy();
    expect(screen.getByText('No output yet — tasks land with the terminal slice.')).toBeTruthy();
    expect(screen.queryByText(/rust-analyzer v0/i)).toBeNull();
    expect(screen.queryByText(/Compiling selora-core/i)).toBeNull();
  });
});

