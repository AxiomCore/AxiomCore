// Plain TypeScript web client: Axiom sign-in with @axiomcore/auth-web, then a call to the
// assurance-guarded Acore endpoint. Passkeys run on this origin (RP ID localhost).
import { createAxiomAuth, runtimeAuthRefusal, sessionStorageAdapter, type PublicSession } from '@axiomcore/auth-web';

// Absolute: the same script runs on /auth/callback after the hosted page.
const config = await (await fetch('/config.json', { cache: 'no-store' })).json() as { issuer: string; audience: string; api: string };
const auth = createAxiomAuth({
  issuer: config.issuer,
  audience: config.audience,
  redirectUri: location.origin + '/auth/callback',
  storage: sessionStorageAdapter(),
  // The bearer goes only to this page's origin and these: the API is another origin.
  apiOrigins: [new URL(config.api).origin],
});

const $ = (id: string) => document.getElementById(id) as HTMLElement;
const show = (s: PublicSession) => { $('status').textContent = s.status + (s.principal ? ' as ' + s.principal.subject + ' (' + s.acr + ')' : ''); };
auth.subscribe(show);

try {
  const { returnTo } = await auth.handleRedirect();
  if (returnTo) history.replaceState(null, '', returnTo);
} catch (error) {
  $('error').textContent = (error as { code?: string }).code ?? 'unavailable';
}
show(await auth.restore());

$('sign-in').addEventListener('click', () => { void auth.signInHosted({ returnTo: '/' }); });
$('sign-out').addEventListener('click', async () => { await auth.signOut(); $('me').textContent = ''; });
$('me-button').addEventListener('click', async () => {
  // authorizedFetch refreshes once and retries once on 401 authorization_changed.
  const response = await auth.authorizedFetch(config.api + '/admin');
  if (response.status === 401) {
    const refusal = runtimeAuthRefusal(await response.clone().text());
    if (refusal?.code === 'step_up_required') { $('me').textContent = 'step-up required: ' + (refusal.acr ?? ''); return; }
  }
  $('me').textContent = response.ok ? JSON.stringify(await response.json()) : 'HTTP ' + response.status;
});
$('passkey').addEventListener('click', async () => { show(await auth.signInWithPasskey()); });
$('register').addEventListener('click', async () => { await auth.registerPasskey('This device'); });
