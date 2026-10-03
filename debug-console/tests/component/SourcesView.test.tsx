import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';
import { SourcesView } from '../../src/features/SourcesView';
import { DEMO_SOURCES } from '../../src/features/sourcesModel';
import { ES_419 } from '../../src/i18n/es419';

afterEach(cleanup);

describe('SourcesView', () => {
  it('shows a stand_in demo banner stating the data is not from a real API', () => {
    render(<SourcesView />);
    const b = screen.getByTestId('sources-standin');
    expect(b.getAttribute('data-level')).toBe('stand_in');
    expect(b.textContent).toContain('DEMO');
    expect(b.textContent).toMatch(/no es una API real/);
  });
  it('renders one labelled section per source with its phase level', () => {
    render(<SourcesView />);
    for (const s of DEMO_SOURCES) expect(screen.getByTestId(`source-${s.id}`).getAttribute('aria-labelledby')).toBeTruthy();
    expect(screen.getByTestId('source-platform_live').textContent).toContain('platform_live.phase1/1');
  });
  it('capability table is accessible and shows presence as text, not colour', () => {
    render(<SourcesView />);
    const t = within(screen.getByTestId('source-platform_live')).getByRole('table', { name: /capacidades/i });
    expect(within(t).getAllByRole('columnheader').length).toBeGreaterThanOrEqual(2);
    expect(screen.getByTestId('cap-platform_live-tool_call').textContent).toContain('absent');
    expect(screen.getByTestId('cap-platform_live-origin').textContent).toContain('absent');
    expect(screen.getByTestId('cap-e0_enriched-tool_call').textContent).toContain('present');
  });
  it('unsupported family names the missing capability', () => {
    render(<SourcesView />);
    const row = screen.getByTestId('fam-platform_live-tool_failure');
    expect(row.getAttribute('data-status')).toBe('unsupported');
    expect(row.textContent).toContain('tool_call');
    expect(screen.getByTestId('fam-platform_live-human_resolution_verified').textContent).toContain('insufficient_human_evidence');
    expect(screen.getByTestId('fam-e0_enriched-tool_failure').getAttribute('data-status')).toBe('eligible');
  });
  it('exporter counters render for platform_live; sources without exporter say unknown', () => {
    render(<SourcesView />);
    const c = within(screen.getByTestId('counters-platform_live'));
    for (const k of ['events_sent', 'unknown_event_type', 'gap_suspected', 'late_event']) expect(c.getByTestId(`counter-${k}`)).toBeTruthy();
    expect(screen.getByTestId('counters-e0_enriched').textContent).toContain('unknown');
  });
  it('insight cards are read-only: no buttons, links or publish wording, terminal status shown', () => {
    render(<SourcesView />);
    const cards = screen.getAllByTestId(/^insight-/);
    expect(cards.length).toBeGreaterThan(0);
    for (const c of cards) {
      expect(c.getAttribute('data-status')).toMatch(/^(insufficient_|waiting_dependency)/);
      expect(within(c).queryAllByRole('button')).toHaveLength(0);
      expect(within(c).queryAllByRole('link')).toHaveLength(0);
      expect(c.textContent).not.toMatch(/publicar|publish|aprobar/i);
    }
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
  it('has no extra live regions and es-419 keys exist', () => {
    const { container } = render(<SourcesView />);
    expect(container.querySelectorAll('[aria-live],[role=status],[role=alert]')).toHaveLength(0);
    expect(Object.keys(ES_419)).toContain('sources.title');
  });
});
