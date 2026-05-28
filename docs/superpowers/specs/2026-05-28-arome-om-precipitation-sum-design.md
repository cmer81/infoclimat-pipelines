# Design — Variable `precipitation_sum` AROME-OM (cumul depuis le début du run)

**Date** : 2026-05-28
**Statut** : proposé
**Pipeline concerné** : `arome-om-forecast`
**Crate touché** : `pipeline-core` (brique d'accumulation) + `arome-om-forecast` (flux)

## Contexte & motivation

Les cumuls de précipitation sont aujourd'hui calculés **on-the-fly** par le service
`infoclimat-om-worker` (somme de N OMfiles horaires Open-Meteo à la demande). Deux
limites motivent ce changement, côté AROME-OM uniquement :

1. **Intégration client** — les variables cumul ne sont pas des variables first-class :
   elles ne figurent pas dans le dropdown `maps/`, et le routing client est bricolé
   (`getOMUrl()` détecte `^(.+)_sum_(\d+)h$` et un `resolveRequest` custom parse
   `/v1/sum/{domain}/…`).
2. **Robustesse / correctness** — le worker ne valide pas le `time_interval`, suppose un
   pas horaire, et dépend de la rétention 7 j d'Open-Meteo.

AROME-OM est **notre propre donnée** : la pipeline `arome-om-forecast` publie déjà
`precipitation` horaire dans des OMfiles multi-variables. Ajouter le cumul comme **un
array de plus dans ces OMfiles** le rend first-class sans nouvelle infrastructure ni
routing custom. Le client lit `reader.getChildByName("precipitation_sum")` comme
n'importe quelle autre variable du domaine `arome_om_reunion`.

**Hors scope** : les modèles métropole (ARPEGE Europe, AROME France, AROME France HD)
restent servis par le worker. La brique d'accumulation est placée dans `core` pour être
réutilisable si on généralise plus tard (approche B du brainstorm), mais aucune ingestion
métropole n'est construite ici.

## Sémantique du produit

Une nouvelle variable dérivée, **`precipitation_sum`**, présente dans chaque OMfile de
leadtime : le cumul de précipitation depuis H0 du run jusqu'au leadtime courant.

| Échéance | `precipitation_sum` = |
|----------|------------------------|
| H0       | 0 partout (rien n'est encore tombé ; `tp` n'existe pas à leadtime 0) |
| H+1      | pluie tombée entre H0 et H+1 |
| H+6      | cumul H0 → H+6 |
| H+24     | cumul H0 → H+24 |
| H+48     | **cumul total du run complet** |

- **Monotone croissante** au fil des leadtimes (à NaN près) → animation « façon Météociel ».
- **Cumul depuis le départ du run**, pas calendaire. Un run 06 UTC donne le total sur
  06 UTC J → 06 UTC J+2. (Un cumul calendaire « depuis 00 UTC » serait une variable
  distincte, non couverte ici, mais réalisable plus tard avec la même brique.)
- **H0 inclus** : la variable existe dans l'OMfile H0 et vaut 0 partout, pour cohérence
  (toutes les échéances exposent la variable).

### Décisions tranchées

- **(a) NaN propagé** : un pixel manquant à l'heure N rend le cumul NaN pour ce pixel à
  partir de N. Cohérent avec la philosophie du projet (« on ne ment pas sur la donnée
  manquante », cf. `subtract_with_nan`, NaN propagés volontairement) et avec le worker.
  *Tradeoff assumé* : une heure trouée « tue » le pixel pour la suite du run.
- **(b) Nom `precipitation_sum`** : pas de suffixe `_Nh` (ce n'est pas une fenêtre
  glissante figée, c'est le cumul de l'échéance depuis le run).

## Architecture

### 1. Brique réutilisable — `pipeline-core`

Nouvelle fonction pure, sans I/O, testée, à côté de `anomaly::subtract_with_nan` :

```rust
/// Accumule `hour` dans `acc` (addition pixel-à-pixel, NaN propagé).
/// `acc` est l'accumulateur courant ; après l'appel il contient le cumul
/// incluant `hour`. Une fois NaN, un pixel reste NaN pour les heures suivantes.
pub fn accumulate_into(acc: &mut Array2<f32>, hour: &Array2<f32>);
```

Localisation : nouveau module `accumulation` dans `pipeline-core` (ou ajout dans
`anomaly` si on préfère regrouper ; recommandé : module dédié `accumulation` pour la
clarté du domaine).

Sémantique pixel :

| `acc[p]` | `hour[p]` | résultat |
|----------|-----------|----------|
| x        | y         | x + y    |
| NaN      | y         | NaN      |
| x        | NaN       | NaN      |
| NaN      | NaN       | NaN      |

### 2. Restructuration du flux `arome-om-forecast`

**Problème** : aujourd'hui les leadtimes sont décodés **et écrits** en parallèle
(`buffer_unordered`, chaque `process_leadtime` écrit son OMfile isolément). Un cumul
roulant H0→N exige toutes les heures 0..N dans l'ordre. Il faut **découpler le décodage
(parallèle) de l'écriture (séquentielle, ordonnée par leadtime)**.

Nouveau découpage :

- **Étage de décodage (parallèle)** : `decode_leadtime(leadtime) -> (leadtime, Vec<DecodedSlice>)`
  — download + decode uniquement, **plus d'écriture dedans**. Le stream passe de
  `buffer_unordered` à **`buffered(concurrency)`** pour livrer les leadtimes dans l'ordre
  croissant.
- **Consommateur séquentiel** : maintient `acc: Array2<f32>` initialisé à zéro
  (shape `(ny, nx)` de la grille Réunion). Pour chaque leadtime dans l'ordre :
  1. trouver la slice `precipitation` (si présente — absente à H0) ;
  2. `accumulate_into(&mut acc, &precip)` si présente ;
  3. pousser une slice dérivée `precipitation_sum` = `acc.clone()` ;
  4. écrire + uploader l'OMfile multi-var (toutes les vars du leadtime + `precipitation_sum`).

`write_and_upload_timestep` est appelé depuis le consommateur séquentiel au lieu de
l'intérieur de la tâche parallèle.

**Budget mémoire** : le streaming ordonné garde au plus `concurrency` leadtimes décodés
en vol (~11 vars × 5 Mo ≈ 55 Mo chacun) + l'accumulateur (5 Mo). À `concurrency=4`,
~225 Mo de pic. OK pour un runner CI (7 Go).

**Comportement à H0** : `tp`/`precipitation` absent → étape 2 sautée → `acc` reste à 0 →
`precipitation_sum` = 0 partout écrit dans l'OMfile H0. ✔

**Gestion d'erreur / leadtime trou** : si un leadtime échoue (0 slices, 404 packages),
le flux actuel le skip. Décision : **un leadtime sauté ne fait pas avancer `acc`** (on
n'ajoute rien) mais les leadtimes suivants restent cohérents en cumul relatif. À
documenter comme limite (un trou au milieu sous-estime le cumul aval). *Alternative
écartée* : marquer tout NaN après un trou — trop destructif pour un skip transitoire.

### 3. Métadonnées

`precipitation_sum` est une variable **dérivée**, pas un mapping GRIB → elle ne va pas
dans le registry `VARIABLES`. Elle est exposée via une constante dédiée
(ex. `pub const DERIVED_PRECIP_SUM: &str = "precipitation_sum";`) et **ajoutée à la liste
`var_names`** passée à `update_metadata` (`main.rs:204`) pour que le `meta.json` du domaine
`arome_om_reunion` l'annonce.

### 4. Côté `maps/` (test local)

- Ajouter `precipitation_sum` aux `variableOptions` (override local, cf. README maps) +
  son entrée metadata + une échelle de couleurs cumul.
- Pour AROME-OM, **plus besoin** du routing `_sum_` vers le worker : c'est un array normal
  du domaine `arome_om_reunion`, lu par `getChildByName`. Le worker reste intact pour les
  autres domaines.

## Tests

- **Unitaires `core`** (`accumulation`) :
  - addition simple sur 2-3 étapes (vérifie le cumul croissant) ;
  - propagation NaN (un NaN à l'étape N → NaN persistant) ;
  - accumulateur initial à 0 inchangé par une étape vide / cas H0.
- **`arome-om-forecast`** : ajuster les tests de flux impactés par la restructuration
  decode/write ; vérifier qu'un OMfile produit contient bien `precipitation_sum` et que sa
  valeur à la dernière échéance = somme des `precipitation` horaires.
- **Filet workspace** : les 68 tests existants restent verts (`cargo test --workspace`).

## Documentation à mettre à jour (même PR)

- `README.md` + `CLAUDE.md` : nouvelle variable dérivée `precipitation_sum` (sémantique
  cumul-depuis-run, NaN propagé), restructuration decode/write (`buffered` ordonné),
  brique `core::accumulation`. Ajout au « layout R2 / format OMfile » (un array de plus).

## Limites connues (à documenter)

- Cumul **relatif au run**, pas calendaire.
- Un leadtime manquant au milieu sous-estime le cumul des échéances suivantes (on n'ajoute
  pas ce qu'on n'a pas).
- NaN propagé : une heure trouée tue le pixel pour le reste du run.
