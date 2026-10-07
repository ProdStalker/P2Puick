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

Prérequis : Rust, Node.js, dépendances Tauri pour ton OS.

```bash
cd apps/desktop
npm install
npm run tauri:dev
```

Tests Rust :

```bash
cargo test -p p2puick-core
```

## Branches

- `master` — branche principale
- `staging` — pré-production

## Mises à jour

Voir [apps/desktop/docs/UPDATES.md](apps/desktop/docs/UPDATES.md).

Repo binaires : [ProdStalker/p2puick-updates](https://github.com/ProdStalker/p2puick-updates).

## Licence

MIT
