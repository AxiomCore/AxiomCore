#!/usr/bin/env bash
set -euo pipefail

docs_root="$(cd "$(dirname "$0")/.." && pwd)"
repository_root="$(cd "$docs_root/.." && pwd)"

mode="${1:-production}"
requested_branch="${2:-}"
project="${CLOUDFLARE_PAGES_PROJECT:-axiomcore-docs}"
production_branch="${CLOUDFLARE_PAGES_PRODUCTION_BRANCH:-main}"

for required in CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_API_TOKEN; do
  value="${!required:-}"
  if [[ -z "$value" || "$value" == your-* ]]; then
    echo "Infisical must inject a real ${required} value before deploying." >&2
    exit 1
  fi
done

case "$mode" in
  initial)
    branch="$production_branch"
    ;;
  production)
    branch="$production_branch"
    ;;
  preview)
    branch="$requested_branch"
    if [[ -z "$branch" ]]; then
      branch="$(git -C "$(dirname "$0")/../.." branch --show-current)"
    fi
    if [[ -z "$branch" || "$branch" == "$production_branch" ]]; then
      echo "Preview deployment requires a non-production branch name." >&2
      exit 1
    fi
    ;;
  *)
    echo "Usage: $0 [initial|production|preview] [branch]" >&2
    exit 2
    ;;
esac

commit_hash="$(git -C "$repository_root" rev-parse HEAD)"
commit_message="$(git -C "$repository_root" log -1 --pretty=%s)"
commit_dirty=false
if [[ -n "$(git -C "$repository_root" status --porcelain)" ]]; then
  commit_dirty=true
fi

cd "$docs_root"
pnpm install --frozen-lockfile
pnpm check
pnpm static:check

if [[ "$mode" == "initial" ]]; then
  echo "Creating Cloudflare Pages project '$project' with production branch '$production_branch'."
  pnpm exec wrangler pages project create "$project" --production-branch "$production_branch"
fi

echo "Deploying static output to Cloudflare Pages project '$project' on branch '$branch'."
exec pnpm exec wrangler pages deploy out \
  --project-name "$project" \
  --branch "$branch" \
  --commit-hash "$commit_hash" \
  --commit-message "$commit_message" \
  --commit-dirty "$commit_dirty"
