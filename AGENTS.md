# Project Rules

## Releases

- Whenever you push code to GitHub, also trigger a new build. Pushing to `main` alone does **not** build anything — the Release workflow (`.github/workflows/release.yml`) only runs on a `v*` tag push or a manual dispatch.
- To build the current `main` (including UI-only changes that don't come with a redumper bump), run the Release workflow after pushing:

  ```bash
  gh workflow run release.yml --ref main
  ```

  This builds `main` HEAD and updates the existing `v<release>` draft release with the new artifacts.
- The release version is derived from the bundled redumper tag in `.redumper/upstream.json` (`b729` → release `729` → tag `v729`), not from `package.json`. `package.json`'s version is only recorded as the "internal app version" in the release notes, so bumping it does not change the release number or trigger a build.
- Do not force-move a published `v*` tag to a new commit unless the user explicitly asks — prefer the manual dispatch above.

## Release policy

- The Release workflow auto-publishes at the end of the build (the `publish-release` job flips the release out of draft) — never leave a release as a draft, and never require manual publishing.
- Only the newest release is kept. The `publish-release` job's "Remove previous releases" step (`keepCount = 1`) deletes every older release and its `v*` tag, so exactly one "Release" is present at any time.
