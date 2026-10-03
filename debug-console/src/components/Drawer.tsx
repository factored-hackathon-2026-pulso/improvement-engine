import { useEffect, useRef, type ReactNode } from 'react';
import { t } from '../i18n/es419';

interface Props { title: string; instance: string; onClose: () => void; children: ReactNode }

/** Non-modal drawer: named region, focus moves in on open, Escape closes and restores focus to the trigger. */
export function Drawer({ title, instance, onClose, children }: Props) {
  const ref = useRef<HTMLElement>(null);
  const opener = useRef<Element | null>(document.activeElement);
  useEffect(() => {
    ref.current?.focus();
    const back = opener.current;
    return () => { if (back instanceof HTMLElement) back.focus(); };
  }, []);
  return (
    <aside
      ref={ref} tabIndex={-1} role="dialog" aria-label={title} data-instance={instance} data-testid="drawer"
      onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } }}
      style={{ overflow: 'auto', maxHeight: '40vh', border: '1px solid #888', padding: 12 }}
    >
      <h3>{title}</h3>
      <button type="button" onClick={onClose}>{t('drawer.close')}</button>
      {children}
    </aside>
  );
}
