import { menuPlacement } from './context-menu-core.js';

export function contextMenu(onChange = () => {}) {
  let element = null;
  let anchor = null;
  const position = () => {
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const at = menuPlacement(anchor, { w: rect.width, h: rect.height }, { w: innerWidth, h: innerHeight });
    element.style.left = `${at.x}px`;
    element.style.top = `${at.y}px`;
    onChange();
  };
  const close = () => {
    if (!element) return;
    element.remove();
    element = null;
    onChange();
  };
  document.addEventListener('pointerdown', (event) => {
    if (element && !element.contains(event.target)) close();
  });
  document.addEventListener('keydown', (event) => {
    if (element && event.key === 'Escape') { event.preventDefault(); close(); }
  });
  addEventListener('resize', position);
  return {
    close,
    rect: () => element?.getBoundingClientRect() ?? null,
    open(point, choices) {
      close();
      anchor = point;
      element = document.createElement('div');
      element.className = 'context-menu';
      element.setAttribute('role', 'menu');
      element.setAttribute('aria-label', 'Snapshot actions');
      element.addEventListener('contextmenu', (event) => event.preventDefault());
      for (const { label, action, disabled = false } of choices) {
        const button = document.createElement('button');
        button.type = 'button';
        button.textContent = label;
        button.disabled = disabled;
        button.setAttribute('role', 'menuitem');
        button.onclick = () => { close(); action?.(); };
        element.append(button);
      }
      document.body.append(element);
      position();
    },
  };
}
