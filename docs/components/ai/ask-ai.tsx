'use client';
// Ask AI: the Fumadocs AI chat (components/ai/chat) wired to /api/chat. This
// Fumadocs version's DocsLayout has no AI chat slot, so the panel is a fixed
// side sheet: 420px wide on desktop and full screen on small screens.
import { useChat } from '@ai-sdk/react';
import { DefaultChatTransport } from 'ai';
import Link from 'fumadocs-core/link';
import { FileTextIcon, LoaderCircleIcon } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { AskAiUIMessage } from '@/lib/ask-ai/chat';
import { TURNSTILE_HEADER } from '@/lib/ask-ai/guard';
import { AIChatPanel, AIChatProvider, AIChatSearch, AIChatTrigger, useAIChat } from './chat';
import { humanCheck } from './human-check';

const suggestions = [
  'How do I run my first app?',
  'What is Acore?',
  'How do I connect an existing FastAPI backend?',
  'Which client SDKs are available?',
];

// Edge rules on the zone answer with Cloudflare's own pages, not the JSON errors of /api/chat.
const errorFallbacks: Record<number, string> = {
  403: 'Ask AI could not confirm this request came from a person. Reload the page and try again.',
  429: 'Too many questions in a short time. Wait a minute and try again.',
};

/** The chat shows a failed response body as its error, so make that body one readable sentence. */
async function readableErrors(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const response = await fetch(input, init);
  if (response.ok) return response;
  let message = errorFallbacks[response.status] ?? 'Ask AI could not answer right now. Try again in a moment.';
  try {
    const body: unknown = await response.json();
    if (body && typeof body === 'object' && typeof (body as { error?: unknown }).error === 'string') {
      message = (body as { error: string }).error;
    }
  } catch {
    // Not JSON: keep the fallback.
  }
  return new Response(message, { status: response.status, statusText: response.statusText });
}

const transport = new DefaultChatTransport<AskAiUIMessage>({
  api: '/api/chat',
  headers: async (): Promise<Record<string, string>> => ({ [TURNSTILE_HEADER]: await humanCheck.take() }),
  fetch: readableErrors,
});

export function AskAI() {
  const chat = useChat<AskAiUIMessage>({ id: 'ask-ai', throttle: 40, transport });

  return (
    <AIChatProvider
      chat={chat}
      renderPart={renderPart}
      suggestions={suggestions}
      description={
        <>
          Answers come from the AxiomCore docs and can contain mistakes, so check the linked pages. Questions go to an
          AI model through OpenRouter; leave out secrets.{' '}
          <Link href="/reference/documentation-site-privacy#ask-ai" className="underline underline-offset-2">
            Privacy
          </Link>
        </>
      }
    >
      {/* Tailwind 4.1 has no inset-e utility, which the trigger's default position uses. */}
      <AIChatTrigger className="end-4" />
      <AskAISheet />
    </AIChatProvider>
  );
}

function AskAISheet() {
  const { open } = useAIChat();
  if (!open) return null;
  return (
    <aside
      role="dialog"
      aria-label="Ask AI about AxiomCore"
      className="fixed inset-y-0 end-0 z-50 flex w-full flex-col border-s bg-fd-background shadow-2xl sm:w-[420px]"
    >
      <div className="min-h-0 flex-1">
        <AIChatPanel />
      </div>
      <HumanCheckSlot />
    </aside>
  );
}

/** Where the Turnstile checkbox appears, only when Cloudflare asks for an interaction. */
function HumanCheckSlot() {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    void humanCheck.mount(ref.current!).catch(() => {
      // The question then fails with a message and Try again.
    });
    return () => humanCheck.unmount();
  }, []);
  return <div ref={ref} className="flex justify-center" />;
}

type Part = AskAiUIMessage['parts'][number];

function renderPart(part: Part, live: boolean) {
  if (part.type === 'data-sources') {
    // The server searches the docs before the model answers; show what it found.
    return (
      <AIChatSearch
        live={live}
        part={{
          state: 'output-available',
          input: { query: part.data.query },
          output: part.data.results.map((result) => ({
            doc: {
              url: result.url,
              title: result.heading && result.heading !== result.title ? `${result.title} › ${result.heading}` : result.title,
            },
          })),
        }}
      />
    );
  }
  if (part.type === 'tool-read_page') return <ReadPage part={part} live={live} />;
  return null;
}

function ReadPage({ part, live }: { part: Extract<Part, { type: 'tool-read_page' }>; live: boolean }) {
  const running = live && !part.state.startsWith('output-');
  const output = part.state === 'output-available' ? part.output : undefined;
  return (
    <div className="flex items-center gap-2 text-sm text-fd-muted-foreground motion-safe:animate-fd-fade-in">
      <span className="flex w-4 shrink-0 justify-center [&_svg]:size-3.5">
        {running ? <LoaderCircleIcon className="animate-spin" /> : <FileTextIcon />}
      </span>
      {output?.found ? (
        <span className="truncate">
          Read{' '}
          <Link href={output.url} className="underline-offset-2 hover:text-fd-accent-foreground hover:underline">
            {output.title}
          </Link>
        </span>
      ) : (
        <span className="truncate">{running ? 'Reading a page' : output ? 'Page not found' : 'Page read stopped'}</span>
      )}
    </div>
  );
}
