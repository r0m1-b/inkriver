# Reader — feuille de route

## Objectif

Réunir dans une même application les articles provenant de flux Medium,
Substack et, à terme, d'autres sources RSS ou Atom.

Le cœur de l'application reste écrit en Rust et indépendant de l'interface afin
d'être réutilisé par :

- le programme en ligne de commande actuel ;
- une application Tauri sous Linux ;
- la future application Android.

## État actuel

- [x] Initialiser le projet Cargo.
- [x] Télécharger un flux avec `reqwest`.
- [x] Vérifier les statuts HTTP et propager les erreurs.
- [x] Analyser les documents RSS et Atom avec `feed-rs`.
- [x] Représenter les plateformes avec `Source` et `Platform`.
- [x] Convertir une entrée `feed-rs` vers le modèle commun `Article`.
- [x] Conserver le titre, l'auteur, la date, l'URL et le contenu d'un article.
- [x] Charger plusieurs abonnements depuis `feeds.toml`.
- [x] Réunir les articles de plusieurs flux dans un seul `Vec<Article>`.
- [x] Trier les articles du plus récent au plus ancien.
- [x] Ajouter des tests unitaires pour HTTP, la configuration et les modèles.
- [x] Ajouter deux flux de test hors ligne, Medium et Substack, contenant cinq
      articles chacun.

Le premier jalon est donc atteint : le programme sait charger plusieurs flux et
produire une chronologie commune sans dépendre du format propre à chaque
plateforme.

## Configuration actuelle

Les abonnements sont déclarés dans `feeds.toml` :

```toml
[[feeds]]
id = "example-substack"
platform = "substack"
url = "https://example.substack.com/feed"

[[feeds]]
id = "example-medium"
platform = "medium"
url = "https://medium.com/feed/@example"
```

Les dépendances actuellement utilisées sont :

- `reqwest` avec son client HTTP asynchrone ;
- `feed-rs` pour RSS et Atom ;
- `chrono` pour les dates ;
- `serde` et `toml` pour la configuration ;
- `anyhow` pour enrichir les erreurs ;
- `sha2` pour les empreintes d'identité de dernier recours ;
- `clap` pour les sous-commandes et options du CLI ;
- `html2text` pour rendre le contenu lisible dans le terminal ;
- `sqlx` avec Tokio, SQLite, Chrono et les migrations embarquées pour le
  stockage local.

Avec `reqwest` 0.13, la feature TLS explicite s'appelle `rustls`, et non plus
`rustls-tls`. La configuration actuelle active les fonctionnalités par défaut de
`reqwest`, qui utilisent déjà Rustls.

## Correctifs prioritaires issus de l'audit

- [x] **P2 — Stabiliser l'identité des articles sans GUID.** Définir et tester
      explicitement la stratégie GUID → URL canonique → empreinte stable, sans
      dépendre de l'identifiant synthétique de `feed-rs` qui peut changer avec
      le titre et créer des doublons persistants.
- [x] **P2 — Séparer la liste des résumés du chargement du contenu.** Ajouter
      une lecture SQLite légère pour la chronologie et une lecture du détail par
      identifiant, afin de ne pas charger le corps HTML de toute l'archive pour
      le seul affichage des résumés.
- [ ] **P3 — Compléter la validation de `feeds.toml`.** Rejeter dès le chargement
      les URL vides ou invalides, les schémas autres que HTTP(S) et les URL
      dupliquées, avec des erreurs de configuration explicites.
- [ ] **P3 — Améliorer la collecte HTTP de plusieurs flux.** Partager un client
      `reqwest` et télécharger les flux avec une concurrence bornée pour éviter
      que les latences des abonnements lents ou indisponibles s'additionnent.

## Prochaines étapes

### 5. Consolider le cœur Rust

- [x] Déplacer l'orchestration encore présente dans `main.rs` vers un module de
      bibliothèque, par exemple `reader.rs` ou `service.rs`.
- [x] Faire de `main.rs` un point d'entrée très court : charger la configuration,
      appeler le cœur puis afficher le résultat ou l'erreur.
- [x] Remplacer les messages de repli tels que `"No title"` et `"No content"`
      par une politique explicite adaptée à l'interface.
- [x] Définir une stratégie d'identification et de déduplication des articles à
      partir de l'identifiant du flux et, en repli, de l'URL.
- [x] Tester l'agrégation de deux flux et leur tri chronologique avec les
      fixtures hors ligne.
- [x] Nettoyer le HTML des articles avant tout affichage dans une WebView.
- [x] Conserver temporairement une sortie CLI permettant de contrôler la
      chronologie avant de construire l'interface.

Le téléchargement utilise désormais l'API asynchrone de `reqwest`. Le cœur peut
donc être appelé par Tauri sans bloquer le thread de l'interface.

### 6. Ajouter le stockage local SQLite

Décision de persistance pour l'application installée :

- SQLite sera la source de vérité des abonnements et des articles ;
- la base sera placée dans `AppData/io.github.r0m1-b.reader/reader.db`, en résolvant
  le répertoire avec `app.path().app_data_dir()` lors de l'ajout de Tauri ;
- le répertoire `AppData` devra être créé avant l'ouverture de la base ;
- `feeds.toml` restera le format de développement du CLI et un futur format
  d'import/export explicite ;
- pour le CLI, chaque rafraîchissement importe explicitement `feeds.toml` dans
  SQLite ; aucun fichier n'est surveillé et aucun export automatique n'est
  effectué ;
- les futurs réglages réellement configurables pourront être placés dans
  `AppConfig`, séparément de la base SQLite ;
- le bundle Tauri stable sera `io.github.r0m1-b.reader`.

- [x] Utiliser SQLx avec Tokio, SQLite et des migrations embarquées.
- [x] Créer une table `feeds` pour les abonnements actifs ou retirés.
- [x] Créer une table `articles` contenant au minimum : identifiant, titre,
      auteur, date, URL, contenu et source.
- [x] Ajouter les états locaux `is_read` et `is_favorite`.
- [x] Insérer ou mettre à jour les articles sans créer de doublons ni écraser
      les états locaux.
- [x] Charger les articles depuis SQLite dans l'ordre chronologique inverse,
      avec les articles sans date à la fin.
- [x] Tester les opérations avec des bases SQLite temporaires et en mémoire.

Le CLI stocke provisoirement sa base dans
`CARGO_MANIFEST_DIR/reader.db`. Un abonnement absent du prochain import devient
inactif, mais sa ligne, ses articles et leurs états locaux sont conservés. Le
CLI relit toujours SQLite après la collecte : le cache reste donc visible si le
réseau est indisponible.

À la fin de cette étape, l'application doit pouvoir être utilisée hors ligne
après un premier rafraîchissement.

### 7. Stabiliser le programme en ligne de commande

- [x] Ajouter une commande de rafraîchissement des flux.
- [x] Afficher les articles stockés, du plus récent au plus ancien.
- [x] Permettre de sélectionner un article et d'afficher son contenu ou son URL.
- [x] Présenter clairement les erreurs de configuration, de réseau et d'analyse.

Le CLI propose désormais `refresh`, `list`, `show`, `mark-read`, `mark-unread`,
`favorite` et `unfavorite`. Les commandes hors ligne acceptent un numéro de la
chronologie ou un identifiant stable. Un rafraîchissement partiel conserve les
succès, affiche les erreurs par flux et retourne le code de sortie `2`.

Cette étape valide l'API du cœur Rust avant son branchement à Tauri.

### 8. Créer l'interface Linux avec Tauri

- [x] Créer une application Tauri 2 avec une interface Vanilla TypeScript.
- [x] Réutiliser le cœur Rust existant comme dépendance de l'application Tauri.
- [x] Exposer des commandes Tauri pour :
  - lister les articles ;
  - charger le détail d'un article ;
  - rafraîchir les flux ;
  - marquer un article comme lu ou favori.
- [x] Afficher une liste d'articles avec titre, source, auteur et date.
- [x] Afficher l'article sélectionné dans un panneau de lecture.
- [x] Ajouter un bouton ouvrant l'article original lorsque le flux ne contient
      qu'un extrait.
- [x] Gérer les états de chargement, l'absence d'articles et les erreurs.
- [x] Gérer les abonnements SQLite dans l'interface : ajout, détection de la
      plateforme, activation et désactivation sans perte d'historique.
- [x] Installer les prérequis WebKitGTK sur la machine de développement, lancer
      l'application native et valider les paquets `.deb` et AppImage.

Sur ordinateur, l'interface utilise deux panneaux : liste à gauche et article à
droite. Le cache est affiché sans réseau au démarrage et l'actualisation reste
une action manuelle.

### 8.1. Traiter les retours du premier test utilisateur

- [ ] Préciser puis implémenter les demandes enregistrées dans
      [FEATURE_REQUESTS.md](FEATURE_REQUESTS.md) : suppression d'un abonnement,
      états lu/non lu, liens externes, détail des erreurs et conversion des URL
      Medium.
  - [x] FR-001 — supprimer un abonnement et ses articles dans une transaction ;

Ces améliorations doivent être évaluées avant de commencer l'étape Android afin
de stabiliser le comportement du lecteur sur ordinateur.

### 9. Ajouter Android

- [ ] Rendre l'interface responsive : liste puis écran de lecture sur mobile.
- [ ] Configurer le SDK Android, le NDK et les cibles Rust.
- [ ] Vérifier le stockage SQLite dans le répertoire privé de l'application.
- [ ] Tester le téléchargement en arrière-plan sans bloquer la WebView.
- [ ] Construire et installer un premier APK de développement.

### 10. Évolutions ultérieures

- [ ] Importer et exporter les abonnements au format OPML.
- [ ] Ajouter la recherche locale.
- [ ] Ajouter des filtres par source et par état de lecture.
- [ ] Synchroniser les abonnements et l'état de lecture entre Linux et Android.
- [ ] Étudier séparément les contenus payants ou nécessitant une authentification.

La synchronisation et les comptes privés restent volontairement hors du premier
périmètre : ils nécessitent un service distant ou une stratégie
d'authentification distincte du lecteur RSS local.

## Commandes de contrôle

Pendant le développement :

```bash
cargo fmt
cargo check
cargo test
cargo run
```

Les chapitres les plus utiles du
[Rust Book](https://doc.rust-lang.org/book/) restent les structures, les enums,
`Option` et `Result`, les collections, les traits, la gestion des erreurs et les
modules.
