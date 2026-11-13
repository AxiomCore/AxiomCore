"""Regression cases for R3-VERIFY, including real CLI anchor removal."""
import shutil
import tempfile
from pathlib import Path
from test_parity import checker, DashboardExtractionTests
import parity_cli
from parity_dashboard import group_for

CORE = Path(__file__).resolve().parents[2]
P = '/api/v1/projects/{project}'

class VerifierDashboardTests(DashboardExtractionTests):
    def test_allowlist_without_call_is_capability_only(self):
        f = self.root / 'static/js/auth-admin-client.js'
        f.write_text("const ROUTES = ([['POST', 'keys/rotate']]);")
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'capability only')
        f.write_text(f.read_text() + " call('POST', 'keys/rotate');")
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')

    def test_unused_wrapper_is_capability_only(self):
        (self.root / 'static/js/api-client.js').write_text('      releases(slug) { return queryOnce("listProjects"); }')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'capability only')
        (self.root / 'static/js/page.js').write_text('api.projects.releases(slug)')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')

    def test_wrapper_namespace_collision_and_alias(self):
        (self.root / 'static/js/api-client.js').write_text('projects: Object.freeze({\n  get(slug) { return queryOnce("listProjects"); }\n});')
        page = self.root / 'static/js/page.js'
        page.write_text('other.get(slug)')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'capability only')
        page.write_text('api?.projects?.get?.(slug)')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')
        page.write_text('const projectAPI = () => window.axiomAPI?.projects; projectAPI().get(slug);')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')
        page.write_text('const lookup = api?.projects?.get; lookup(slug);')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')

    def test_unused_download_wrapper_is_capability_only(self):
        (self.root / 'static/js/api-client.js').write_text('      latestContractDownloadURL(slug) { return `${apiBase()}/api/v1/projects/${slug}/contracts/latest.axiom`; }')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'capability only')
        (self.root / 'static/js/page.js').write_text('api.projects.latestContractDownloadURL(slug)')
        self.assertEqual(checker.compare(self.scan(), [])[0]['status'], 'gap')

    def test_realtime_routes_are_not_calls(self):
        (self.root / 'static/js/api-client.js').write_text('const DASHBOARD_REALTIME_ROUTES = ["axiom.list_projects", "axiom.create_project",];')
        self.assertEqual(self.scan(), [])

    def test_dead_partials_are_excluded(self):
        for name in ('import_terminal.tmpl', 'welcome_demographics.tmpl'):
            (self.root / 'templates' / name).write_text('sdk.axiom.createOrg(); axiom.list_projects()')
        self.assertEqual(self.scan(), [])

    def test_auth_policy_group(self):
        self.assertEqual(group_for(P + '/auth/policy/releases'), 'Authentication administration')

    def test_action_label_ignores_control_flow(self):
        (self.root / 'static/js/page.js').write_text("async confirmRevoke(){ if (allowed) { this.request('/things', {method:'POST'}); } }")
        self.assertIn('confirmRevoke', self.scan()[0]['action'])

class VerifierCliTests(__import__('unittest').TestCase):
    cases = [
        ('POST', P+'/resources/{id}/conversions', 'mapped'),
        ('GET', P+'/resources/{id}/conversions/{id}', 'mapped'),
        ('GET', '/api/v1/resources/receipt-keys', 'mapped'),
        ('GET', P+'/auth/policy/releases', 'partial'),
        ('GET', P+'/auth/environments/{environment}/roles', 'partial'),
        ('GET', P+'/contracts', 'mapped'),
        ('GET', P+'/contracts/{version}', 'mapped'),
        ('GET', P, 'mapped'),
    ]
    def test_fixed_routes_and_removing_only_call_drops_coverage(self):
        # Real source, real CALLERS; dispatch stays intact, including main.rs.
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root/'cli').mkdir()
            shutil.copytree(CORE/'cli/src', root/'cli/src')
            original = parity_cli.scan_cli(root)
            for method, path, status in self.cases:
                with self.subTest(path=path, method=method):
                    key = checker.route_key(method, path)
                    action = dict(method=method,path=path,action='test',group='test',source='test',line=1)
                    self.assertEqual(checker.compare([action], original)[0]['status'], status)
                    callers = [c for c in parity_cli.CALLERS if checker.route_key(c['method'],c['path']) == key]
                    saved = {}
                    for c in callers:
                        f = root/c['source']; saved.setdefault(f, f.read_text())
                        f.write_text(f.read_text().replace(c['evidence'], '/* call removed */'))
                        self.assertIn(c['command_evidence'], f.read_text())
                    self.assertEqual(checker.compare([action], parity_cli.scan_cli(root))[0]['status'], 'gap')
                    for f, text in saved.items(): f.write_text(text)

    def test_grouped_normalization_and_partial_labels(self):
        def row(path, status):
            return dict(method='GET',path=path,status=status,action='lookup',group='Test',source='x',line=1,commands=[],cli_evidence=[])
        rows = [row('/things/{slug}', 'gap'),row('/things/{id}', 'gap'),row('/other', 'partial')]
        text = checker.markdown(rows, 'f', 'c').split('## Complete matrix')[0]
        self.assertEqual(text.count('**gap**'), 1)
        self.assertIn('1 gaps, 1 partial', text)
        self.assertIn('**partial**', text)
