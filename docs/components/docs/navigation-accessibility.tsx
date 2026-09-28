"use client";

import { useEffect } from "react";
import { useSidebar } from "fumadocs-ui/components/sidebar/base";

/** Supply modal keyboard behavior for Fumadocs' mobile aside. */
export function NavigationAccessibility() {
  const { open, mode, setOpen } = useSidebar();

  useEffect(() => {
    const trigger = document.querySelector<HTMLButtonElement>(
      '#nd-subnav button[aria-label="Open Sidebar"]',
    );
    trigger?.setAttribute("aria-expanded", String(open));
    trigger?.setAttribute("aria-controls", "nd-sidebar-mobile");
    if (!open || mode !== "drawer") return;

    const drawer = document.getElementById("nd-sidebar-mobile");
    if (!drawer) return;
    drawer.setAttribute("role", "dialog");
    drawer.setAttribute("aria-modal", "true");
    drawer.setAttribute("aria-label", "Documentation navigation");
    const close = drawer.querySelector<HTMLButtonElement>(
      'button[aria-label="Open Sidebar"]',
    );
    close?.setAttribute("aria-label", "Close navigation");
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    close?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        drawer.querySelectorAll<HTMLElement>(
          'a[href], button:not(:disabled), input, [tabindex="0"]',
        ),
      ).filter((element) => element.getClientRects().length > 0);
      const first = focusable[0];
      const last = focusable.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.body.style.overflow = previousOverflow;
      drawer.removeAttribute("role");
      drawer.removeAttribute("aria-modal");
      trigger?.focus({ preventScroll: true });
    };
  }, [open, mode, setOpen]);

  return null;
}
