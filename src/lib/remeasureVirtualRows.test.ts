import { describe, expect, it } from 'vitest';
import { Virtualizer } from '@tanstack/react-virtual';
import { remeasureVirtualRows } from './remeasureVirtualRows';

function createList() {
  const virtualizer = new Virtualizer<HTMLDivElement, HTMLDivElement>({
    count: 3,
    initialRect: { width: 800, height: 1000 },
    getScrollElement: () => null,
    estimateSize: () => 122,
    gap: 10,
    scrollToFn: () => {},
    observeElementRect: () => {},
    observeElementOffset: () => {},
  });
  virtualizer.getTotalSize();
  return virtualizer;
}

function row(index: number, height: number, isConnected = true) {
  return {
    isConnected,
    offsetHeight: height,
    getAttribute: () => String(index),
  } as unknown as HTMLDivElement;
}

describe('remeasureVirtualRows', () => {
  it('restores actual spacing after a cache reset without remounting rows', () => {
    const list = createList();
    list.elementsCache.set(0, row(0, 99));
    list.elementsCache.set(1, row(1, 114));
    list.elementsCache.set(2, row(2, 99));
    list.resizeItem(0, 300);
    list.resizeItem(1, 300);

    remeasureVirtualRows(list);

    expect(list.getTotalSize()).toBe(332);
    expect(list.getVirtualItems().map(({ start }) => start)).toEqual([0, 109, 233]);
    remeasureVirtualRows(list);
    expect(list.getTotalSize()).toBe(332);
  });

  it('refreshes mounted rows even while scrolling and drops stale offscreen sizes', () => {
    const list = createList();
    list.isScrolling = true;
    list.elementsCache.set(0, row(0, 99));
    list.resizeItem(2, 400);

    remeasureVirtualRows(list);

    list.getTotalSize();
    expect(list.getVirtualItems().map(({ size }) => size)).toEqual([99, 122, 122]);
  });

  it('does not store measurements from detached or hidden rows', () => {
    const list = createList();
    list.elementsCache.set(0, row(0, 300, false));
    list.elementsCache.set(1, row(1, 0));

    remeasureVirtualRows(list);

    list.getTotalSize();
    expect(list.getVirtualItems().map(({ size }) => size)).toEqual([122, 122, 122]);
  });
});
