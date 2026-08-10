# Reader

[English](README.md) | **Français**

Reader est un lecteur de flux RSS/Atom écrit en Rust. Son objectif est de réunir
les abonnements Medium, Substack et les autres flux compatibles dans une même
chronologie, puis de rendre les articles disponibles hors ligne.

Le projet comprend un programme en ligne de commande et une interface Tauri 2
pour Linux. Les deux utilisent le même cœur Rust et le même schéma SQLite. Une
adaptation Android est prévue dans une étape ultérieure.

## Fonctionnalités actuelles

- chargement de plusieurs flux depuis `feeds.toml` ;
- prise en charge de Medium, Substack et des autres flux RSS/Atom ;
- téléchargement asynchrone avec `reqwest` et Tokio ;
- nettoyage du HTML reçu avant son stockage ;
- déduplication des articles avec un identifiant propre à chaque flux ;
- stockage local dans SQLite avec migrations automatiques ;
- désactivation d'un abonnement sans perte d'historique ou suppression
  définitive avec ses articles ;
- états locaux « lu » et « favori » dans le cœur Rust ;
- affichage des articles du plus récent au plus ancien ;
- lecture du cache même lorsque certains flux sont indisponibles ;
- commandes distinctes pour rafraîchir, lister, lire et modifier les états
  locaux des articles ;
- interface Linux à deux panneaux avec gestion des abonnements ;
- distinction entre contenu complet, extrait et contenu absent ;
- ouverture sécurisée de l'article original pour les extraits.

## Prérequis sous Ubuntu

Le projet nécessite :

- une installation récente de Rust et Cargo, de préférence avec
  [rustup](https://rustup.rs/) ;
- un compilateur C pour construire la copie embarquée de SQLite ;
- Git pour récupérer le dépôt.

Les outils système du CLI peuvent être installés avec :

```bash
sudo apt update
sudo apt install build-essential git
```

Pour compiler l'application Tauri sous Ubuntu, installer également Node.js,
`pkg-config`, WebKitGTK et les bibliothèques recommandées par Tauri :

```bash
sudo apt update
sudo apt install pkg-config libwebkit2gtk-4.1-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev curl wget file
```

Le frontend a été validé avec Node.js 24 et npm 11. Une version LTS récente de
Node.js convient également.

SQLite est compilé avec l'application. Il n'est pas nécessaire d'installer un
serveur SQLite ni les bibliothèques de développement du système.

Le programme `sqlite3` est toutefois pratique pour inspecter ou sauvegarder la
base manuellement :

```bash
sudo apt install sqlite3
```

## Installation du projet

Depuis une copie locale du dépôt :

```bash
cargo build
```

Cargo télécharge les crates déclarées dans `Cargo.toml` et produit le binaire de
développement dans `target/debug/reader`.

Pour une compilation optimisée :

```bash
cargo build --release
```

Le binaire se trouve alors dans `target/release/reader`.

Installer ensuite les dépendances du frontend :

```bash
cd app
npm install
```

Le fichier `app/package-lock.json` fixe les versions résolues et doit rester
committé.

## Utiliser l'application Linux

Depuis `app/`, lancer l'application de développement :

```bash
npm run tauri dev
```

Au premier lancement, Reader crée automatiquement `reader.db` dans le
répertoire de données du bundle `io.github.r0m1-b.reader` — généralement
`~/.local/share/io.github.r0m1-b.reader/` sous Ubuntu. Cette base est distincte de
la base `reader.db` du CLI.

L'application affiche immédiatement son cache et ne contacte jamais le réseau
au démarrage. Utiliser « Abonnements » pour ajouter une URL RSS/Atom, corriger
si nécessaire la plateforme détectée, désactiver un flux ou le supprimer.
Utiliser ensuite « Actualiser » pour télécharger les articles.

Désactiver un abonnement conserve son identifiant, ses articles, les favoris et
les états de lecture. Ajouter de nouveau la même URL réactive cet abonnement au
lieu de créer un nouvel historique.

Supprimer est une action définitive distincte. Après confirmation, l'application
efface dans une même transaction l'abonnement, tous ses articles ainsi que leurs
états lu et favori. Ajouter de nouveau cette URL crée alors un nouvel abonnement
sans restaurer l'ancien historique.

Créer les paquets Linux optimisés :

```bash
cd app
npm run tauri build -- --bundles deb,appimage
```

Les paquets sont produits sous `target/release/bundle/deb/` et
`target/release/bundle/appimage/`.

## Configurer les abonnements

Créer un fichier `feeds.toml` à la racine du projet :

```toml
[[feeds]]
id = "mon-substack"
platform = "substack"
url = "https://exemple.substack.com/feed"

[[feeds]]
id = "mon-medium"
platform = "medium"
url = "https://medium.com/feed/@exemple"

[[feeds]]
id = "autre-blog"
platform = "other"
url = "https://example.org/feed.xml"
```

Règles de configuration :

- chaque `id` doit être non vide et unique ;
- `platform` accepte `medium`, `substack` ou `other`, en minuscules ;
- `url` doit désigner directement un flux RSS ou Atom public ;
- deux abonnements actifs ne peuvent pas utiliser exactement la même URL.

`feeds.toml` est volontairement ignoré par Git : il représente la configuration
personnelle du développeur.

## Utiliser Reader

Afficher l'aide et les commandes disponibles :

```bash
cargo run -- --help
```

Rafraîchir les abonnements :

```bash
cargo run -- refresh
```

La commande `refresh` :

1. lit `feeds.toml` ;
2. ouvre ou crée `reader.db` ;
3. applique les migrations SQLite manquantes ;
4. importe la liste courante des abonnements ;
5. télécharge les flux ;
6. insère ou met à jour les articles ;
7. affiche le nombre d'articles reçus, ajoutés et mis à jour.

Une erreur sur un flux est écrite sur la sortie d'erreur, mais elle n'efface pas
les articles déjà enregistrés. Les autres flux continuent d'être traités.

Lister les articles stockés, sans charger `feeds.toml` et sans réseau :

```bash
cargo run -- list
```

La liste affiche un numéro à partir de 1 et l'identifiant stable de chaque
article. Les commandes suivantes acceptent indifféremment l'un ou l'autre :

```bash
cargo run -- show 1
cargo run -- show "mon-substack::identifiant-editeur"

cargo run -- mark-read 1
cargo run -- mark-unread 1
cargo run -- favorite 1
cargo run -- unfavorite 1
```

`show` charge uniquement l'article sélectionné, convertit son HTML en texte
lisible dans le terminal, affiche son URL originale et le marque automatiquement
comme lu.

Un numéro correspond à la position actuelle dans la chronologie et peut changer
après un rafraîchissement. Pour les scripts, préférer l'identifiant stable.

Les chemins peuvent être remplacés pour une commande :

```bash
cargo run -- \
  --config /chemin/vers/feeds.toml \
  --database /chemin/vers/reader.db \
  refresh
```

`--config` n'est consulté que par `refresh`. `list`, `show` et les commandes
d'état sont entièrement hors ligne.

Codes de sortie :

- `0` : commande réussie ;
- `1` : erreur fatale de configuration, SQLite, sélection ou rendu ;
- `2` : rafraîchissement partiellement réussi, avec au moins un flux en erreur.

Le CLI actuel utilise des chemins déterminés à la compilation et ancrés à la
racine du projet. Il s'agit d'un comportement de développement, pas encore d'une
installation système portable.

## Base de données SQLite

### Installation et création

Il n'existe aucune étape d'installation séparée pour la base. Au premier
`cargo run -- refresh` ou `cargo run -- list`, SQLx crée automatiquement à la
racine du projet :

```text
reader.db
```

Les migrations présentes dans `migrations/` sont intégrées au binaire puis
appliquées à l'ouverture. La base contient actuellement :

- `feeds` : abonnements, plateforme, URL et état actif ;
- `articles` : contenu distant, type de contenu, relation au flux, état lu et
  favori ;
- `_sqlx_migrations` : migrations déjà appliquées par SQLx.

SQLite peut aussi créer temporairement `reader.db-wal` et `reader.db-shm`. Ces
fichiers, comme la base principale, sont ignorés par Git.

### Inspecter la base

Avec le client optionnel `sqlite3` :

```bash
sqlite3 reader.db ".tables"
sqlite3 reader.db ".schema feeds"
sqlite3 reader.db ".schema articles"
```

Quelques requêtes de diagnostic en lecture seule :

```bash
sqlite3 -header -column reader.db \
  "SELECT id, platform, is_active, url FROM feeds ORDER BY id;"

sqlite3 -header -column reader.db \
  "SELECT id, title, published_at, is_read, is_favorite FROM articles ORDER BY published_at DESC LIMIT 20;"
```

Il vaut mieux éviter de modifier manuellement ces tables : l'API Rust garantit
les contraintes et préserve les états locaux pendant les rafraîchissements.

### Sauvegarder la base

Après avoir arrêté Reader, utiliser la commande de sauvegarde SQLite :

```bash
sqlite3 reader.db ".backup 'reader-backup.db'"
```

Une simple copie est également possible lorsque Reader et tout client SQLite
sont fermés :

```bash
cp reader.db reader-backup.db
```

Le fichier contient les articles, les abonnements importés et les états lu et
favori. `feeds.toml` doit être sauvegardé séparément.

### Réinitialiser complètement la base

Attention : cette opération supprime l'historique, les favoris et les états de
lecture. Arrêter Reader, effectuer éventuellement une sauvegarde, puis exécuter
depuis la racine du projet :

```bash
rm -f reader.db reader.db-shm reader.db-wal
cargo run -- refresh
```

Le lancement suivant recrée une base vide, réapplique toutes les migrations,
importe `feeds.toml` et télécharge les articles encore présents dans les flux.
Les anciens articles qui ne figurent plus dans les flux ne pourront pas être
récupérés sans sauvegarde.

Pour réinitialiser la base de l'application Tauri, fermer Reader, sauvegarder si
nécessaire puis supprimer les trois fichiers SQLite de son répertoire AppData :

```bash
rm -f ~/.local/share/io.github.r0m1-b.reader/reader.db \
  ~/.local/share/io.github.r0m1-b.reader/reader.db-shm \
  ~/.local/share/io.github.r0m1-b.reader/reader.db-wal
```

Le prochain lancement recrée une base vide. Contrairement au CLI, l'application
ne réimporte pas automatiquement `feeds.toml` : les abonnements doivent être
ajoutés de nouveau dans l'interface.

### Retirer ou réactiver un abonnement

Cette section concerne le CLI. Dans l'application graphique, « Désactiver »
conserve l'historique tandis que « Supprimer » efface définitivement
l'abonnement et tous ses articles après confirmation.

Retirer une entrée de `feeds.toml`, puis exécuter `cargo run -- refresh`, marque
l'abonnement comme inactif. Ses articles, favoris et états de lecture restent
dans SQLite.

Remettre ultérieurement le même `id` dans `feeds.toml`, puis rafraîchir, réactive
l'abonnement et met à jour son URL et sa plateforme si nécessaire.

### Faire évoluer le schéma

Ne pas modifier une migration déjà appliquée. Pour faire évoluer la base :

1. ajouter un nouveau fichier SQL versionné dans `migrations/` ;
2. conserver les migrations précédentes ;
3. compiler et exécuter les tests ;
4. relancer Reader pour appliquer la nouvelle migration.

Le script `build.rs` demande à Cargo de reconstruire le binaire lorsque le
dossier des migrations change.

## Développement et qualité

Commandes usuelles :

```bash
cargo fmt
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings

cd app
npm run typecheck
npm test
npm run build
```

Les tests de collecte utilisent des contenus injectés et des fixtures locales.
Les tests SQLite utilisent des bases en mémoire ou des fichiers temporaires :
ils ne modifient pas `reader.db`.

Organisation principale :

```text
src/config.rs   lecture et validation de feeds.toml
src/cli.rs      arguments, commandes, rendu et codes de sortie
src/http.rs     téléchargement HTTP asynchrone
src/feed.rs     conversion RSS/Atom vers le modèle commun
src/service.rs  collecte, déduplication et tri
src/storage.rs  stockage SQLite et états locaux
src/refresh.rs  orchestration import → collecte → stockage
src/main.rs     point d'entrée du CLI
migrations/     évolution versionnée du schéma SQLite
app/src/         interface Vanilla TypeScript
app/src-tauri/   adaptation, commandes et configuration Tauri
```

## Limites actuelles

- les numéros affichés par `list` ne sont pas persistants entre deux
  chronologies, contrairement aux identifiants ;
- les contenus nécessitant une connexion ou un abonnement payant ne sont pas
  pris en charge ;
- l'interface n'est pas encore adaptée aux écrans mobiles ;
- `reader.db` du CLI se trouve encore dans le dépôt de développement.

L'étape suivante consiste à adapter l'interface à Android. Dans l'application
installée, SQLite est déjà la source de vérité et se trouve dans le répertoire
AppData propre au bundle `io.github.r0m1-b.reader`.

## Licence

Reader est distribué sous la [licence MIT](LICENSE).

## Dépannage rapide

### `feeds.toml` est introuvable

Créer le fichier à la racine du projet. Le chemin ne dépend pas du répertoire
depuis lequel le binaire est lancé. Ce fichier n'est nécessaire que pour
`refresh`.

### La configuration TOML est refusée

Vérifier les guillemets, les blocs `[[feeds]]`, les identifiants uniques et les
valeurs autorisées de `platform`.

### Un flux échoue

Vérifier que son URL retourne directement du RSS ou de l'Atom. Les articles déjà
stockés restent affichés même lorsque le serveur est indisponible.

### La base est verrouillée

Fermer les autres processus `reader` et les sessions `sqlite3` ouvertes sur
`reader.db`, puis réessayer. Ne supprimer les fichiers de la base qu'après avoir
arrêté ces processus.

### Une migration échoue

Conserver le message d'erreur, sauvegarder la base et vérifier l'ordre ainsi que
le contenu des fichiers dans `migrations/`. En développement seulement, une
réinitialisation complète permet de repartir d'un schéma vierge.

### Une erreur GLIBC mentionne `/snap/core20`

Un terminal intégré à une installation snap de VS Code peut injecter ses propres
variables GTK/GIO. Elles mélangent alors les bibliothèques du snap et celles du
système. Lancer Reader depuis un terminal Ubuntu normal. Pour un diagnostic
ponctuel depuis le terminal intégré, retirer notamment `GTK_PATH`,
`GIO_MODULE_DIR`, `GDK_PIXBUF_MODULE_FILE`, `GSETTINGS_SCHEMA_DIR` et `LOCPATH`
de l'environnement avant `npm run tauri dev`.
