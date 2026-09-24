from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ctl
import release_cli


class CandidatePreflightTests(unittest.TestCase):
    def test_clean_prepared_train_reports_ci_gap_without_building(self):
        intent = {"trainId": "train.2", "changes": [{"component": "sdk", "type": "fix",
                                                  "summary": "Update SDK.", "version": "1.0.1"}]}
        plan = {"blocked": [], "blockedVersions": [], "components": [
            {"id": "sdk", "selected": True, "adapter": "ci-only"}]}
        with tempfile.TemporaryDirectory(prefix="axiom-preflight-test-") as temporary:
            root = Path(temporary)
            evidence = root / "trains/train.2/preparation.json"
            evidence.parent.mkdir(parents=True)
            evidence.write_text(json.dumps({"applied": True,
                                            "intentSha256": ctl.sha256(ctl.canonical(intent))}))
            output = io.StringIO()
            with patch.dict(os.environ, {"AXIOM_RELEASE_BUILD_ROOT": str(root)}), \
                    patch.object(release_cli.ctl, "make_plan", return_value=plan), \
                    redirect_stdout(output):
                release_cli.candidate_preflight({}, intent)
            self.assertIn("Local candidate preflight passed", output.getvalue())
            self.assertIn("CI builder gap: sdk", output.getvalue())
            self.assertIn("No candidate was built or published", output.getvalue())


if __name__ == "__main__":
    unittest.main()
