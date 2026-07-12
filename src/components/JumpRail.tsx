import { useEffect, useState, type RefObject } from 'react';

export interface JumpGroup {
  label: string;
  index: number;
}

interface JumpRailProps {
  groups: JumpGroup[];
  totalCount: number;
  containerRef: RefObject<HTMLDivElement | null>;
}

/**
 * Plex-style sticky index. Jumping measures the real DOM position of the
 * target item's row (via the item's own element, since grid rows share a
 * top edge) rather than estimating proportionally - a proportional guess
 * can land mid-row in the multi-column grid view and clip the row above it.
 * The active-label highlight on scroll is still a proportional estimate,
 * which is fine since it only needs to be approximately right.
 */
export function JumpRail({ groups, totalCount, containerRef }: JumpRailProps) {
  const [active, setActive] = useState(groups[0]?.label);

  useEffect(() => {
    const el = containerRef.current;
    if (!el || groups.length === 0) return;

    function handleScroll() {
      if (!el) return;
      const scrollable = el.scrollHeight - el.clientHeight;
      const ratio = scrollable > 0 ? el.scrollTop / scrollable : 0;
      const approxIndex = Math.round(ratio * totalCount);
      let current = groups[0];
      for (const g of groups) {
        if (g.index <= approxIndex) current = g;
      }
      setActive(current.label);
    }
    el.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll();
    return () => el.removeEventListener('scroll', handleScroll);
  }, [groups, totalCount, containerRef]);

  function jumpTo(group: JumpGroup) {
    const el = containerRef.current;
    if (!el) return;
    const itemEl = el.firstElementChild?.children[group.index] as HTMLElement | undefined;
    if (itemEl) {
      el.scrollTop += itemEl.getBoundingClientRect().top - el.getBoundingClientRect().top;
      return;
    }
    const ratio = totalCount > 0 ? group.index / totalCount : 0;
    el.scrollTop = ratio * (el.scrollHeight - el.clientHeight);
  }

  if (groups.length === 0) return null;

  return (
    <div
      style={{
        flex: 'none',
        width: 26,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 2,
        overflow: 'hidden',
      }}
    >
      {groups.map((g) => (
        <div
          key={g.label}
          onClick={() => jumpTo(g)}
          role="button"
          style={{
            font: '700 10px "Space Grotesk", sans-serif',
            color: active === g.label ? 'var(--app-accent2)' : 'var(--app-dim2)',
            cursor: 'pointer',
          }}
        >
          {g.label}
        </div>
      ))}
    </div>
  );
}
