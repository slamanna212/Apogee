import type { Virtualizer } from '@tanstack/react-virtual';

/** Reset stale sizes and refill them for rows that will not remount or resize. */
export function remeasureVirtualRows(virtualizer: Virtualizer<HTMLDivElement, HTMLDivElement>) {
  const rows = [...virtualizer.elementsCache.values()]
    .filter((row) => row.isConnected)
    .map((row) => ({ index: virtualizer.indexFromElement(row), height: row.offsetHeight }))
    .filter((row) => row.height > 0);
  virtualizer.measure();
  // Rebuild offsets before measuring: resizeItem needs current row entries.
  virtualizer.getTotalSize();
  // Explicit reads also work during scrolling, when measureElement can defer.
  for (const row of rows) virtualizer.resizeItem(row.index, row.height);
}
