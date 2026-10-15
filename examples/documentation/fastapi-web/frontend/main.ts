import { atmx } from 'atmx-web';
import { AxiomDefaultConfig } from './generated/sdk';

const output = document.querySelector<HTMLOutputElement>('#result')!;
const button = document.querySelector<HTMLButtonElement>('#submit')!;
const input = document.querySelector<HTMLInputElement>('#name')!;
const config = {
  ...AxiomDefaultConfig,
  contracts: {
    greetings: { ...AxiomDefaultConfig.contracts.greetings, baseUrl: 'http://127.0.0.1:8083' },
  },
  wasmUrl: '/axiom_runtime.wasm',
};
const ready = await atmx.init(config);
let query: ReturnType<typeof atmx.query> | undefined;
let unsubscribe: (() => void) | undefined;
if (!ready.ok) {
  output.textContent = ready.error?.message ?? 'Runtime initialization failed';
} else {
  output.textContent = 'Ready. Ask your Python backend for a greeting.';
  button.disabled = false;
}
document.querySelector<HTMLFormElement>('#greeting')!.addEventListener('submit', (event) => {
  event.preventDefault();
  unsubscribe?.();
  query?.dispose();
  query = atmx.query('greetings.greet', { name: input.value.trim() });
  unsubscribe = query.subscribe((state) => {
    if (state.error) output.textContent = state.error.message ?? 'Request failed';
    else if (state.data) output.textContent = (state.data as {message: string}).message;
    else output.textContent = 'Loading...';
  });
});
window.addEventListener('pagehide', () => { unsubscribe?.(); query?.dispose(); });
