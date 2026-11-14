"""P2 caller coverage must retain explicit browser-session restrictions."""
import unittest
from pathlib import Path
from test_parity import checker
import parity_cli

CORE = Path(__file__).resolve().parents[2]


class DataParityTests(unittest.TestCase):
    def test_all_data_routes_have_callers_and_browser_only_remains_partial(self):
        callers = parity_cli.scan_cli(CORE)
        data = [r for r in callers if r['command'].startswith('axiom data ')]
        self.assertEqual(len(data), 20)
        self.assertEqual(sum(bool(r.get('note')) for r in data), 7)
        for row in data:
            action = {**row, 'group': 'Data', 'action': row['command']}
            status = checker.compare([action], data)[0]['status']
            self.assertEqual(status, 'partial' if row.get('note') else 'mapped')

    def test_environment_metadata_and_release_impact_have_callers(self):
        commands = {r['command'] for r in parity_cli.scan_cli(CORE)}
        self.assertTrue({'axiom secrets environment list',
                         'axiom secrets environment bindings',
                         'axiom database impact get'} <= commands)
