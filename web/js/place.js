/**
 * Put a floating panel next to the thing that opened it, inside the window:
 * under it, lined up with its left edge (or `align: 'right'` edge). When it
 * doesn't fit under it, it goes over it (instead of sliding up on top of the
 * very thing you clicked); if it fits neither way, on the roomier side.
 */
export function placeNear(el, anchor, { align = 'left' } = {}) {
  const r = anchor.getBoundingClientRect();
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const x = align === 'right' ? r.right - w : r.left;
  const below = r.bottom + 6;
  const above = r.top - 6 - h;
  const fitsBelow = below + h <= innerHeight - 8;
  const fitsAbove = above >= 8;
  const y = fitsBelow || (!fitsAbove && innerHeight - r.bottom >= r.top) ? below : above;
  el.style.left = `${Math.max(8, Math.min(x, innerWidth - w - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(y, innerHeight - h - 8))}px`;
}
