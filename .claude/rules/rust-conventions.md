---
paths:
  - "**/*.rs"
---

# Conventions Rust

Trois skills Rust sont versionnés dans le repo (`.agents/skills/`, symlinks `.claude/skills/`) — **les invoquer au début de la tâche concernée** et passer la consigne aux subagents :

- **`rust-best-practices`** (handbook Apollo) : pas de `unwrap()`/`expect()` hors tests, `thiserror` pour les libs / `anyhow` pour les binaires, `&str`/`&[T]` en paramètres, itérateurs plutôt que boucles manuelles. Référence par défaut pour tout code Rust.
- **`rust-async-patterns`** : Tokio, async traits, concurrence. À mobiliser pour le code async — notamment le pipeline `arome-om-forecast` (streams `buffered` du dé-cumul précip, cf. `arome-om-forecast.md`).
- **`rust-testing`** : tests unitaires/intégration, async, property-based, mocking, couverture (TDD). À mobiliser pour écrire ou étendre les tests (le filet de 68 tests).

Les binaires sont fins : orchestration dans `main.rs` (doc-comment d'entête décrivant les étapes), logique réutilisable dans `crates/core` (`pipeline-core`).
