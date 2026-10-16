# P2Puick desktop

Shell **Tauri 2 + Angular**. Voir le [README racine](../../README.md) pour l’overview.

## Scripts

Tous les scripts npm sont cross-platform (Node `.mjs`, pas de bash requis).

```bash
npm install
npm run tauri:dev    # développement
npm run tauri:try    # build release sans updater / sans clé
npm run tauri:build  # build complet
npm run test:rust    # tests des crates Rust
npm run update -- "Notes de version"   # bump + updater signé (OS courant)
```

Après `tauri:try` (artefacts dans le `target/` **à la racine du monorepo**, pas dans `src-tauri/target/`) :

```bash
# macOS
open ../../target/release/bundle/macos/P2Puick.app

# Windows
start ../../target/release/bundle/nsis/P2Puick_*_x64-setup.exe
# ou : ..\..\target\release\P2Puick.exe

# Linux
../../target/release/p2puick
```

Mises à jour : [docs/UPDATES.md](docs/UPDATES.md).
