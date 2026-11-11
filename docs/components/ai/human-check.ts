'use client';
// Cloudflare Turnstile for Ask AI. The script loads only when the chat opens.
// The widget is invisible for most visitors and shows a checkbox only when
// Cloudflare wants an interaction. Each question sends a fresh single-use
// token; the next one is fetched in the background.
import { TURNSTILE_ACTION } from '@/lib/ask-ai/guard';

/** Public site key of the "AxiomCore docs Ask AI" widget (docs.axiomcore.dev and axiomcore-docs.pages.dev). */
const SITE_KEY = '0x4AAAAAAFezKUJSBNA7oJgd';
/** Cloudflare's always-passing test key, for localhost, which the widget does not allow. */
const TEST_SITE_KEY = '1x00000000000000000000AA';
const SCRIPT_URL = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit&onload=';
const LOADED_CALLBACK = '__askAiTurnstileLoaded';
/** Tokens expire after 300 seconds; a prefetched one is used only while well inside that. */
const TOKEN_MAX_AGE = 240_000;
/** How long a question waits for a token, including time to select the checkbox. */
const TOKEN_WAIT = 90_000;

interface Turnstile {
  render(container: HTMLElement, options: Record<string, unknown>): string | undefined;
  execute(container: HTMLElement): void;
  reset(widgetId: string): void;
  remove(widgetId: string): void;
}

declare global {
  interface Window {
    turnstile?: Turnstile;
  }
}

let script: Promise<Turnstile> | null = null;

function loadTurnstile(): Promise<Turnstile> {
  if (window.turnstile) return Promise.resolve(window.turnstile);
  script ??= new Promise<Turnstile>((resolve, reject) => {
    (window as unknown as Record<string, () => void>)[LOADED_CALLBACK] = () => resolve(window.turnstile!);
    const element = document.createElement('script');
    element.src = SCRIPT_URL + LOADED_CALLBACK;
    element.async = true;
    element.onerror = () => {
      script = null;
      element.remove();
      reject(new Error('Turnstile did not load'));
    };
    document.head.append(element);
  });
  return script;
}

class HumanCheck {
  private container?: HTMLElement;
  private widgetId?: string;
  private running = false;
  private ready?: { token: string; at: number };
  private waiting: ((token: string) => void)[] = [];

  /** Renders the widget into the chat panel and fetches a first token. */
  async mount(container: HTMLElement) {
    this.container = container;
    const turnstile = await loadTurnstile();
    if (this.container !== container || this.widgetId) return;
    this.widgetId = turnstile.render(container, {
      sitekey: /^(localhost|127\.0\.0\.1)$/.test(location.hostname) ? TEST_SITE_KEY : SITE_KEY,
      action: TURNSTILE_ACTION,
      execution: 'execute',
      appearance: 'interaction-only',
      'refresh-expired': 'manual',
      retry: 'auto',
      'retry-interval': 2000,
      callback: (token: string) => this.settle(token),
      'expired-callback': () => {
        this.ready = undefined;
      },
      // 110xxx errors (configuration, unsupported browser) cannot recover, so the
      // waiting question fails and shows Try again. After other errors Turnstile
      // retries or shows its checkbox, and the question keeps waiting for it.
      'error-callback': (code: string | number) => {
        if (String(code).startsWith('110')) this.settle('');
        return true;
      },
      'timeout-callback': () => this.settle(''),
    });
    this.run();
  }

  unmount() {
    if (this.widgetId) window.turnstile?.remove(this.widgetId);
    this.container = this.widgetId = this.ready = undefined;
    this.running = false;
    for (const resolve of this.waiting.splice(0)) resolve('');
  }

  /** A token for one question, or '' when the check failed or the chat is closed. */
  take(): Promise<string> {
    const ready = this.ready && Date.now() - this.ready.at < TOKEN_MAX_AGE ? this.ready.token : undefined;
    this.ready = undefined;
    if (ready) {
      this.run();
      return Promise.resolve(ready);
    }
    if (!this.widgetId) return Promise.resolve('');
    return new Promise((resolve) => {
      this.waiting.push(resolve);
      setTimeout(() => {
        const index = this.waiting.indexOf(resolve);
        if (index < 0) return;
        this.waiting.splice(index, 1);
        // The check never finished; the next question starts a fresh one.
        this.running = false;
        resolve('');
      }, TOKEN_WAIT);
      this.run();
    });
  }

  private run() {
    if (!this.widgetId || !this.container || this.running) return;
    this.running = true;
    window.turnstile!.reset(this.widgetId);
    window.turnstile!.execute(this.container);
  }

  private settle(token: string) {
    this.running = false;
    const next = this.waiting.shift();
    if (!next) {
      if (token) this.ready = { token, at: Date.now() };
      return;
    }
    next(token);
    // Fetch the token for the next question now.
    if (token) this.run();
  }
}

export const humanCheck = new HumanCheck();
