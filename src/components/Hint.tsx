import { useId, type ReactNode } from "react";

/** One discreet, keyboard-accessible explanation at the point of use. */
export default function Hint({
  children,
  label = "说明",
}: {
  children: ReactNode;
  label?: string;
}) {
  const id = useId();
  return (
    <span className="hint">
      <button type="button" className="hint-trigger" aria-label={label} aria-describedby={id}>
        i
      </button>
      <span id={id} role="tooltip" className="hint-content">
        {children}
      </span>
    </span>
  );
}
