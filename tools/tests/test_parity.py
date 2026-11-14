import importlib.util
from pathlib import Path
import sys
import unittest

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('parity_check', TOOLS / 'parity-check.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class ComparisonTests(unittest.TestCase):
    def action(self, method='GET', path='/api/v1/projects/{slug}/branches'):
        return dict(method=method, path=path, action='listBranches', source='page.js', line=1, group='Branches')

    def test_parameter_names_are_not_gaps(self):
        cli = [dict(method='GET', path='/api/v1/projects/{project}/branches', command='axiom branch list')]
        self.assertEqual(checker.compare([self.action()], cli)[0]['commands'], ['axiom branch list'])

    def test_same_path_wrong_method_is_gap(self):
        cli = [dict(method='POST', path='/api/v1/projects/{project}/branches', command='axiom branch create')]
        self.assertEqual(checker.compare([self.action()], cli)[0]['status'], 'gap')

    def test_similar_suffix_different_api_is_gap(self):
        cli = [dict(method='GET', path='/api/v1/projects/{project}/data/branches', command='axiom branch list')]
        self.assertEqual(checker.compare([self.action()], cli)[0]['status'], 'gap')

    SESSION = dict(method='GET', path='/api/v1/auth/session', action='GetSession', source='a.js', line=1, group='Account authentication')
    REASON = 'Browser-session route of the dashboard sign-in; the CLI uses device login.'

    def exemption(self, reason=REASON, category='browser-session'):
        return {'category': category, 'reason': reason}

    def test_valid_exemption_marks_the_route_exempt(self):
        exempt = {('GET', '/api/v1/auth/session'): self.exemption()}
        row = checker.compare([self.SESSION], [], exempt)[0]
        self.assertEqual((row['status'], row['exempt_reason'], row['exempt_category']), ('exempt', self.REASON, 'browser-session'))
        self.assertEqual(checker.grouped_gaps([row]), [])
        self.assertEqual(checker.compare([self.SESSION], [])[0]['status'], 'gap')

    def test_exemption_without_a_reason_fails_the_checker(self):
        for reason in ['', ' ', '\n\t', None, 7]:
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, 'no reason'):
                checker.compare([self.SESSION], [], {('GET', '/api/v1/auth/session'): self.exemption(reason)})
        for bare in ['a reason as a bare string', '', None]:
            with self.subTest(bare=bare), self.assertRaisesRegex(ValueError, 'category'):
                checker.compare([self.SESSION], [], {('GET', '/api/v1/auth/session'): bare})

    def test_exemption_outside_an_allowlisted_category_fails_the_checker(self):
        for category in [None, '', 'not-needed', 'Browser-Session', 'browser-session ']:
            with self.subTest(category=category), self.assertRaisesRegex(ValueError, 'not an allowed exemption category'):
                checker.compare([self.SESSION], [], {('GET', '/api/v1/auth/session'): self.exemption(category=category)})

    def test_exemption_of_a_real_gap_fails_the_checker(self):
        # The verifier's reproduction: GET /api/v1/workspace with no CLI mapping was exempt
        # with any reason. A route outside the category stays a gap, whatever the reason.
        workspace = dict(method='GET', path='/api/v1/workspace', action='GetWorkspace', source='w.js', line=1, group='Workspace')
        for key in [('GET', '/api/v1/workspace'), ('PUT', '/api/v1/auth/session'), ('GET', '/api/v1/auth/session/extra'), ('GET', '/api/v1/projects/{slug}/auth/environments/{env}/sessions')]:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'not a browser-session route'):
                checker.compare([workspace], [], {key: self.exemption('arbitrary gap reason')})

    def test_exemption_used_by_another_dashboard_flow_fails_the_checker(self):
        # An allowlisted route the dashboard also calls from a main flow is a gap there.
        elsewhere = {**self.SESSION, 'group': 'Workspace', 'action': 'loadWorkspaceSession'}
        with self.assertRaisesRegex(ValueError, 'uses it in Workspace'):
            checker.compare([self.SESSION, elsewhere], [], {('GET', '/api/v1/auth/session'): self.exemption()})

    def test_exemption_called_from_another_dashboard_operation_fails_the_checker(self):
        # The dashboard groups by path, so a main flow reaching an allowlisted route shows up
        # as another action (and source) on the same route, not as another group.
        for other in [{**self.SESSION, 'action': 'loadWorkspaceMembers', 'source': 'static/js/pages/workspace.js'},
                      {**self.SESSION, 'action': 'request', 'source': 'static/js/api-client.js', 'line': 900},
                      {**self.SESSION, 'action': None}]:
            with self.subTest(other=other), self.assertRaisesRegex(ValueError, 'not a browser-session operation'):
                checker.compare([self.SESSION, other], [], {('GET', '/api/v1/auth/session'): self.exemption()})
        wrong_route = {**self.SESSION, 'method': 'DELETE', 'action': 'GetSession'}
        with self.assertRaisesRegex(ValueError, 'calls it from GetSession'):
            checker.compare([wrong_route], [], {('DELETE', '/api/v1/auth/session'): self.exemption()})

    def test_malformed_exemption_key_fails_the_checker(self):
        for key in [('GET',), ('GET', '/api/v1/auth/session', 'x'), (None, '/api/v1/auth/session'), 'GET /api/v1/auth/session']:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, r'key must be \(METHOD, path\)'):
                checker.compare([self.SESSION], [], {key: self.exemption()})

    def test_exemption_of_a_mapped_route_fails_the_checker(self):
        exempt = {('GET', '/api/v1/auth/session/'): self.exemption()}
        for cli in [[dict(method='GET', path='/api/v1/auth/session', command='axiom whoami')],
                    [dict(method='GET', path='/api/v1/auth/{x}', command='axiom other')] + [dict(method='get', path='/api/v1/auth/session?full=1', command='axiom whoami', note='partial')]]:
            with self.subTest(cli=cli), self.assertRaisesRegex(ValueError, 'a CLI command maps this route'):
                checker.compare([self.SESSION], cli, exempt)

    def test_duplicate_exemption_after_normalisation_fails(self):
        exempt = {('GET', '/api/v1/auth/session'): self.exemption(), ('get', '/api/v1/auth/session/'): self.exemption()}
        with self.assertRaisesRegex(ValueError, 'exempted twice'):
            checker.compare([self.SESSION], [], exempt)

    def test_invalid_exemption_makes_the_checker_exit_2(self):
        from unittest import mock
        import io
        import parity_cli
        import parity_dashboard
        bad = {('GET', '/api/v1/workspace'): {'category': 'browser-session', 'reason': ' '}}
        workspace = dict(method='GET', path='/api/v1/workspace', action='GetWorkspace', source='w.js', line=1, group='Workspace')
        with mock.patch.object(parity_dashboard, 'scan_dashboard', return_value=[workspace]), \
             mock.patch.object(parity_cli, 'scan_cli', return_value=[]), \
             mock.patch.object(parity_cli, 'EXEMPT', bad), \
             mock.patch.object(sys, 'argv', ['parity-check.py', '--frontend', '.', '--strict']), \
             mock.patch('sys.stderr', new_callable=io.StringIO) as err, mock.patch('sys.stdout', new_callable=io.StringIO):
            self.assertEqual(checker.main(), 2)
        self.assertIn('no reason', err.getvalue())
        self.assertIn('not a browser-session route', err.getvalue())

    def test_every_registered_exemption_is_a_valid_browser_session_route(self):
        import parity_cli
        self.assertEqual(len(parity_cli.EXEMPT), 5)
        rows = parity_cli.scan_cli(TOOLS.parent)
        valid = checker.validate_exemptions(parity_cli.EXEMPT, [], rows)
        allowed = {checker.route_key(*r) for r in checker.EXEMPTION_CATEGORIES['browser-session']['routes']}
        # Against the real dashboard source too, when the sibling checkout is there.
        frontend = TOOLS.parent.parent / 'axiom-frontend'
        if frontend.is_dir():
            import parity_dashboard
            rows = checker.compare(parity_dashboard.scan_dashboard(frontend), rows, parity_cli.EXEMPT)
            self.assertEqual({checker.route_key(r['method'], r['path']) for r in rows if r['status'] == 'exempt'}, allowed)
        self.assertEqual(set(valid), allowed)
        for entry in valid.values():
            self.assertEqual(entry['category'], 'browser-session')
            self.assertGreater(len(entry['reason']), 40)

    def test_query_and_trailing_slash_normalized(self):
        self.assertEqual(checker.route_key('get', '/api/v1/items/?limit=5'), ('GET', '/api/v1/items'))

    def test_multiple_actions_and_commands_retained(self):
        cli = [dict(method='GET', path='/api/v1/projects/{id}/branches', command=c) for c in ['axiom branch list', 'axiom branch inspect']]
        rows = checker.compare([self.action(), {**self.action(), 'action': 'refreshBranches'}], cli)
        self.assertEqual(len(rows), 2)
        self.assertEqual(len(rows[0]['commands']), 2)

    def test_gap_report_has_every_missing_route(self):
        rows = checker.compare([self.action(), self.action('POST')], [])
        doc = checker.markdown(rows, 'frontend-sha', 'core-sha')
        self.assertIn('GET /api/v1/projects/{slug}/branches', doc)
        self.assertIn('POST /api/v1/projects/{slug}/branches', doc)
        self.assertIn('2 endpoints with missing', doc)


if __name__ == '__main__':
    unittest.main()


class DashboardExtractionTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        import json
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'static/js').mkdir(parents=True)
        (self.root / 'templates').mkdir()
        (self.root / 'static/js/api-client.js').write_text('')
        (self.root / 'public').mkdir()
        (self.root / 'public/axiom.axiom').write_text(json.dumps({'endpoints': [
            {'name': 'ListProjects', 'method': 'GET', 'path': '/api/v1/projects'},
            {'name': 'CreateProject', 'method': 'POST', 'path': '/api/v1/projects'}]}))
        for name in ['auth-admin-client.js', 'data-editor-client.js']:
            (self.root / 'static/js' / name).write_text("const ROUTES = ([\n]);")

    def scan(self):
        from parity_dashboard import scan_dashboard
        return scan_dashboard(self.root)

    def test_template_and_js_calls_use_contract_methods(self):
        (self.root / 'templates/page.tmpl').write_text('axiom.list_projects()')
        (self.root / 'static/js/page.js').write_text("mutate('createProject', {});")
        rows = self.scan()
        self.assertEqual({(r['method'], r['path']) for r in rows}, {('GET', '/api/v1/projects'), ('POST', '/api/v1/projects')})

    def test_unused_contract_operation_is_not_coverage(self):
        self.assertEqual(self.scan(), [])

    def test_new_sdk_operation_without_contract_fails(self):
        (self.root / 'static/js/page.js').write_text("mutate('unknownMutation', {});")
        with self.assertRaises(ValueError):
            self.scan()

    def test_typed_table_addition_becomes_gap(self):
        (self.root / 'static/js/data-editor-client.js').write_text("const ROUTES = ([\n['POST', 'sql/read', 'data.rows.read', ''],\n]); call('POST', 'sql/read');")
        rows = checker.compare(self.scan(), [])
        self.assertEqual(rows[0]['method'], 'POST')
        self.assertTrue(rows[0]['path'].endswith('/data/environments/{environment}/sql/read'))
        self.assertEqual(rows[0]['status'], 'gap')

    def test_unknown_path_concat_fails_instead_of_dropping_identifier(self):
        (self.root / 'static/js/page.js').write_text("this.request('/things/' + id)")
        with self.assertRaisesRegex(ValueError, 'unresolved path expression'):
            self.scan()

    def test_unknown_options_fail_instead_of_assuming_get(self):
        (self.root / 'static/js/page.js').write_text("this.request('/things', options)")
        with self.assertRaisesRegex(ValueError, 'unresolved request options'):
            self.scan()


class CliEvidenceTests(unittest.TestCase):
    def test_deleted_dispatch_or_call_does_not_establish_coverage(self):
        import tempfile
        from unittest.mock import patch
        import parity_cli
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'cli/src').mkdir(parents=True)
            main = root / 'cli/src/main.rs'
            source = root / 'cli/src/action.rs'
            main.write_text('Commands::Auth => handle()')
            source.write_text('Action::Show => client.get_settings()')
            caller = dict(command='axiom auth show', method='GET', path='/settings', source='cli/src/action.rs', evidence='client.get_settings()', command_evidence='Action::Show')
            with patch.object(parity_cli, 'CALLERS', [caller]):
                self.assertEqual(len(parity_cli.scan_cli(root)), 1)
                source.write_text('// Action::Show => client.get_settings()')
                self.assertEqual(parity_cli.scan_cli(root), [])
                source.write_text('Action::Show => client.get_settings()')
                main.write_text('Commands::Other => handle()')
                self.assertEqual(parity_cli.scan_cli(root), [])

    def test_test_only_call_does_not_establish_coverage(self):
        from parity_cli import production
        self.assertNotIn('client.post()', production('fn main() {}\n#[cfg(test)] mod tests { client.post(); }'))

    def test_bulk_workflow_remains_partial(self):
        row = dict(method='DELETE', path='/providers/{id}', action='delete provider', source='client.js', line=1, group='Auth')
        cli = [{**row, 'command': 'axiom auth config promote', 'note': 'Bulk only'}]
        result = checker.compare([row], cli)
        self.assertEqual(result[0]['status'], 'partial')
        doc = checker.markdown(result, 'f', 'c')
        self.assertIn('Bulk only', doc)
        self.assertIn('1 endpoints with missing or partial', doc)
