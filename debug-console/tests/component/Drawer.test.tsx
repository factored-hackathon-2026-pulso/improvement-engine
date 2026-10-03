import { describe, it, expect } from 'vitest';
import { useState } from 'react';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { Drawer } from '../../src/components/Drawer';

function Host() {
  const [open, setOpen] = useState(false);
  const [tick, setTick] = useState(0);
  return (
    <div>
      <button onClick={() => setOpen(true)}>open</button>
      <button onClick={() => setTick(tick + 1)}>rerender</button>
      {open && <Drawer title="Nodo" instance="verify" onClose={() => setOpen(false)}><p>tick {tick}</p></Drawer>}
    </div>
  );
}

describe('Drawer', () => {
  it('moves focus in, survives re-render without remount, Escape restores focus', () => {
    render(<Host />);
    const trigger = screen.getByText('open');
    trigger.focus();
    fireEvent.click(trigger);
    const dlg = screen.getByRole('dialog', { name: 'Nodo' });
    expect(document.activeElement).toBe(dlg);
    fireEvent.click(screen.getByText('rerender'));
    expect(screen.getByRole('dialog', { name: 'Nodo' })).toBe(dlg); // same DOM node: no remount
    fireEvent.keyDown(dlg, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    cleanup();
  });
});
