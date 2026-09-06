import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

/**
 * The keyboard contract a `role="menu"` promises.
 *
 * 🔑 Both menus in this app declared `role="menu"` and `role="menuitem"` and
 * implemented none of it: no arrow keys, no Home/End, and the active-row
 * highlight was wired to `onMouseEnter` alone — so for a keyboard user the
 * menu had no visible selection at all. Declaring the role is a promise to
 * assistive technology about how the widget behaves; this keeps it.
 *
 * Returns the handler to spread onto the menu container. Items are found from
 * the DOM rather than tracked in state, so it works for both the overflow menu
 * and the context menu without either of them owning an index.
 */
export function useMenuKeys(
  ref: React.RefObject<HTMLElement | null>,
  close: () => void,
) {
  return useCallback(
    (e: React.KeyboardEvent) => {
      const root = ref.current;
      if (!root) return;
      const items = Array.from(
        root.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)'),
      );
      if (items.length === 0) return;
      const index = items.indexOf(document.activeElement as HTMLButtonElement);

      const move = (to: number) => {
        e.preventDefault();
        items[(to + items.length) % items.length].focus();
      };

      switch (e.key) {
        case "ArrowDown":
          return move(index + 1);
        case "ArrowUp":
          return move(index < 0 ? items.length - 1 : index - 1);
        case "Home":
          return move(0);
        case "End":
          return move(items.length - 1);
        case "Escape":
          e.preventDefault();
          e.stopPropagation();
          close();
          return;
        case "Tab":
          // A menu is a single stop, not a tab group. Leaving it closes it.
          close();
          return;
      }
    },
    [ref, close],
  );
}

export interface MenuItemSpec {
  label: string;
  icon: IconName;
  onSelect: () => void;
  /** Right-aligned shortcut hint. Build it with `shortcut()` from lib/platform
   *  rather than hardcoding "⌘⇧E" — the modifier differs per platform. */
  shortcut?: string;
  /** Numeric badge (e.g. unfinished-note count). Hidden when 0/undefined. */
  badge?: number;
  /** Section heading to group items under. */
  section?: string;
  /** Renders in the danger colour and sits below a divider. */
  destructive?: boolean;
  disabled?: boolean;
  /** Submenu items */
  children?: MenuItemSpec[];
}

interface MenuProps {
  /** The trigger's accessible name. */
  label: string;
  items: MenuItemSpec[];
  triggerClassName?: string;
  children?: ReactNode;
}

/**
 * Overflow menu. Everything that used to be a permanent toolbar button and
 * isn't part of the main coding loop lives in here — the whole point is that
 * the workspace shows three controls, not nine.
 */
export function Menu({
  label,
  items,
  triggerClassName = "btn btn-ghost btn-icon",
  children,
}: MenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const close = useCallback(() => {
    setOpen(false);
    // Back to the trigger, not to the top of the page — closing a menu should
    // leave you where you opened it.
    triggerRef.current?.focus();
  }, []);
  const onMenuKeys = useMenuKeys(popRef, close);

  // Focus the first row when the menu opens. Doing this for pointer users too
  // is deliberate: it costs a mouse user nothing and means the very next key
  // press works, without having to detect how the menu was opened.
  useEffect(() => {
    if (!open) return;
    const first = popRef.current?.querySelector<HTMLButtonElement>(
      '[role="menuitem"]:not(:disabled)',
    );
    first?.focus();
  }, [open]);

  // Click outside closes. Listen on pointerdown so dragging into a backdrop
  // doesn't fire an accidental click on what was underneath.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) {
        close();
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open, close]);

  const primary = items.filter((i) => !i.destructive);
  const destructive = items.filter((i) => i.destructive);

  return (
    <div ref={rootRef} className="relative inline-block">
      <button
        ref={triggerRef}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={triggerClassName}
      >
        {children ?? <Icon name="dots" size={16} />}
      </button>

      {open && (
        <div
          ref={popRef}
          role="menu"
          aria-label={label}
          onKeyDown={onMenuKeys}
          className="glass-pop anim-rise absolute right-0 top-[calc(100%+6px)] z-50 min-w-56 p-1.5"
        >
          {primary.map((item) => (
            <MenuRow key={item.label} item={item} onDone={close} />
          ))}
          {destructive.length > 0 && (
            <>
              <div className="divider my-1.5" />
              {destructive.map((item) => (
                <MenuRow key={item.label} item={item} onDone={close} />
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}

function highlight(el: HTMLElement, item: MenuItemSpec) {
  if (item.disabled) return;
  el.style.background = item.destructive ? "var(--danger-soft)" : "var(--fill)";
}

export function MenuRow({
  item,
  onDone,
}: {
  item: MenuItemSpec;
  onDone: () => void;
}) {
  const [subOpen, setSubOpen] = useState(false);
  const hasSubmenu = Boolean(item.children && item.children.length > 0);

  return (
    <div
      className="relative w-full"
      onMouseEnter={() => hasSubmenu && !item.disabled && setSubOpen(true)}
      onMouseLeave={() => hasSubmenu && setSubOpen(false)}
    >
      <button
        type="button"
        role="menuitem"
        disabled={item.disabled}
        onClick={() => {
          if (hasSubmenu) {
            setSubOpen((v) => !v);
          } else {
            onDone();
            item.onSelect();
          }
        }}
        className="flex w-full items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-left text-[13px] transition-colors disabled:opacity-40"
        style={{
          color: item.destructive ? "var(--danger)" : "var(--ink)",
        }}
        onMouseEnter={(e) => highlight(e.currentTarget, item)}
        onFocus={(e) => {
          highlight(e.currentTarget, item);
          if (hasSubmenu && !item.disabled) setSubOpen(true);
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
        onBlur={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        <Icon name={item.icon} size={15} />
        <span className="flex-1">{item.label}</span>
        {item.badge != null && item.badge > 0 ? (
          <span
            className="grid min-h-5 min-w-5 place-items-center rounded-full px-1 text-[10.5px] font-semibold"
            style={{ background: "var(--accent-soft)", color: "var(--accent)" }}
            aria-label={`${item.badge} unfinished`}
          >
            {item.badge}
          </span>
        ) : null}
        {item.shortcut && (
          <span
            className="font-mono text-[11px]"
            style={{ color: "var(--ink-4)" }}
          >
            {item.shortcut}
          </span>
        )}
        {hasSubmenu && (
          <span className="text-[11px] text-[var(--ink-4)] ml-1 select-none">
            ▸
          </span>
        )}
      </button>

      {hasSubmenu && subOpen && (
        <div
          role="menu"
          className="glass-pop anim-rise absolute left-[calc(100%-4px)] top-0 z-50 min-w-44 p-1.5 shadow-lg"
        >
          {item.children!.map((subItem) => (
            <MenuRow
              key={subItem.label}
              item={subItem}
              onDone={() => {
                setSubOpen(false);
                onDone();
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}
