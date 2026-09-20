# Documentation URL migration policy

Public documentation URLs are an interface. Renaming a file or navigation
label must not silently break links in repositories, package metadata, search
indexes, support conversations, or saved bookmarks.

## Current migration inventory

| Legacy path | Canonical destination | Reason | Redirect |
| --- | --- | --- | --- |
| `/cloud/deplying-mocks` | `/cloud/deploying-mocks` | Correct the historical filename typo | Permanent |

The executable inventory lives in `lib/legacy-redirects.mjs`. Next.js consumes
that inventory directly, and the production smoke test verifies every redirect
without following it.

## Adding or changing a route

1. Keep the old public path in `lib/legacy-redirects.mjs`.
2. Point it directly to the final canonical page; do not create redirect chains.
3. Use a permanent redirect only after the destination and canonical metadata
   are final.
4. Update navigation, internal links, sitemap, machine-reader output, and any
   checked-in examples to use the canonical path.
5. Run `pnpm check` and inspect the preview before promotion.

Redirect sources must be unique, must not also be current documentation routes,
and must not be redirect destinations. Destinations must resolve to a current
page. The content-policy check enforces those rules.

## Removal policy

Do not remove a redirect merely because the current site no longer links to its
source. Removal requires evidence that supported repositories, published
packages, search results under the operator's control, and maintained support
materials no longer reference it. Record that evidence in the review that
removes the entry and test the old URL against the last deployment before
promotion.
