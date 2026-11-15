"""Tests for source extraction, including dynamic actions and drift failures."""
import json
from pathlib import Path
import tempfile
import unittest
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from parity_dashboard import arguments, scan_dashboard


class DashboardInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for folder in ['static/js', 'templates/pages', 'public']:
            (self.root / folder).mkdir(parents=True)
        self.put('public/axiom.axiom', json.dumps({'endpoints': [
            {'name': 'ListThings', 'method': 'GET', 'path': '/api/v1/things'},
            {'name': 'DeleteThing', 'method': 'DELETE', 'path': '/api/v1/things/{thing}'},
        ]}))
        self.js('api-client.js', '')
        for name in ['auth-admin-client.js','data-editor-client.js']:
            self.js(name, "const ROUTES = ([['GET', 'overview', 'read']]);")

    def put(self, path, text):
        (self.root / path).write_text(text)

    def js(self, name, text):
        self.put('static/js/'+name, text)

    def endpoints(self):
        return {(r['method'],r['path']) for r in scan_dashboard(self.root)}

    def test_archive_transport(self):
        self.js('project-archive.js', """
base() { return `/projects/${encodeURIComponent(this.projectSlug)}`; }
call(`${this.base()}/resources`);
call(`${this.base()}/archive`);
this.send('archive', {confirm: phrase}, key);
this.send('restore', {}, key);
call(`${this.base()}/${action}`, {method: 'POST', body: JSON.stringify(body)}, action);
call('/projects?archived=true', {}, 'list');
fetch(`${api.baseUrl}/api/v1${path}`, {credentials: 'include', ...init});
""")
        self.assertTrue({
            ('GET', '/api/v1/projects/{project}/archive'),
            ('GET', '/api/v1/projects/{project}/resources'),
            ('POST', '/api/v1/projects/{project}/archive'),
            ('POST', '/api/v1/projects/{project}/restore'),
            ('GET', '/api/v1/projects'),
        } <= self.endpoints())

    def test_native_session_transport(self):
        self.js('api-client.js', """
const SESSION_PATH = "/api/v1/auth/session";
return nativeFetch(`${apiBase()}${SESSION_PATH}`, {credentials: "include"});
window.fetch = function fetch(input, init) { return nativeFetch(input, init); };
""")
        rows = scan_dashboard(self.root)
        session = next(r for r in rows if r['path'] == '/api/v1/auth/session')
        self.assertEqual(session['method'], 'GET')
        self.assertEqual(session['action'], 'GetSession')
        self.assertFalse(session.get('capability_only'))

    def test_new_raw_fetch_in_api_client_fails(self):
        self.js('api-client.js', "fetch('/api/v1/unreviewed');")
        with self.assertRaisesRegex(ValueError, 'unreviewed raw fetch transport'):
            self.endpoints()

    def test_unreviewed_native_fetch_fails(self):
        for source in ('nativeFetch("/api/v1/new-route");',
                       'nativeFetch(input, {method: "POST"});'):
            with self.subTest(source=source):
                self.js('api-client.js', source)
                with self.assertRaisesRegex(ValueError, 'unreviewed native fetch'):
                    self.endpoints()

    def test_native_session_path_and_method_drift_fail(self):
        for path, options in [('/api/v1/new-session', '{}'),
                              ('/api/v1/auth/session', '{method: "DELETE"}'),
                              ('/api/v1/auth/session', 'options')]:
            self.js('api-client.js', 'const SESSION_PATH = "' + path + '";' +
                    'nativeFetch(`${apiBase()}${SESSION_PATH}`, ' + options + ');')
            with self.assertRaisesRegex(ValueError, 'unreviewed native session'):
                self.endpoints()

    def test_sdk_send_recovery_is_an_action(self):
        self.js('api-client.js', 'send("listThings");')
        row = next(r for r in scan_dashboard(self.root) if r['path'] == '/api/v1/things')
        self.assertFalse(row.get('capability_only'))
        self.js('api-client.js', 'send("unknownOperation");')
        with self.assertRaisesRegex(ValueError, 'unknown SDK operation'):
            self.endpoints()

    def test_archive_new_selector_is_visible(self):
        self.js('project-archive.js', """
this.send('new_action', {}, key);
call(`${this.base()}/${action}`, {method: 'POST'});
""")
        self.assertIn(('POST', '/api/v1/projects/{project}/new_action'), self.endpoints())

    def test_balanced_calls(self):
        self.assertEqual(arguments("`/things/${encode(x)}`, {method: 'POST', body: JSON.stringify({x: [1,2]})})", 0),
                         ['`/things/${encode(x)}`', "{method: 'POST', body: JSON.stringify({x: [1,2]})}"])

    def test_sdk_method_and_template(self):
        self.js('api-client.js', 'queryOnce("listThings");')
        self.put('templates/pages/page.tmpl', ':ax-mutate="sdk.axiom.deleteThing()"')
        self.assertIn(('GET','/api/v1/things'), self.endpoints())
        self.assertIn(('DELETE','/api/v1/things/{thing}'), self.endpoints())

    def test_unused_sdk_operation_excluded(self):
        self.assertNotIn(('GET','/api/v1/things'), self.endpoints())

    def test_new_raw_transport_fails(self):
        self.js('new-page.js', "fetch('/api/v1/new-action', {method: 'POST'});")
        with self.assertRaisesRegex(ValueError, 'unreviewed raw fetch transport'):
            self.endpoints()

    def test_download_url_interpolation(self):
        self.js('api-client.js', 'return `${apiBase()}/api/v1/projects/${encodeURIComponent(slug)}/contracts/${encodeURIComponent(version)}/pull`;')
        rows = scan_dashboard(self.root)
        download = next(r for r in rows if r['action'] == 'download contract release')
        self.assertEqual(download['path'], '/api/v1/projects/{id}/contracts/{id}/pull')

    def test_unknown_sdk_fails(self):
        self.js('api-client.js', 'mutate("newOperation", {});')
        with self.assertRaisesRegex(ValueError, 'unknown SDK operation'):
            self.endpoints()

    def test_typed_routes_methods(self):
        self.js('auth-admin-client.js', "const ROUTES = ([['DELETE', 'users/{id}', 'write']]);")
        self.assertIn(('DELETE','/api/v1/projects/{project}/auth/environments/{environment}/users/{id}'), self.endpoints())

    def test_inherited_platform_request_prefix(self):
        self.js('project-auth-advanced.js', "this.platformRequest('/auth/policy/releases')")
        self.assertIn(('GET','/api/v1/projects/{project}/auth/policy/releases'), self.endpoints())

    def test_new_dynamic_resource_action_detected(self):
        self.js('project-resources.js', """actions(resource){return {ready:['suspend','new_action']}[resource.status];}
this.request(`${this.base()}/${encode(resource.id)}/${action}`, {method:'POST'});
""")
        self.assertIn(('POST','/api/v1/projects/{project}/resources/{id}/new_action'), self.endpoints())

    def test_query_removed_and_post_preserved(self):
        self.js('project-branches.js', "this.send('/branches/settings?environment=' + encode(this.parentSlug), 'PUT', {});")
        self.assertIn(('PUT','/api/v1/projects/{project}/branches/settings'), self.endpoints())

    def test_legacy_unresolved_visible(self):
        self.put('templates/pages/legacy.tmpl', 'sdk.axiom.createOrg()')
        self.assertIn(('UNRESOLVED','SDK:createOrg'), self.endpoints())


if __name__ == '__main__':
    unittest.main()
