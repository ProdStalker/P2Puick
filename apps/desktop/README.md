# P2Puick desktop

Shell **Tauri 2 + Angular**. Voir le [README racine](../../README.md) pour l’overview.

## Scripts

```bash
npm install
npm run tauri:dev    # développement
npm run tauri:try    # build release sans updater / sans clé
npm run tauri:build  # build complet
npm run test:rust    # tests des crates Rust
npm run update -- "Notes de version"
```

macOS après `tauri:try` (artefacts dans le `target/` **à la racine du monorepo**, pas dans `src-tauri/target/`) :

```bash
# depuis apps/desktop
open ../../target/release/bundle/macos/P2Puick.app
```

Mises à jour : [docs/UPDATES.md](docs/UPDATES.md).
