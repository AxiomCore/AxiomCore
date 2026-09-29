"use client";
import { useEffect } from "react";
import { usePathname } from "next/navigation";
import { initAnalytics } from "@/lib/brand/analytics/client";
export function SiteAnalytics() {
  const pathname = usePathname();
  useEffect(() => initAnalytics(process.env.NEXT_PUBLIC_GA4_MEASUREMENT_ID, "docs"), []);
  useEffect(() => { window.dispatchEvent(new Event("axiom:pageview")); }, [pathname]);
  return null;
}
