# P2Puick

Application desktop **Rust + Tauri 2 + Angular** pour transférer des fichiers en P2P entre deux ordinateurs sur le **même réseau local**.

## Fonctionnalités v1

- Session hôte / rejoindre avec code à 6 chiffres
- Découverte LAN via mDNS (`_p2puick._tcp`) + IP manuelle en fallback
- Transfert multi-fichiers avec intégrité **blake3**
- UI Angular (héberger, rejoindre, progression, historique, mises à jour)
- Compatible **Windows / macOS / Linux**

## Structure

```
P2Puick/
  apps/desktop/           # Angular + src-tauri
  crates/p2puick-core/    # protocole + transfert
  crates/p2puick-discovery/
```

## Développement

Prérequis : Rust, Node.js (≥ 22.22.3), dépendances Tauri pour ton OS.

```bash
cd apps/desktop
npm install
npm run tauri:dev
```

### Scripts npm (`apps/desktop`)

| Script | Description |
|--------|-------------|
| `npm run tauri:dev` | Dev hot-reload (Angular + Rust) |
| `npm run tauri:try` | Build release local **sans** artefacts updater (pas besoin de clé de signature) |
| `npm run tauri:build` | Build complet (updater inclus, clé privée requise si `createUpdaterArtifacts`) |
| `npm run test:rust` | Tests Rust (`p2puick-core`, `p2puick-discovery`) |
| `npm run update -- "notes"` | Bump + build signé + publish vers `p2puick-updates` |

### Essayer en release (local)

Build optimisé **sans** signature updater. Le workspace Cargo place les artefacts à la **racine du repo** (`P2Puick/target/…`), pas sous `apps/desktop/src-tauri/target/`.

```bash
cd apps/desktop
npm install
npm run tauri:try
# macOS (depuis apps/desktop) :
open ../../target/release/bundle/macos/P2Puick.app
# ou depuis la racine du repo :
# open target/release/bundle/macos/P2Puick.app
```

Si tu as déjà build et que seule la signature a échoué, le `.app` est quand même là — `open` suffit.

Build complet (artefacts updater, clé privée requise) :

```bash
npm run tauri:build
```

> Premier build release : plus long (compilation optimisée). Les suivants sont plus rapides.

Tests Rust (depuis la racine ou via npm) :

```bash
cargo test -p p2puick-core
# ou
cd apps/desktop && npm run test:rust
```

## Branches

- `master` — branche principale
- `staging` — pré-production

## Mises à jour

Repo public des binaires : [ProdStalker/p2puick-updates](https://github.com/ProdStalker/p2puick-updates).

```bash
cd apps/desktop
npm run update -- "Notes de version"
npm run update -- --no-upload --notes "WIP"
```

Détails : [apps/desktop/docs/UPDATES.md](apps/desktop/docs/UPDATES.md).

## Licence

MIT
