import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type KeyboardEvent,
  Children,
  isValidElement,
} from "react";
import { createPortal } from "react-dom";
import "./select.css";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

/** Shared single-choice control. Keeps focus on the combobox for keyboard navigation. */
export default function Select({
  value,
  onValueChange,
  options,
  label,
  icon,
  title,
  disabled = false,
  className = "",
}: {
  value: string | number;
  onValueChange: (value: string) => void;
  options: SelectOption[];
  label: string;
  icon?: ReactNode;
  title?: string;
  disabled?: boolean;
  className?: string;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const search = useRef({ text: "", at: 0 });
  const [open, setOpen] = useState(false);
  const selected = options.findIndex((o) => o.value === String(value));
  const [active, setActive] = useState(selected);
  const [position, setPosition] = useState({ left: 0, top: 0, width: 220, maxHeight: 280 });
  const enabled = options.map((o, i) => (o.disabled ? -1 : i)).filter((i) => i >= 0);
  const close = () => setOpen(false);
  const choose = (index: number) => {
    if (!options[index] || options[index].disabled) return;
    onValueChange(options[index].value);
    close();
    trigger.current?.focus();
  };
  const show = () => {
    trigger.current?.focus();
    setActive(selected >= 0 && !options[selected]?.disabled ? selected : (enabled[0] ?? -1));
    setOpen(true);
  };
  useLayoutEffect(() => {
    if (!open || !trigger.current) return;
    const rect = trigger.current.getBoundingClientRect();
    const width = Math.min(Math.max(rect.width, 240), window.innerWidth - 24);
    const below = window.innerHeight - rect.bottom - 16;
    const above = rect.top - 16;
    const height = Math.min(280, Math.max(below, above));
    setPosition({
      left: Math.max(12, Math.min(rect.left, window.innerWidth - width - 12)),
      top:
        below >= Math.min(280, options.length * 40 + 12) || below >= above
          ? rect.bottom + 6
          : Math.max(12, rect.top - Math.min(height, options.length * 40 + 12) - 6),
      width,
      maxHeight: height,
    });
    const outside = (e: PointerEvent) => {
      if (!trigger.current?.contains(e.target as Node) && !menu.current?.contains(e.target as Node))
        close();
    };
    const scroll = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("scroll", scroll, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", close);
    };
  }, [open, options.length]);
  useLayoutEffect(() => {
    if (open)
      menu.current
        ?.querySelector<HTMLElement>(`[data-index="${active}"]`)
        ?.scrollIntoView({ block: "nearest" });
  }, [open, active]);
  const keyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key === "Tab") {
      close();
      return;
    }
    if (e.key === "Escape" && open) {
      e.preventDefault();
      e.stopPropagation();
      close();
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
      e.preventDefault();
      if (!open) {
        show();
        return;
      }
      const next =
        e.key === "Home"
          ? enabled[0]
          : e.key === "End"
            ? enabled.at(-1)
            : enabled[
                (enabled.indexOf(active) + (e.key === "ArrowDown" ? 1 : -1) + enabled.length) %
                  enabled.length
              ];
      setActive(next ?? -1);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (open) choose(active);
      else show();
    } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const now = Date.now();
      search.current = {
        text:
          (now - search.current.at < 700 ? search.current.text : "") + e.key.toLocaleLowerCase(),
        at: now,
      };
      const match = options.findIndex(
        (o) => !o.disabled && o.label.toLocaleLowerCase().startsWith(search.current.text),
      );
      if (match >= 0) {
        if (!open) setOpen(true);
        setActive(match);
      }
    }
  };
  const portal =
    typeof document === "undefined" ? null : (trigger.current?.closest("dialog") ?? document.body);
  return (
    <span className={`pd-select ${className}`}>
      <button
        type="button"
        ref={trigger}
        className="pd-select-trigger"
        role="combobox"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-activedescendant={open && active >= 0 ? `${id}-${active}` : undefined}
        title={title ?? options[selected]?.label}
        disabled={disabled}
        onClick={() => (open ? close() : show())}
        onKeyDown={keyDown}
        onBlur={(e) => {
          if (!menu.current?.contains(e.relatedTarget)) close();
        }}
      >
        {icon && (
          <span className="pd-select-icon" aria-hidden="true">
            {icon}
          </span>
        )}
        <span className="pd-select-value">{options[selected]?.label ?? "请选择"}</span>
        <svg
          className="pd-select-chevron"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
      {open &&
        portal &&
        createPortal(
          <div
            ref={menu}
            id={id}
            role="listbox"
            aria-label={label}
            className="pd-select-menu"
            style={position}
          >
            {options.map((option, index) => (
              <div
                key={option.value}
                id={`${id}-${index}`}
                role="option"
                aria-selected={index === selected}
                aria-disabled={option.disabled || undefined}
                data-index={index}
                className={`pd-select-option${active === index ? " highlighted" : ""}`}
                onPointerDown={(e) => e.preventDefault()}
                onPointerMove={() => {
                  if (!option.disabled) setActive(index);
                }}
                onClick={() => choose(index)}
              >
                <span className="pd-select-check" aria-hidden="true">
                  {index === selected ? "✓" : ""}
                </span>
                <span>{option.label}</span>
              </div>
            ))}
            {!options.length && <div className="pd-select-empty">暂无选项</div>}
          </div>,
          portal,
        )}
    </span>
  );
}

/** Compatibility wrapper for existing forms; all menus use the same controlled component. */
export function SelectField({
  children,
  value,
  onChange,
  "aria-label": label = "选择选项",
  title,
  disabled,
  className,
  id,
  icon,
}: {
  children: ReactNode;
  value: string | number;
  onChange: (event: { target: { value: string } }) => void;
  "aria-label"?: string;
  title?: string;
  disabled?: boolean;
  className?: string;
  id?: string;
  icon?: ReactNode;
}) {
  const options = Children.toArray(children)
    .filter(isValidElement<{ value?: string | number; children?: ReactNode; disabled?: boolean }>)
    .map((child) => {
      const text = Children.toArray(child.props.children).join("");
      return {
        value: String(child.props.value ?? text),
        label: text,
        disabled: child.props.disabled,
      };
    });
  return (
    <span id={id} className="pd-select-field">
      <Select
        value={value}
        onValueChange={(next) => onChange({ target: { value: next } })}
        options={options}
        label={label}
        icon={icon}
        title={title}
        disabled={disabled}
        className={className}
      />
    </span>
  );
}
