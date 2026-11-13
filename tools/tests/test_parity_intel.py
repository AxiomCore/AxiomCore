"""Intel mappings need live command arms, dispatch, and the authenticated sender."""
import shutil
import tempfile
import unittest
from pathlib import Path
from test_parity import checker
import parity_cli

CORE = Path(__file__).resolve().parents[2]


class IntelParityTests(unittest.TestCase):
    def test_all_37_routes_have_reviewed_evidence(self):
        rows = [r for r in parity_cli.scan_cli(CORE) if r['command'].startswith('axiom intel ')]
        self.assertEqual(len(rows), 37)
        self.assertEqual(len({checker.route_key(r['method'], r['path']) for r in rows}), 37)
        self.assertTrue(all(not r.get('note') for r in rows))

    def test_removing_sender_or_registration_removes_all_intel_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(CORE / 'cli/src', root / 'cli/src')
            for filename, anchor in [
                ('cli/src/commands/intel.rs', '.dashboard_request('),
                ('cli/src/main.rs', 'Commands::Intel(args) => commands::intel::handle(args).await'),
            ]:
                with self.subTest(filename=filename):
                    path = root / filename
                    original = path.read_text()
                    self.assertIn(anchor, original)
                    path.write_text(original.replace(anchor, 'removed_sender'))
                    self.assertFalse(any(r['command'].startswith('axiom intel ') for r in parity_cli.scan_cli(root)))
                    path.write_text(original)

    def test_removing_a_request_arm_drops_its_mapping(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(CORE / 'cli/src', root / 'cli/src')
            path = root / 'cli/src/commands/intel_routes.rs'
            original = path.read_text()
            for caller in [c for c in parity_cli.CALLERS if c['command'].startswith('axiom intel ')]:
                with self.subTest(command=caller['command']):
                    path.write_text(original.replace(caller['evidence'], '/* removed arm */'))
                    self.assertNotIn(caller['command'], [r['command'] for r in parity_cli.scan_cli(root)])
            path.write_text(original)
