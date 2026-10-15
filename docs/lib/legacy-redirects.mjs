/** Permanent, direct redirects for published documentation URLs. */
export const legacyRedirects = Object.freeze(
  [
  {
    "source": "/cloud/deplying-mocks",
    "destination": "/cloud/deploying-mocks",
    "permanent": true
  },
  {
    "source": "/acore-language/database-contracts",
    "destination": "/database/contracts",
    "permanent": true
  },
  {
    "source": "/acore-language/database-fleets",
    "destination": "/database/fleets",
    "permanent": true
  },
  {
    "source": "/acore-language/database-persistence",
    "destination": "/database/persistence",
    "permanent": true
  },
  {
    "source": "/acore-language/frontend-testing",
    "destination": "/testing/frontend-applications",
    "permanent": true
  },
  {
    "source": "/acore-language/sqlite-contracts",
    "destination": "/database/sqlite",
    "permanent": true
  },
  {
    "source": "/acore-language/ui-profile",
    "destination": "/frontend/acore-ui",
    "permanent": true
  },
  {
    "source": "/clients/flutter-dart",
    "destination": "/frontend/flutter-dart",
    "permanent": true
  },
  {
    "source": "/clients",
    "destination": "/frontend",
    "permanent": true
  },
  {
    "source": "/clients/react-atmx",
    "destination": "/frontend/react",
    "permanent": true
  },
  {
    "source": "/clients/swift-ios",
    "destination": "/frontend/swift",
    "permanent": true
  },
  {
    "source": "/clients/vanilla-web-atmx",
    "destination": "/frontend/web",
    "permanent": true
  },
  {
    "source": "/guides/ci-contract-gate",
    "destination": "/testing/ci",
    "permanent": true
  },
  {
    "source": "/guides/example-catalog",
    "destination": "/examples",
    "permanent": true
  },
  {
    "source": "/guides/production-readiness",
    "destination": "/delivery/production-readiness",
    "permanent": true
  },
  {
    "source": "/reference/server-adapters",
    "destination": "/backend/adapters",
    "permanent": true
  },
  {
    "source": "/reference/server-cli-migration",
    "destination": "/backend/cli-migration",
    "permanent": true
  },
  {
    "source": "/reference/server-containers",
    "destination": "/backend/containers",
    "permanent": true
  },
  {
    "source": "/reference/server-delivery",
    "destination": "/backend/events-and-jobs",
    "permanent": true
  },
  {
    "source": "/reference/server-execution-plan",
    "destination": "/backend/execution",
    "permanent": true
  },
  {
    "source": "/reference/server-integrations",
    "destination": "/backend/integrations",
    "permanent": true
  },
  {
    "source": "/reference/server-policies",
    "destination": "/backend/policies",
    "permanent": true
  },
  {
    "source": "/reference/server-release-workflow",
    "destination": "/backend/release-workflow",
    "permanent": true
  },
  {
    "source": "/reference/server-simulation",
    "destination": "/testing/server-simulation",
    "permanent": true
  },
  {
    "source": "/reference/server-targets-and-releases",
    "destination": "/backend/targets-and-releases",
    "permanent": true
  },
  {
    "source": "/reference/server-workers",
    "destination": "/backend/workers",
    "permanent": true
  },
  {
    "source": "/reference/server-workflow",
    "destination": "/backend/local-development",
    "permanent": true
  },
  {
    "source": "/reference/typed-request-batching",
    "destination": "/backend/request-batching",
    "permanent": true
  },
  {
    "source": "/tooling/axiom-mock-engine",
    "destination": "/testing/mock-engine",
    "permanent": true
  },
  {
    "source": "/tooling/axiom-test-runner",
    "destination": "/testing/test-runner",
    "permanent": true
  },
  {
    "source": "/workflows/acore-ui-application-lifecycle",
    "destination": "/frontend/application-lifecycle",
    "permanent": true
  },
  {
    "source": "/workflows/auditing-and-diffing",
    "destination": "/testing/semantic-diffs",
    "permanent": true
  },
  {
    "source": "/workflows/backend-contract-lifecycle",
    "destination": "/backend/contract-lifecycle",
    "permanent": true
  },
  {
    "source": "/workflows/contract-pull-requests",
    "destination": "/delivery/contract-pull-requests",
    "permanent": true
  },
  {
    "source": "/workflows/existing-frontend-integration",
    "destination": "/frontend/integration-workflow",
    "permanent": true
  },
  {
    "source": "/workflows/local-validation-loop",
    "destination": "/testing/local-and-ci",
    "permanent": true
  },
  {
    "source": "/workflows/mocking-apis",
    "destination": "/testing/mocks",
    "permanent": true
  },
  {
    "source": "/workflows/release-and-upgrade",
    "destination": "/delivery/releases",
    "permanent": true
  },
  {
    "source": "/workflows/security-review-and-release-gates",
    "destination": "/delivery/security-review",
    "permanent": true
  },
  {
    "source": "/workflows/testing-contracts",
    "destination": "/testing/contracts",
    "permanent": true
  }
]
);

// Reader URLs are public interfaces too. Keep saved agent links working when
// the corresponding human-facing page moves.
export const legacyReaderRedirects = Object.freeze(legacyRedirects.map(({ source, destination }) => ({
  source: `/docs${source}.mdx`,
  destination: `/docs${destination}.mdx`,
  permanent: true,
})));
