import { useEffect, useRef, useState } from "react";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "bloomin8-theme";

function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

function resolve(mode: ThemeMode): "light" | "dark" {
  return mode === "system" ? (systemPrefersDark() ? "dark" : "light") : mode;
}

let transitionTimer: ReturnType<typeof setTimeout> | undefined;

/** Apply the resolved theme to <html> so token utilities switch. When
 * `animate` is set (a user/OS-driven change, not the initial paint), briefly
 * add `.theme-transition` so every element cross-fades its colours together
 * instead of some snapping and some transitioning (which reads as flicker). */
function apply(mode: ThemeMode, animate = false) {
  const root = document.documentElement;
  if (animate) {
    root.classList.add("theme-transition");
    if (transitionTimer) clearTimeout(transitionTimer);
    transitionTimer = setTimeout(() => root.classList.remove("theme-transition"), 160);
  }
  root.classList.toggle("dark", resolve(mode) === "dark");
}

function readStored(): ThemeMode {
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

/**
 * Theme hook: light / dark / system, persisted in localStorage and applied to
 * <html>. When in "system" mode, live-updates if the OS theme changes.
 */
export function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(() => {
    const m = readStored();
    apply(m);
    return m;
  });

  // Skip animating the first effect run — the theme is already applied
  // synchronously in the initializer, so animating here would be a no-op flash.
  const firstRun = useRef(true);
  useEffect(() => {
    apply(mode, !firstRun.current);
    firstRun.current = false;
    localStorage.setItem(STORAGE_KEY, mode);
    if (mode !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => apply("system", true);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [mode]);

  const resolved = resolve(mode);
  return { mode, setMode, resolved };
}
