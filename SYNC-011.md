# SYNC-011 — Compacter, réparer et observer la synchronisation

État : **en cours**.

Ce document détaille les lots restant à réaliser avant de pouvoir considérer
SYNC-011 comme terminée. Il complète `FEATURE_REQUESTS.md` sans remplacer les
critères d'acceptation de la feature.

## Socle déjà disponible

- accusés de réception chiffrés et monotones par appareil ;
- registre distribué et authentifié des appareils actifs et révoqués ;
- calcul de frontière de compaction dérivé obligatoirement du registre actif ;
- checkpoints chiffrés v2 contenant un état compact et des frontières complètes ;
- lecture rétrocompatible des instantanés contigus v1 ;
- restauration d'une base neuve et reprise avec les segments suivants ;
- réparation d'un trou de segments depuis un checkpoint ;
- diagnostic JSON expurgé disponible avec `sync-diagnostic` ;
- vérification distante d'un checkpoint inchangé et republication automatique
  lorsqu'il a disparu ou a été corrompu ;
- compaction locale bornée activée uniquement après confirmation du checkpoint ;
- suppression WebDAV bornée aux segments locaux entièrement couverts et sûrs.

## Lot 8 — Compaction locale des événements

- [x] Ne déclencher la compaction que si le checkpoint courant est confirmé.
- [x] Calculer pour chaque journal la frontière autoritaire acquittée par tous
  les appareils actifs.
- [x] Ne jamais supprimer un événement postérieur à cette frontière.
- [x] Conserver les événements nécessaires au checkpoint compact :
  - créations d'abonnements encore nécessaires à leurs incarnations ;
  - gagnants des registres LWW ;
  - pierres tombales ;
  - dépendances encore en attente.
- [x] Supprimer transactionnellement uniquement les événements devenus
  redondants.
- [x] Borner le nombre de suppressions par cycle afin de ne pas bloquer l'UI.
- [x] Rendre l'opération idempotente et sûre après une interruption.
- [x] Ne pas modifier les projections, états lu/favori, abonnements ou articles.
- [x] Ajouter au rapport interne le nombre d'événements locaux compactés.

Critère de sortie : après compaction, le checkpoint produit avant et après
l'opération représente le même état et une base restaurée converge vers les
mêmes projections.

## Lot 9 — Suppression sûre des segments WebDAV

- [x] Étendre le contrat de transport avec une suppression idempotente et
  strictement limitée aux chemins de segments validés.
- [x] Autoriser un appareil à supprimer uniquement les segments de son propre
  journal.
- [x] Ne supprimer que les segments entièrement couverts par la frontière
  autoritaire et par un checkpoint distant vérifié.
- [x] Conserver les segments contenant au moins un événement postérieur à la
  frontière.
- [x] Borner les suppressions par cycle et limiter leur concurrence.
- [x] Considérer un fichier déjà absent comme un succès idempotent.
- [x] En cas d'erreur WebDAV, arrêter la suppression sans invalider les données
  locales ni le checkpoint.
- [x] Ne jamais faire échouer l'import des nouveaux événements parce qu'une
  suppression différée a échoué.
- [x] Ajouter aux diagnostics les compteurs de segments supprimés et différés.

Critère de sortie : aucun segment nécessaire à un appareil actif ou à une
restauration depuis le checkpoint courant ne peut être supprimé.

## Lot 10 — Reprise après interruption et corruption

- [x] Tester une interruption après publication du checkpoint mais avant la
  compaction locale.
- [x] Tester une interruption pendant la suppression locale.
- [x] Tester une interruption entre deux suppressions WebDAV.
- [x] Tester un checkpoint distant supprimé ou corrompu juste avant la
  compaction : aucune suppression ne doit avoir lieu avant sa réparation.
- [x] Tester une base SQLite locale perdue ou corrompue, recréée puis restaurée
  depuis WebDAV.
- [x] Tester un appareil très en retard après la disparition de tous les anciens
  segments couverts.
- [x] Tester la restauration depuis le checkpoint, puis l'application des
  segments plus récents.
- [x] Tester un appareil actif sans accusé : la frontière doit rester bloquée.
- [x] Tester qu'un appareil révoqué ne bloque plus la frontière et ne peut plus
  importer de nouvelles données.

Critère de sortie : chaque point d'interruption laisse soit l'ancien état
récupérable, soit le nouveau, sans état intermédiaire irréparable.

## Lot 11 — Versionnement et migrations de protocole

- [x] Conserver des fixtures chiffrées ou déterministes des formats v1 et v2.
- [x] Tester explicitement l'import d'un checkpoint v1 après mise à niveau.
- [x] Tester le rejet sans effet de bord d'une version future inconnue.
- [x] Documenter la règle de compatibilité des segments, checkpoints, accusés et
  registres d'appareils.
- [x] Définir comment une future migration réécrira un checkpoint sans supprimer
  le dernier format encore récupérable.

Critère de sortie : une mise à niveau ne rend pas les données distantes
existantes illisibles et une version inconnue ne modifie pas SQLite.

## Lot 12 — Diagnostic exportable dans l'application

Ce lot est souhaitable pour Linux et Android, mais il n'est pas un prérequis à
la sécurité de la compaction.

- [x] Exposer le diagnostic expurgé existant par une commande Tauri.
- [x] Permettre son enregistrement ou son partage depuis l'interface.
- [x] Ne jamais inclure clé, mot de passe, URL ou identifiant WebDAV, UUID ou nom
  d'appareil, URL/titre d'article ou contenu en cache.
- [x] Inclure les compteurs de checkpoints et de compaction utiles au support.
- [x] Tester l'export sur desktop et le contrat TypeScript sur mobile.

Critère de sortie : l'utilisateur peut transmettre un diagnostic exploitable
sans exposer ses secrets ni ses contenus.

## Lot 13 — Documentation opérationnelle

- [x] Documenter la sauvegarde et la restauration de SQLite.
- [x] Documenter la reconstruction d'un appareil depuis WebDAV.
- [x] Documenter la perte d'un appareil et sa révocation définitive.
- [x] Expliquer qu'un appareil réinstallé reçoit un nouvel UUID.
- [x] Documenter la perte de la clé de groupe et la perte du stockage WebDAV.
- [x] Décrire les métadonnées visibles par l'hébergeur WebDAV malgré le
  chiffrement.
- [x] Documenter les limites de taille, de nombre d'appareils et de rétention.
- [x] Documenter les commandes de diagnostic et de vérification utiles.

## Validation finale

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `npm run typecheck`
- [ ] `npm test`
- [ ] `npm run build`
- [ ] `git diff --check`
- [ ] test manuel d'une synchronisation et d'une restauration avec deux appareils ;
- [ ] vérifier qu'aucun test automatisé n'accède à Internet ;
- [ ] passer SYNC-011 à `terminée` dans `FEATURE_REQUESTS.md` ;
- [ ] mettre à jour les README bilingues et `codex_report.md`.

## Ordre recommandé

1. Lot 8 — compaction locale ;
2. Lot 9 — suppression WebDAV ;
3. Lot 10 — interruption et récupération ;
4. Lot 11 — compatibilité de protocole ;
5. Lot 13 — documentation et clôture ;
6. Lot 12 — export graphique, parallèlement ou juste après la clôture technique.

La règle de sécurité reste simple : en cas de doute sur le checkpoint, le
registre des appareils ou les accusés de réception, InkRiver conserve les
événements et les segments.
