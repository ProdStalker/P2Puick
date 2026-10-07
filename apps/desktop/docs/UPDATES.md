# Automatic updates — P2Puick

The app uses the official **Tauri Updater** plugin. Installed copies check an HTTPS endpoint and can download a signed build.

## Flow

1. Install a build of P2Puick.
2. Publish a new version (`P2Puick.app.tar.gz` + `latest.json` + `changelog.json`) on GitHub Releases (`ProdStalker/p2puick-updates`).
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
