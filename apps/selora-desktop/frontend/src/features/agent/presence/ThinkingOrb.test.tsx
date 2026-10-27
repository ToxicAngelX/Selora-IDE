// Selora IDE — ThinkingOrb & Presence Components Tests
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import React from 'react';
import { ThinkingOrb } from './ThinkingOrb';
import { BorderBeam } from './BorderBeam';
import { LiquidPill } from './LiquidPill';
import { PromptSuggestions } from '../components/PromptSuggestions';

describe('Presence Components', () => {
  describe('ThinkingOrb', () => {
    it('renders SVG with high-DPI attributes and specified state', () => {
      const { getByRole, container } = render(
        <ThinkingOrb state="working" size={64} isLive={true} />
      );

      const orb = getByRole('img', { name: 'Agent state: working' });
      expect(orb).toBeTruthy();
      expect(orb.classList.contains('orb-working')).toBe(true);
      expect(orb.classList.contains('orb-hero')).toBe(true);

      const svg = container.querySelector('svg');
      expect(svg).toBeTruthy();
      expect(svg?.getAttribute('shape-rendering')).toBe('geometricPrecision');
    });

    it('renders inline 20px status orb', () => {
      const { getByRole } = render(
        <ThinkingOrb state="breathing" size={20} />
      );

      const orb = getByRole('img', { name: 'Agent state: breathing' });
      expect(orb.classList.contains('orb-inline')).toBe(true);
    });
  });

  describe('BorderBeam', () => {
    it('renders active perimeter beam with preset class', () => {
      const { container } = render(
        <BorderBeam active={true} colorPreset="emerald" />
      );

      const beam = container.querySelector('.border-beam-container');
      expect(beam).toBeTruthy();
      expect(beam?.classList.contains('is-active')).toBe(true);
      expect(beam?.classList.contains('preset-emerald')).toBe(true);
      expect(container.querySelector('.border-beam-aura')).toBeTruthy();
    });

    it('returns null when inactive and strength 0', () => {
      const { container } = render(
        <BorderBeam active={false} strength={0} />
      );

      expect(container.querySelector('.border-beam-container')).toBeNull();
    });
  });

  describe('LiquidPill', () => {
    it('renders fluid toggle button and responds to click', () => {
      const onToggle = vi.fn();
      const { getByRole } = render(
        <LiquidPill
          active={true}
          onToggle={onToggle}
          label="Plan Mode"
          activeLabel="Plan Active"
        />
      );

      const switchBtn = getByRole('switch');
      expect(switchBtn.getAttribute('aria-checked')).toBe('true');
      expect(switchBtn.textContent).toContain('Plan Active');

      fireEvent.click(switchBtn);
      expect(onToggle).toHaveBeenCalledTimes(1);
    });
  });

  describe('PromptSuggestions', () => {
    it('renders all four categories with animated pills and invokes onSelect', () => {
      const onSelect = vi.fn();
      const { getByText } = render(
        <PromptSuggestions onSelectPrompt={onSelect} layout="grid" />
      );

      expect(getByText('Fix Bug')).toBeTruthy();
      expect(getByText('Add Tests')).toBeTruthy();
      expect(getByText('Refactor Code')).toBeTruthy();
      expect(getByText('Explain Flow')).toBeTruthy();

      const fixPrompt = getByText('Review the workspace for bugs');
      fireEvent.click(fixPrompt);
      expect(onSelect).toHaveBeenCalledWith('Review the workspace for bugs');
    });
  });
});
