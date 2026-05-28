# AROME-OM `precipitation_sum` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publier une variable dérivée `precipitation_sum` (cumul de précipitation depuis le début du run) dans chaque OMfile de leadtime du pipeline `arome-om-forecast`.

**Architecture:** Une brique pure NaN-aware d'accumulation dans `pipeline-core`, un helper testable dans le crate `arome-om-forecast` qui injecte la slice dérivée, et une restructuration du flux principal pour découpler le décodage parallèle de l'écriture séquentielle (ordonnée par leadtime) afin d'entretenir un accumulateur.

**Tech Stack:** Rust 2024, `ndarray`, `futures::StreamExt` (`buffered`), `tokio`. Tests via `cargo test`.

**Spec:** `docs/superpowers/specs/2026-05-28-arome-om-precipitation-sum-design.md`

---

## File Structure

- **Create** `crates/core/src/accumulation.rs` — fonction pure `accumulate_into` (addition NaN-aware en place). Responsabilité unique : la sémantique du cumul.
- **Modify** `crates/core/src/lib.rs` — déclarer `pub mod accumulation;`.
- **Create** `crates/arome-om-forecast/src/cumul.rs` — consts `DERIVED_PRECIP_SUM` / `PRECIP_OM_NAME` + helper `accumulate_and_inject` (met à jour l'accumulateur depuis la slice `precipitation` et pousse la slice dérivée). Responsabilité : faire le pont entre `DecodedSlice` et la brique `core`.
- **Modify** `crates/arome-om-forecast/src/lib.rs` — déclarer `pub mod cumul;`.
- **Modify** `crates/arome-om-forecast/src/main.rs` — `process_leadtime` → `decode_leadtime` (sans écriture), stream `buffered` ordonné, consommateur séquentiel avec accumulateur, ajout de `DERIVED_PRECIP_SUM` aux `var_names` métadonnées.
- **Modify** `README.md` + `CLAUDE.md` — documenter la nouvelle variable, la restructuration, la brique `core`.

---

## Task 1 : Brique d'accumulation dans `pipeline-core`

**Files:**
- Create: `crates/core/src/accumulation.rs`
- Modify: `crates/core/src/lib.rs:1`
- Test: `crates/core/src/accumulation.rs` (module `#[cfg(test)]` en bas de fichier, comme `anomaly.rs`)

- [ ] **Step 1: Écrire le module avec la fonction et ses tests (test-first dans le même fichier)**

Create `crates/core/src/accumulation.rs` :

```rust
//! Accumulation de grilles avec propagation NaN (cumuls roulants).

use ndarray::{Array2, Zip};

/// Ajoute `hour` dans l'accumulateur `acc`, élément-à-élément, avec propagation
/// NaN. Après l'appel, `acc` contient le cumul incluant `hour`. Une fois un
/// pixel NaN, il reste NaN aux étapes suivantes (cohérent avec la philosophie
/// du projet : on ne masque pas une donnée manquante).
///
/// Utilisé par `arome-om-forecast` pour bâtir `precipitation_sum` (cumul depuis
/// le début du run).
pub fn accumulate_into(acc: &mut Array2<f32>, hour: &Array2<f32>) {
    debug_assert_eq!(acc.dim(), hour.dim(), "shape mismatch in accumulate_into");
    Zip::from(acc).and(hour).for_each(|a, &h| {
        *a = if a.is_nan() || h.is_nan() {
            f32::NAN
        } else {
            *a + h
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn accumulate_into_running_sum() {
        let mut acc = array![[0.0_f32, 0.0], [0.0, 0.0]];
        accumulate_into(&mut acc, &array![[1.0_f32, 2.0], [3.0, 4.0]]);
        assert!((acc[[0, 0]] - 1.0).abs() < 1e-6);
        assert!((acc[[1, 1]] - 4.0).abs() < 1e-6);

        accumulate_into(&mut acc, &array![[0.5_f32, 1.0], [1.0, 0.0]]);
        assert!((acc[[0, 0]] - 1.5).abs() < 1e-6);
        assert!((acc[[0, 1]] - 3.0).abs() < 1e-6);
        assert!((acc[[1, 0]] - 4.0).abs() < 1e-6);
        assert!((acc[[1, 1]] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn accumulate_into_propagates_nan_forever() {
        let mut acc = array![[0.0_f32, 0.0]];
        accumulate_into(&mut acc, &array![[1.0_f32, f32::NAN]]);
        assert!((acc[[0, 0]] - 1.0).abs() < 1e-6);
        assert!(acc[[0, 1]].is_nan());

        // Une heure valide après le trou ne « ressuscite » pas le pixel.
        accumulate_into(&mut acc, &array![[2.0_f32, 5.0]]);
        assert!((acc[[0, 0]] - 3.0).abs() < 1e-6);
        assert!(acc[[0, 1]].is_nan());
    }

    #[test]
    fn accumulate_into_zero_hour_is_noop() {
        let mut acc = array![[2.0_f32, 3.0]];
        accumulate_into(&mut acc, &array![[0.0_f32, 0.0]]);
        assert!((acc[[0, 0]] - 2.0).abs() < 1e-6);
        assert!((acc[[0, 1]] - 3.0).abs() < 1e-6);
    }
}
```

- [ ] **Step 2: Déclarer le module**

Modify `crates/core/src/lib.rs` — ajouter la ligne dans l'ordre alphabétique (juste après `pub mod anomaly_metadata;`) :

```rust
pub mod accumulation;
pub mod anomaly;
pub mod anomaly_metadata;
```

(soit : `accumulation` en première ligne du fichier, avant `anomaly`.)

- [ ] **Step 3: Lancer les tests, vérifier qu'ils passent**

Run: `cargo test -p pipeline-core accumulation`
Expected: PASS — `accumulate_into_running_sum`, `accumulate_into_propagates_nan_forever`, `accumulate_into_zero_hour_is_noop`.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/accumulation.rs crates/core/src/lib.rs
git commit -m "feat(core): add accumulate_into for NaN-aware running sums"
```

---

## Task 2 : Helper d'injection `cumul` dans `arome-om-forecast`

**Files:**
- Create: `crates/arome-om-forecast/src/cumul.rs`
- Modify: `crates/arome-om-forecast/src/lib.rs`
- Test: `crates/arome-om-forecast/src/cumul.rs` (module `#[cfg(test)]`)

- [ ] **Step 1: Écrire le module avec helper et tests**

Create `crates/arome-om-forecast/src/cumul.rs` :

```rust
//! Cumul de précipitation depuis le début du run (`precipitation_sum`).
//!
//! Variable *dérivée* (pas un mapping GRIB → absente du registry `VARIABLES`).
//! Le flux principal entretient un accumulateur et appelle [`accumulate_and_inject`]
//! pour chaque leadtime, dans l'ordre croissant.

use ndarray::Array2;
use pipeline_core::accumulation::accumulate_into;

use crate::grib_decoder::DecodedSlice;

/// Nom de la variable de précip horaire décodée (cf. registry `VARIABLES`).
pub const PRECIP_OM_NAME: &str = "precipitation";

/// Nom de la variable dérivée publiée dans les OMfiles.
pub const DERIVED_PRECIP_SUM: &str = "precipitation_sum";

/// Met à jour `acc` avec la slice `precipitation` de `slices` (si présente —
/// elle est absente à leadtime 0 car `tp` n'existe pas à l'instant initial),
/// puis pousse une slice dérivée `precipitation_sum` (= snapshot de `acc`) dans
/// `slices`.
///
/// `acc` doit être initialisé à zéro (shape de la grille) avant le premier
/// leadtime et réutilisé tel quel pour les suivants.
pub fn accumulate_and_inject(slices: &mut Vec<DecodedSlice>, acc: &mut Array2<f32>, leadtime_h: u32) {
    if let Some(precip) = slices.iter().find(|s| s.om_name == PRECIP_OM_NAME) {
        accumulate_into(acc, &precip.data);
    }
    slices.push(DecodedSlice {
        om_name: DERIVED_PRECIP_SUM,
        leadtime_h,
        data: acc.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, array};

    fn find_sum(slices: &[DecodedSlice]) -> &Array2<f32> {
        &slices
            .iter()
            .find(|s| s.om_name == DERIVED_PRECIP_SUM)
            .expect("precipitation_sum injected")
            .data
    }

    #[test]
    fn accumulates_precip_across_leadtimes() {
        let mut acc = Array2::<f32>::zeros((2, 2));

        let mut s1 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 1,
            data: array![[1.0_f32, 2.0], [3.0, 4.0]],
        }];
        accumulate_and_inject(&mut s1, &mut acc, 1);
        assert!((find_sum(&s1)[[0, 0]] - 1.0).abs() < 1e-6);

        let mut s2 = vec![DecodedSlice {
            om_name: "precipitation",
            leadtime_h: 2,
            data: array![[0.5_f32, 1.0], [1.0, 1.0]],
        }];
        accumulate_and_inject(&mut s2, &mut acc, 2);
        assert!((find_sum(&s2)[[0, 0]] - 1.5).abs() < 1e-6);
        assert!((find_sum(&s2)[[1, 1]] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn leadtime_zero_without_precip_stays_zero() {
        let mut acc = Array2::<f32>::zeros((2, 2));
        let mut s0 = vec![DecodedSlice {
            om_name: "temperature_2m",
            leadtime_h: 0,
            data: array![[20.0_f32, 20.0], [20.0, 20.0]],
        }];
        accumulate_and_inject(&mut s0, &mut acc, 0);
        assert!(find_sum(&s0).iter().all(|&v| v == 0.0));
        // La slice dérivée est bien ajoutée (la variable existe à H0).
        assert!(s0.iter().any(|s| s.om_name == DERIVED_PRECIP_SUM));
    }
}
```

- [ ] **Step 2: Déclarer le module**

Modify `crates/arome-om-forecast/src/lib.rs` — ajouter (après `pub mod variables;` ou dans l'ordre du fichier) :

```rust
pub mod cumul;
```

- [ ] **Step 3: Lancer les tests, vérifier qu'ils passent**

Run: `cargo test -p arome-om-forecast cumul`
Expected: PASS — `accumulates_precip_across_leadtimes`, `leadtime_zero_without_precip_stays_zero`.

- [ ] **Step 4: Commit**

```bash
git add crates/arome-om-forecast/src/cumul.rs crates/arome-om-forecast/src/lib.rs
git commit -m "feat(arome-om): add cumul helper injecting precipitation_sum"
```

---

## Task 3 : Restructurer le flux `main.rs` (décodage parallèle → écriture séquentielle)

**Files:**
- Modify: `crates/arome-om-forecast/src/main.rs:145-221` (bloc stream + post-loop) et `:228-277` (`process_leadtime`)

> Pas de nouveau test unitaire ici (flux async dépendant de l'API MF + Python). La validation = build + clippy + `cargo test --workspace` vert (Task 1/2 couvrent la logique), puis run local (Task 5).

- [ ] **Step 1: Transformer `process_leadtime` en `decode_leadtime` (retire l'écriture)**

Modify `crates/arome-om-forecast/src/main.rs` — remplacer la signature et la fin de la fonction (actuellement lignes ~228-277). La fonction garde tout le download+decode mais **ne fait plus l'écriture** et retourne les slices :

```rust
/// Décode un leadtime complet : télécharge tous les packages, décode, et
/// retourne les slices. N'écrit PAS l'OMfile (l'écriture est séquentielle dans
/// le consommateur, pour entretenir l'accumulateur de cumul).
///
/// Retourne un vecteur vide si 0 slices décodées (skip).
#[expect(clippy::too_many_arguments, reason = "pipeline context struct not yet introduced")]
async fn decode_leadtime(
    mf: &AromeOmClient,
    territory: AromeOmTerritory,
    packages: &[Package],
    leadtime: u32,
    run: DateTime<Utc>,
    grid: &ReunionGrid,
    work_dir: &std::path::Path,
    script_path: &std::path::Path,
) -> Result<Vec<DecodedSlice>> {
    let run_dir = work_dir.join(format!("{}Z", run.format("%Y%m%dT%H%M")));
    std::fs::create_dir_all(&run_dir)?;

    let mut all_slices: Vec<DecodedSlice> = Vec::new();
    for pkg in packages {
        let grib_path = run_dir.join(format!("{pkg}_{leadtime:03}h.grib2"));
        let bytes = mf
            .fetch_package(territory, pkg.as_api_id(), run, leadtime)
            .await
            .with_context(|| format!("fetch {pkg} leadtime={leadtime}"))?;
        std::fs::write(&grib_path, &bytes)
            .with_context(|| format!("write {grib_path:?}"))?;

        let nc_dir = run_dir.join(format!("nc_{pkg}_{leadtime:03}h"));
        let pkg_id = pkg.as_api_id();
        let vars_of_interest: Vec<&VariableEntry> = variables_for_package(pkg_id).collect();
        let slices = grib_decoder::decode(
            &grib_path,
            &nc_dir,
            &vars_of_interest,
            (grid.ny(), grid.nx()),
            script_path,
        )
        .await
        .with_context(|| format!("decode {pkg} leadtime={leadtime}"))?;
        all_slices.extend(slices);
    }

    Ok(all_slices)
}
```

(L'ancien appel `write_and_upload_timestep(...)` et le `Ok(true)/Ok(false)` sont supprimés ; `write_and_upload_timestep` reste inchangée, appelée désormais depuis le consommateur.)

- [ ] **Step 2: Remplacer le bloc stream `buffer_unordered` + post-loop par un consommateur séquentiel ordonné**

Modify `crates/arome-om-forecast/src/main.rs` — remplacer le bloc actuel (du `// Counters partagés` ligne ~145 jusqu'à la fin du `if let Some(r2)` métadonnées/GC, ligne ~216) par :

```rust
    let run_dir = work_dir.join(format!("{}Z", run.format("%Y%m%dT%H%M")));
    std::fs::create_dir_all(&run_dir).context("creating run_dir")?;

    // Décodage parallèle livré DANS L'ORDRE croissant des leadtimes (`buffered`),
    // condition nécessaire pour entretenir l'accumulateur de cumul séquentiel.
    let decoded = stream::iter(leadtimes.into_iter().map(|leadtime| {
        let mf = mf.clone();
        let work_dir = work_dir.clone();
        let script_path = script_path.clone();
        let packages = packages.clone();
        async move {
            let res =
                decode_leadtime(&mf, territory, &packages, leadtime, run, &ReunionGrid, &work_dir, &script_path)
                    .await;
            (leadtime, res)
        }
    }))
    .buffered(args.concurrency);
    tokio::pin!(decoded);

    // Accumulateur de cumul : zéros au départ, réutilisé d'un leadtime à l'autre.
    let mut acc = ndarray::Array2::<f32>::zeros((grid.ny(), grid.nx()));
    let mut written = 0u32;
    let mut failures = 0u32;

    while let Some((leadtime, res)) = decoded.next().await {
        match res {
            Ok(slices) if slices.is_empty() => {
                tracing::warn!(leadtime, "leadtime skipped (0 slices)");
            }
            Ok(mut slices) => {
                arome_om_forecast::cumul::accumulate_and_inject(&mut slices, &mut acc, leadtime);
                match write_and_upload_timestep(
                    slices,
                    run,
                    leadtime,
                    &run_dir,
                    r2.as_deref(),
                    &r2_prefix,
                    &grid,
                )
                .await
                {
                    Ok(()) => {
                        written += 1;
                        tracing::info!(leadtime, "leadtime OK");
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::error!(leadtime, error = %e, "write/upload FAILED");
                    }
                }
            }
            Err(e) => {
                failures += 1;
                tracing::error!(leadtime, error = %e, "leadtime FAILED");
                if matches!(e.downcast_ref::<MeteoFranceError>(), Some(MeteoFranceError::Auth(_))) {
                    tracing::error!("auth error — aborting");
                    std::process::exit(2);
                }
            }
        }
    }

    tracing::info!(written, failures, "all leadtimes done");

    // Metadata + GC seulement si au moins un fichier a été écrit et qu'on uploade.
    if let Some(r2) = r2.as_deref() {
        if written > 0 {
            let pkg_ids: std::collections::HashSet<&'static str> =
                packages.iter().map(|p| p.as_api_id()).collect();
            let mut var_names: Vec<&'static str> = VARIABLES
                .iter()
                .filter(|v| pkg_ids.contains(v.package))
                .map(|v| v.om_name)
                .collect();
            // Annonce la variable dérivée dès que la précip est présente.
            if var_names.contains(&arome_om_forecast::cumul::PRECIP_OM_NAME) {
                var_names.push(arome_om_forecast::cumul::DERIVED_PRECIP_SUM);
            }
            if let Err(e) = update_metadata(r2, run, &var_names).await {
                tracing::error!(error = %e, "metadata update failed");
            }
            if let Err(e) = gc_old_runs(r2, &r2_prefix, run, args.keep_runs_back).await {
                tracing::error!(error = %e, "GC failed");
            }
        }
    }
```

Notes pour l'implémenteur :
- Le `counters` (Arc<Mutex>) et `tokio::sync::Mutex` ne sont plus utilisés → supprimer la ligne `let counters = ...` et l'import `tokio::sync::Mutex` s'il devient inutilisé.
- `r2` reste `Option<Arc<R2Client>>` ; `r2.as_deref()` donne `Option<&R2Client>` (Arc déréférence vers `R2Client`).
- `territory` (enum) et `&ReunionGrid` (zero-sized) sont `Copy`/triviaux à passer dans les closures.

- [ ] **Step 3: Vérifier compilation, clippy et tests workspace**

Run:
```bash
cargo build -p arome-om-forecast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace
```
Expected: build OK, clippy clean, tous les tests (70 désormais : 68 + 2 nouveaux modules) verts.

- [ ] **Step 4: Commit**

```bash
git add crates/arome-om-forecast/src/main.rs
git commit -m "refactor(arome-om): ordered decode+sequential write to emit precipitation_sum"
```

---

## Task 4 : Documentation (`README.md` + `CLAUDE.md`)

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Mettre à jour `CLAUDE.md`**

Dans la section *Architecture du workspace* (description d'`arome-om-forecast`) et *Pièges qui touchent le code*, ajouter (texte à intégrer au bon endroit, pas en bloc isolé) :

- Description : « …écriture d'**un OMfile multi-variables par timestep**, **incluant la variable dérivée `precipitation_sum`** (cumul de précip depuis le début du run, NaN propagé) … ».
- Nouveau piège :

```markdown
- **`precipitation_sum` (cumul AROME-OM)** : variable *dérivée* (pas dans le registry `VARIABLES`), calculée par accumulation séquentielle de `precipitation` depuis H0 du run via `pipeline_core::accumulation::accumulate_into`. Pour entretenir l'accumulateur, le flux décode les leadtimes en parallèle mais les **écrit séquentiellement dans l'ordre croissant** (`buffered`, pas `buffer_unordered`). Sémantique : cumul **relatif au run** (pas calendaire), H0 = 0 partout, NaN propagé (une heure trouée tue le pixel pour la suite). Ne pas réintroduire `buffer_unordered` sur l'écriture — ça casserait le cumul.
```

- Module `core` : ajouter `accumulation — accumulate_into (cumul roulant NaN-aware)` à la liste des modules de `crates/core`.

- [ ] **Step 2: Mettre à jour `README.md`**

Ajouter `precipitation_sum` à la liste des variables AROME-OM publiées (là où les 11 variables surface sont décrites), avec une phrase : « + `precipitation_sum` (dérivée) : cumul de précipitation depuis le début du run, croissant à chaque échéance ».

- [ ] **Step 3: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "docs: document precipitation_sum derived variable + ordered write"
```

---

## Task 5 : Vérification locale (run réel, sans upload)

**Files:** aucun (validation manuelle).

- [ ] **Step 1: Lancer le pipeline en local sur quelques leadtimes, sans upload**

Pré-requis : `source venv/bin/activate`, `MF_APPLICATION_ID` exporté, `libeccodes0`/`libeccodes-tools` installés.

Run (horizon court pour itérer vite) :
```bash
cargo run --release -p arome-om-forecast -- \
  --territory reunion --horizon-h 6 --packages SP1,SP2 \
  --work-dir work_arome --skip-upload \
  --script-path scripts/decode_arome_om_grib.py
```
Expected: logs `leadtime OK` pour 0..=6, `all leadtimes done` avec `written≈7 failures=0`.

- [ ] **Step 2: Vérifier que `precipitation_sum` est dans les OMfiles et croît**

Inspecter les OMfiles produits sous `work_arome/<run>Z/*.om`. Vérifier que :
- chaque fichier contient un array nommé `precipitation_sum` ;
- à H0 il vaut 0 partout ;
- la valeur (sur un pixel non-NaN) à H+6 ≥ celle à H+1 (cumul croissant).

(Outil : un petit script Python `omfiles`/`xarray-omfiles` ou le lecteur `read_spatial_omfile` du worker en mode debug. Au minimum, vérifier la présence du child array via le lecteur OMfile du projet.)

- [ ] **Step 3: (suivi, repo voisin `maps/`) exposer la variable**

Hors de ce repo — à planifier séparément dans `../maps` : ajouter `precipitation_sum` aux `variableOptions` (override local), son entrée metadata et une échelle de couleurs cumul, puis tester l'animation. Pour AROME-OM, **aucun routing `_sum_` worker** : c'est un array normal du domaine `arome_om_reunion`. Cette étape n'est pas couverte par ce plan (repo distinct, conventions Svelte/Tailwind propres).

---

## Self-Review

- **Spec coverage** : brique `core` (§Archi.1 → Task 1) ✔ ; helper d'injection (§Archi.2 → Task 2) ✔ ; restructuration decode/write parallèle→séquentiel ordonné (§Archi.2 → Task 3) ✔ ; H0=0 (testé Task 2) ✔ ; NaN propagé (testé Task 1) ✔ ; nom `precipitation_sum` (Task 2 const) ✔ ; métadonnées (§Archi.3 → Task 3 step 2) ✔ ; docs (§Doc → Task 4) ✔ ; maps + test local (§Archi.4/Tests → Task 5) ✔ (maps marqué hors-scope repo).
- **Placeholders** : aucun — tout le code est fourni.
- **Type consistency** : `DecodedSlice { om_name: &'static str, leadtime_h: u32, data: Array2<f32> }` (champs publics) cohérent dans Tasks 2 & 3 ; `accumulate_into(&mut Array2<f32>, &Array2<f32>)` et `accumulate_and_inject(&mut Vec<DecodedSlice>, &mut Array2<f32>, u32)` identiques entre définition et appels ; consts `PRECIP_OM_NAME`/`DERIVED_PRECIP_SUM` utilisées telles quelles dans main.rs.
