# Automatic updates — P2Puick

The app uses the official **Tauri Updater** plugin. Installed copies check an HTTPS endpoint and can download a signed build.

## Flow

1. Install a build of P2Puick.
2. Publish a new version (platform archive + `latest.json` + `changelog.json`) on GitHub Releases (`ProdStalker/p2puick-updates`).
3. On launch (or via **Mises à jour**), the app offers to install the new version.

## One-time setup

### Endpoint

`src-tauri/tauri.conf.json`:

```json
"endpoints": [
  "https://github.com/ProdStalker/p2puick-updates/releases/latest/download/latest.json"
]
```

### Signing keys

| File | Role |
|------|------|
| `src-tauri/updater.key` | **Private** — never share or commit |
| `src-tauri/updater.key.pub` | Public — copied into `tauri.conf.json` |

```bash
npx tauri signer generate -w src-tauri/updater.key -f --ci
```

## Publish an update

Scripts are **Node.js** (`.mjs`) and run on Windows, macOS, and Linux:

| Host OS | Bundle | `latest.json` platforms |
|---------|--------|-------------------------|
| macOS | universal `P2Puick.app.tar.gz` | `darwin-aarch64`, `darwin-x86_64` |
| Windows | `P2Puick-setup.nsis.zip` | `windows-x86_64` |
| Linux | `P2Puick.AppImage.tar.gz` | `linux-x86_64` |

```bash
cd apps/desktop
npm run update -- "Notes de version"
```

Other bumps:

```bash
npm run update -- --minor "Nouvelle fonctionnalité"
npm run update -- --version 0.2.0 --notes "Fixes."
npm run update -- --no-upload --notes "WIP"
```

Each run merges into local `updates/latest.json` (existing platform entries are kept). Publish from each OS (or CI) so all platforms appear in the same release.

## In the app

- **Startup**: silent check; prompt only if a newer version exists.
- **Manual**: **Mises à jour** in the header.
- **Historique**: loads `changelog.json`.

## Troubleshooting

| Issue | Likely cause |
|-------|----------------|
| Check failed + `error decoding response body` | Invalid `latest.json` (often a UTF-8 BOM) |
| Check failed | Wrong URL, no HTTPS, or missing `latest.json` |
| Update rejected | Wrong private key when building |
| No prompt | Version in `latest.json` ≤ installed version |
| `tauri:try` / `update` fails on Windows with `.sh` | Use the npm scripts (`node ./scripts/*.mjs`) — do not call the `.sh` wrappers on cmd.exe |
