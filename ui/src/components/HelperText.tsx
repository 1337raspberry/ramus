import type { ReactNode } from "react";

interface HelperTextProps {
  children: ReactNode;
}

/// Small muted descriptive text, typically rendered directly under a
/// settings row to explain a non-obvious option. Distinct from
/// `.bookmark-hint` so future bookmark restyling doesn't ripple here.
export function HelperText({ children }: HelperTextProps) {
  return <p className="settings-helper">{children}</p>;
}
