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
