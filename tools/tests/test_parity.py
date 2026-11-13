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
        (self.root / 'static/js/data-editor-client.js').write_text("const ROUTES = ([\n['POST', 'sql/read', 'data.rows.read', ''],\n]);")
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
