# Redumper UI

Cross-platform desktop frontend for [`superg/redumper`](https://github.com/superg/redumper).

The app is built with Tauri v2, Rust, React, TypeScript, and Tailwind CSS. It
wraps the official redumper CLI with a typed command UI, streamed logs, and
platform-aware packaging.

## Development

```sh
npm install
npm run prepare-redumper
npm run tauri:dev
```

`prepare-redumper` downloads the pinned upstream binary for the current target
into `src-tauri/resources/redumper/`. The downloaded binaries are intentionally
not tracked in git.

## Useful Commands

```sh
npm run dev
npm run build
npm run test
npm run typecheck
npm run update-upstream
```

## Release Model

The `Check Upstream Release` workflow compares the latest upstream redumper tag
against `.redumper/upstream.json`. If it changes, the workflow updates the
manifest and app version, commits that change, and starts the cross-platform
release workflow.

Release builds target Windows x64/ARM64, macOS x64/ARM64, and Linux x64/ARM64.
The release workflow publishes draft releases in
[`whatever-industries/redumper-ui`](https://github.com/whatever-industries/redumper-ui)
and promotes the release after all matrix jobs finish.

## App Updates

The UI includes a `Check for Updates` control backed by Tauri's signed updater.
It checks the latest GitHub release metadata at
`https://github.com/whatever-industries/redumper-ui/releases/latest/download/latest.json`.

Updater installs require signed updater artifacts. Generate the updater key with:

```sh
npm run tauri signer generate -- -w ~/.tauri/redumper-ui.key
```

Store the private key in the `TAURI_SIGNING_PRIVATE_KEY` GitHub secret and the
optional password in `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The generated public
key is committed in `src-tauri/tauri.conf.json`; if you regenerate the keypair,
replace that `plugins.updater.pubkey` value with the new public key.

To emit updater artifacts in CI, set the `CREATE_UPDATER_ARTIFACTS` repository
variable to `true`, or enable `create_updater_artifacts` when manually running
the release workflow.

## Licensing

This repository is GPL-compatible because release artifacts bundle upstream
redumper, which is GPL-3.0 licensed. See `NOTICE.md` for attribution.
