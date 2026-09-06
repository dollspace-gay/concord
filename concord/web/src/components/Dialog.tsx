import { useEffect, useRef, type ReactNode } from 'react';

const FOCUSABLE = [
  'button:not([disabled])',
  'a[href]',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');
const dialogStack: HTMLDivElement[] = [];

function focusableElements(panel: HTMLDivElement | null) {
  return [...(panel?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])].filter((element) => {
    if (element.closest('[inert], [aria-hidden="true"], [hidden]')) return false;
    const style = getComputedStyle(element);
    return element.getClientRects().length > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  });
}

interface DialogProps {
  children: ReactNode;
  label: string;
  onClose: () => void;
  panelClassName?: string;
  backdropClassName?: string;
}

export function Dialog({
  children,
  label,
  onClose,
  panelClassName = '',
  backdropClassName = '',
}: DialogProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = panelRef.current;
    if (!panel) return;
    dialogStack.push(panel);
    const preferred = panel.querySelector<HTMLElement>('[autofocus], [data-dialog-initial-focus]');
    const initial = preferred && focusableElements(panel).includes(preferred)
      ? preferred
      : focusableElements(panel)[0] ?? panel;
    initial?.focus();
    const containFocus = (event: FocusEvent) => {
      if (dialogStack.at(-1) !== panel || panel.contains(event.target as Node)) return;
      (focusableElements(panel)[0] ?? panel).focus();
    };
    document.addEventListener('focusin', containFocus);
    return () => {
      document.removeEventListener('focusin', containFocus);
      const index = dialogStack.lastIndexOf(panel);
      if (index >= 0) dialogStack.splice(index, 1);
      restoreFocus?.focus();
    };
  }, []);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = focusableElements(panelRef.current);
    if (focusable.length === 0) {
      event.preventDefault();
      panelRef.current?.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (!panelRef.current?.contains(document.activeElement)) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    } else if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className={`fixed inset-0 z-50 flex items-center justify-center bg-black/60 ${backdropClassName}`}
      onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        className={panelClassName}
        onKeyDown={handleKeyDown}
      >
        {children}
      </div>
    </div>
  );
}
