/** Optional public-site measurement. Never send editor content, errors, or arbitrary URL queries. */
type Clarity = ((...args: unknown[]) => void) & { q?: IArguments[] };
type AnalyticsWindow = Window & { dataLayer?: unknown[]; gtag?: (...args: unknown[]) => void; clarity?: Clarity; __axiomAnalytics?: boolean };
const clarityProjectId = "yptb4ni40z";
const eventValues: Record<string, Record<string, readonly string[]>> = {
  cta_click: { destination: ["playground", "docs", "app", "github", "blog"], action: ["get_started", "start_building", "playground", "other"] },
  get_started_click: {},
  playground_open_click: {},
  install_command_copy: { method: ["shell", "homebrew"] },
  playground_run: {},
  playground_run_result: { result: ["success", "failure"] },
  inspector_view: { view: ["overview", "graph", "events", "changes"] },
  app_export_complete: {},
};
export function track(name: string, properties: Record<string, string> = {}) {
  window.dispatchEvent(new CustomEvent("axiom:analytics", { detail: { name, properties } }));
}
export function initAnalytics(measurementId: string | undefined, site: "landing" | "docs" | "playground") {
  const w = window as AnalyticsWindow;
  const gaEnabled = !!measurementId && /^G-[A-Z0-9]+$/.test(measurementId);
  const clarityEnabled = site !== "playground";
  if ((!gaEnabled && !clarityEnabled) || w.__axiomAnalytics) return () => {};
  if (!["axiomcore.dev", "docs.axiomcore.dev", "playground.axiomcore.dev"].includes(location.hostname)) return () => {};
  w.__axiomAnalytics = true;
  const consentName = "axiom_analytics_consent";
  const read = () => document.cookie.split("; ").find(c => c.startsWith(`${consentName}=`))?.split("=")[1];
  let accepted = read() === "yes";
  let gaLoaded = false;
  let clarityLoaded = false;
  const save = (value: string) => { document.cookie = `${consentName}=${value}; Path=/; Domain=axiomcore.dev; Max-Age=15552000; SameSite=Lax; Secure`; };
  const pageLocation = () => {
    const url = new URL(location.origin + location.pathname);
    for (const key of ["utm_source", "utm_medium", "utm_campaign"]) {
      const value = new URL(location.href).searchParams.get(key);
      if (value && /^[a-zA-Z0-9._~-]{1,80}$/.test(value)) url.searchParams.set(key, value);
    }
    return url.href;
  };
  const send = (name: string, parameters: Record<string, string> = {}) => {
    if (!accepted) return;
    if (gaLoaded) w.gtag?.("event", name, { ...parameters, site, page_location: pageLocation() });
    if (clarityLoaded) w.clarity?.("event", name === "cta_click" ? `cta_${parameters.action || parameters.destination || "other"}` : name);
  };
  function activateGa() {
    if (!gaEnabled || gaLoaded) return;
    gaLoaded = true;
    w.dataLayer = w.dataLayer || [];
    w.gtag = function () { w.dataLayer!.push(arguments); };
    w.gtag("consent", "default", { analytics_storage: "granted", ad_storage: "denied", ad_user_data: "denied", ad_personalization: "denied" });
    w.gtag("js", new Date());
    w.gtag("config", measurementId, {
      send_page_view: false, allow_google_signals: false, allow_ad_personalization_signals: false,
      cookie_domain: "axiomcore.dev", page_location: pageLocation(),
      page_referrer: document.referrer ? new URL(document.referrer).origin : "",
    });
    const script = document.createElement("script");
    script.async = true;
    script.src = `https://www.googletagmanager.com/gtag/js?id=${measurementId}`;
    document.head.append(script);
    send("page_view");
  }
  function activateClarity() {
    if (!clarityEnabled || clarityLoaded) return;
    clarityLoaded = true;
    w.clarity = w.clarity || function () { (w.clarity!.q = w.clarity!.q || []).push(arguments); };
    w.clarity("consentv2", { ad_Storage: "denied", analytics_Storage: "granted" });
    w.clarity("set", "site", site);
    const script = document.createElement("script");
    script.async = true;
    script.src = `https://www.clarity.ms/tag/${clarityProjectId}`;
    document.head.append(script);
  }
  function activate() { activateGa(); activateClarity(); }
  const banner = document.createElement("section");
  banner.className = "ax-consent";
  banner.setAttribute("aria-label", "Optional analytics");
  banner.innerHTML = `<p>Help improve AxiomCore with optional analytics. We measure visits and selected actions; session replay is limited to the public site and docs, never the Playground editor. <a href="https://docs.axiomcore.dev/reference/documentation-site-privacy/">Privacy details</a></p><div><button type="button" data-choice="no">Decline</button><button type="button" data-choice="yes">Accept analytics</button></div>`;
  const settings = document.createElement("button");
  settings.className = "ax-consent-settings";
  settings.type = "button";
  settings.textContent = "Privacy choices";
  settings.onclick = () => { banner.hidden = false; banner.querySelector("button")?.focus(); };
  banner.hidden = !!read();
  banner.querySelectorAll<HTMLButtonElement>("button").forEach(button => {
    button.onclick = () => {
      accepted = button.dataset.choice === "yes";
      save(accepted ? "yes" : "no");
      banner.hidden = true;
      settings.focus();
      if (accepted) activate();
      else if (gaLoaded || clarityLoaded) {
        w.gtag?.("consent", "update", { analytics_storage: "denied" });
        w.clarity?.("consentv2", { ad_Storage: "denied", analytics_Storage: "denied" });
        for (const cookie of document.cookie.split(";")) {
          const name = cookie.split("=")[0].trim();
          if (!/^(?:_ga(?:_|$)|_clck$|_clsk$|CLID$)/.test(name)) continue;
          for (const domain of ["", "; Domain=axiomcore.dev", `; Domain=${location.hostname}`]) document.cookie = `${name}=; Max-Age=0; Path=/${domain}; Secure; SameSite=Lax`;
        }
        location.reload();
      }
    };
  });
  document.body.append(banner, settings);
  if (accepted) activate();
  function onEvent(event: Event) {
    const detail = (event as CustomEvent).detail;
    if (!detail || !Object.hasOwn(eventValues, detail.name)) return;
    const allowed: Record<string, string> = {};
    for (const [key, values] of Object.entries(eventValues[detail.name])) {
      if (values.includes(detail.properties?.[key])) allowed[key] = detail.properties[key];
    }
    send(detail.name, allowed);
  }
  function onClick(event: MouseEvent) {
    const link = (event.target as Element).closest?.("a[href]") as HTMLAnchorElement | null;
    if (!link) return;
    const destinations: Record<string, string> = { "playground.axiomcore.dev": "playground", "docs.axiomcore.dev": "docs", "app.axiomcore.dev": "app", "github.com": "github" };
    const destination = destinations[link.hostname] || (link.pathname === "/blog/" ? "blog" : "");
    if (destination) {
      const action = link.dataset.analyticsCta || "other";
      send("cta_click", { destination, action });
      if (action === "get_started" && destination === "app") send("get_started_click");
      if (action === "playground" && destination === "playground") send("playground_open_click");
    }
  }
  window.addEventListener("axiom:analytics", onEvent);
  let lastPath = location.pathname;
  const onNavigation = () => {
    if (location.pathname === lastPath) return;
    lastPath = location.pathname;
    send("page_view");
  };
  window.addEventListener("axiom:pageview", onNavigation);
  document.addEventListener("click", onClick);
  return () => { window.removeEventListener("axiom:analytics", onEvent); window.removeEventListener("axiom:pageview", onNavigation); document.removeEventListener("click", onClick); banner.remove(); settings.remove(); w.__axiomAnalytics = false; };
}
