# InkRiver

[English](README.md) | **Français**

InkRiver est un lecteur de flux RSS/Atom écrit en Rust. Son objectif est de réunir
les abonnements Medium, Substack et les autres flux compatibles dans une même
chronologie, puis de rendre les articles disponibles hors ligne.

Le projet comprend un programme en ligne de commande et une interface Tauri 2
pour Linux. Les deux utilisent le même cœur Rust et le même schéma SQLite. Une
première interface responsive Android est en cours de développement sur ce même
backend.

Consulter [CHANGELOG.md](CHANGELOG.md) pour l'historique des versions.

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
- chronologie et lecteur sur deux écrans distincts sur un affichage mobile ;
- sélection multiple par appui long dans la chronologie mobile, avec marquage
  groupé lu/non lu et archivage atomique ;
- indicateur de défilement mobile discret montrant la position et la longueur
  restante dans les articles longs ;
- distinction entre contenu fourni par le flux, extrait du Web, résumé et
  contenu absent ;
- archivage manuel et rétention automatique des articles devenus inutiles ;
- extraction sécurisée des pages complètes pour les articles incomplets des
  autres flux ;
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
développement dans `target/debug/inkriver`.

Pour une compilation optimisée :

```bash
cargo build --release
```

Le binaire se trouve alors dans `target/release/inkriver`.

Installer ensuite les dépendances du frontend :

```bash
cd app
npm install
```

Le fichier `app/package-lock.json` fixe les versions résolues et doit rester
committé.

## Préparer le développement Android

L'interface responsive et la gestion du bouton Retour Android sont
implémentées, mais la génération du projet natif nécessite l'outillage mobile
local. Installer Android Studio, puis utiliser son SDK Manager pour installer
une plateforme SDK Android, Platform-Tools, Build-Tools, Command-line Tools et
un NDK (side by side).

Sous Ubuntu, exposer le JDK fourni par Android Studio ainsi que le SDK et le
NDK. Remplacer ci-dessous la version du NDK par le répertoire installé :

```bash
export JAVA_HOME=/opt/android-studio/jbr
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/<version-installée>"
export PATH="$ANDROID_HOME/platform-tools:$PATH"
```

Installer les cibles Rust Android, initialiser une seule fois le projet natif,
puis le lancer sur un appareil connecté ou un émulateur :

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi \
  i686-linux-android x86_64-linux-android
cd app
npm run tauri android init
npm run tauri android dev
```

Vite lit `TAURI_DEV_HOST`, ce qui permet à l'appareil de joindre le serveur de
développement. Pour produire un APK de test après l'initialisation :

```bash
npm run tauri android build -- --apk
```

La WebView Android utilise les mêmes commandes Tauri, le même cœur Rust, les
mêmes migrations et la même couche SQLite que Linux. Sa base est créée dans le
répertoire AppData privé de l'application et n'est pas partagée avec la base
Linux.

## Utiliser l'application Linux

Depuis `app/`, lancer l'application de développement :

```bash
npm run tauri dev
```

Au premier lancement, InkRiver crée automatiquement `inkriver.db` dans le
répertoire de données du bundle `io.github.r0m1-b.inkriver` — généralement
`~/.local/share/io.github.r0m1-b.inkriver/` sous Ubuntu. Cette base est distincte
de la base `inkriver.db` du CLI.

L'application affiche immédiatement son cache et ne contacte jamais le réseau
au démarrage. La page **Abonnements** liste chaque flux actif ou inactif avec son
URL, son auteur, sa description, sa dernière publication, sa dernière
actualisation réussie et sa dernière erreur détaillée. Cet état est conservé
dans SQLite et reste visible après un redémarrage d'InkRiver. Les actions de
désactivation et de suppression se trouvent sur cette page ; **Ajouter un
abonnement** ouvre une fenêtre séparée contenant uniquement le formulaire
d'ajout. L'action **Actualiser** de l'en-tête met à jour tous les abonnements
actifs ; l'action circulaire de chaque carte active ne rafraîchit que ce flux.
Un abonnement nouvellement ajouté n'est jamais téléchargé automatiquement. Les
erreurs individuelles apparaissent brièvement dans une notification rouge et
restent consultables en détail dans la carte correspondante. Un flux désactivé
doit être réactivé avant son actualisation.

L'ouverture d'un article non lu le marque automatiquement comme lu. Le panneau
de lecture affiche son état courant et permet de le marquer explicitement comme
lu ou non lu ; la modification est enregistrée dans SQLite et immédiatement
répercutée dans la chronologie. Chaque ligne de la chronologie propose également
des boutons étoile et enveloppe toujours visibles pour changer les états favori
et lu sans ouvrir l'article.
Les badges de source associent leur libellé à la marque Medium ou Substack, ou
au logo mis en cache du site pour les autres flux RSS. Une icône RSS générique
reste affichée lorsqu'aucun logo exploitable n'est disponible. Les vecteurs de
marque proviennent de Simple Icons v16.21.0 ; Medium et Substack restent
propriétaires de leurs marques respectives.
Les onglets mutuellement exclusifs **Tous**, **Favoris** et **Non lus** au-dessus
de la chronologie affichent respectivement tous les articles, tous les favoris
quel que soit leur état de lecture, ou tous les articles non lus. Le filtrage
reste entièrement hors ligne et conserve le même panneau de lecture.

Le panneau de lecture propose aussi une icône d'archivage. Après une confirmation
obligatoire, l'article disparaît de toutes les listes et ne peut pas être restauré
depuis l'interface actuelle. InkRiver conserve une petite pierre tombale dans la
base afin qu'un rafraîchissement ultérieur ne recrée pas l'article, mais efface
son corps enregistré.

Au démarrage de l'application et après chaque rafraîchissement, InkRiver applique
une rétention fixe de 30 jours. L'actualisation d'un abonnement limite cette
maintenance, y compris l'extraction des pages d'articles, à cet abonnement ;
l'actualisation globale traite tous les flux actifs. Seuls les articles lus, non
favoris, possédant une date de publication et strictement plus vieux que 30 jours
sont archivés automatiquement. Les articles non lus, favoris, sans date ou vieux
d'exactement 30 jours sont conservés.

Les liens HTTP(S) contenus dans un article s'ouvrent dans le navigateur système
au lieu de naviguer dans InkRiver. Les liens relatifs sont résolus depuis l'URL
de l'article. Les liens vers une section du même article sont actuellement
ignorés.
Le panneau de lecture identifie toujours la source d'origine de l'article par
son domaine et permet de l'ouvrir dans le navigateur système. Lorsque le flux ne
contient qu'un extrait ou aucun contenu, le bouton plus visible **Lire
l'original** reste également disponible. Un état non interactif est affiché si
l'article ne possède aucune source HTTP(S) exploitable.

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

## Utiliser InkRiver

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
2. ouvre ou crée `inkriver.db` ;
3. applique les migrations SQLite manquantes ;
4. importe la liste courante des abonnements ;
5. télécharge les flux ;
6. insère ou met à jour les articles ;
7. applique la même rétention de 30 jours que l'application ;
8. tente d'extraire les pages complètes des articles éligibles hors Medium et
   Substack ;
9. affiche le nombre d'articles reçus, ajoutés, mis à jour, extraits et archivés
   automatiquement.

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
  --database /chemin/vers/inkriver.db \
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
inkriver.db
```

Les migrations présentes dans `migrations/` sont intégrées au binaire puis
appliquées à l'ouverture. La base contient actuellement :

- `feeds` : abonnements, plateforme, URL, état actif, métadonnées du flux et
  dernier succès ou échec d'actualisation ;
- `articles` : contenu distant, type de contenu, relation au flux, états lu et
  favori, ainsi que les pierres tombales d'archivage et leur motif ;
- `_sqlx_migrations` : migrations déjà appliquées par SQLx.

SQLite peut aussi créer temporairement `inkriver.db-wal` et `inkriver.db-shm`. Ces
fichiers, comme la base principale, sont ignorés par Git.

### Inspecter la base

Avec le client optionnel `sqlite3` :

```bash
sqlite3 inkriver.db ".tables"
sqlite3 inkriver.db ".schema feeds"
sqlite3 inkriver.db ".schema articles"
```

Quelques requêtes de diagnostic en lecture seule :

```bash
sqlite3 -header -column inkriver.db \
  "SELECT id, platform, is_active, url FROM feeds ORDER BY id;"

sqlite3 -header -column inkriver.db \
  "SELECT id, title, published_at, is_read, is_favorite FROM articles ORDER BY published_at DESC LIMIT 20;"
```

Il vaut mieux éviter de modifier manuellement ces tables : l'API Rust garantit
les contraintes et préserve les états locaux pendant les rafraîchissements.

Les lignes archivées conservent volontairement leur identifiant et leurs
métadonnées afin que les mêmes entrées distantes restent masquées lors des
rafraîchissements suivants. Leur contenu passe à `NULL`, mais SQLite ne réduit
pas immédiatement la taille du fichier et InkRiver n'exécute pas automatiquement
`VACUUM`.

### Sauvegarder la base

Après avoir arrêté InkRiver, utiliser la commande de sauvegarde SQLite :

```bash
sqlite3 inkriver.db ".backup 'inkriver-backup.db'"
```

Une simple copie est également possible lorsque InkRiver et tout client SQLite
sont fermés :

```bash
cp inkriver.db inkriver-backup.db
```

Le fichier contient les articles, les abonnements importés et les états lu et
favori. `feeds.toml` doit être sauvegardé séparément.

### Réinitialiser complètement la base

Attention : cette opération supprime l'historique, les favoris et les états de
lecture. Arrêter InkRiver, effectuer éventuellement une sauvegarde, puis exécuter
depuis la racine du projet :

```bash
rm -f inkriver.db inkriver.db-shm inkriver.db-wal
cargo run -- refresh
```

Le lancement suivant recrée une base vide, réapplique toutes les migrations,
importe `feeds.toml` et télécharge les articles encore présents dans les flux.
Les anciens articles qui ne figurent plus dans les flux ne pourront pas être
récupérés sans sauvegarde.

Pour réinitialiser la base de l'application Tauri, fermer InkRiver, sauvegarder si
nécessaire puis supprimer les trois fichiers SQLite de son répertoire AppData :

```bash
rm -f ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db \
  ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db-shm \
  ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db-wal
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
4. relancer InkRiver pour appliquer la nouvelle migration.

Le script `build.rs` demande à Cargo de reconstruire le binaire lorsque le
dossier des migrations change.

## Développement et qualité

### Flux de branches

- `dev` est la branche d'intégration utilisée pour le développement courant.
  Les changements ordinaires y sont commités et poussés.
- Une branche temporaire peut être créée depuis `dev` lorsqu'un changement
  mérite une revue isolée, puis fusionnée dans `dev`.
- `main` représente une version stable et publiable. Elle n'est mise à jour
  depuis `dev` que lorsqu'une release est explicitement figée, après réussite
  de toutes les commandes de validation.
- Les releases sont taguées sur `main` ; le développement courant n'est jamais
  commité directement sur cette branche.

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
ils ne modifient pas `inkriver.db`.

### Corpus local facultatif pour l'extraction

L'extracteur de contenu principal possède des fixtures synthétiques committées
et s'exécute donc avec la suite de tests ordinaire. Un corpus plus large de
pages tierces complètes peut être conservé localement sous `tests/pages/` ; ce
répertoire est ignoré par Git et n'est jamais lu par les tests ordinaires.

Chaque page est décrite par le fichier local `tests/pages/description.json`.
Exécuter son test de régression explicitement ignoré avec :

```bash
cargo test --test extraction_corpus -- --ignored
```

Seuls les documents HTML enregistrés sont nécessaires. Les répertoires de
ressources `*_files` créés par le navigateur ne sont pas lus par l'extracteur.

### Extraction automatique des pages d'articles

Pendant une actualisation manuelle, InkRiver peut compléter les articles des
flux `Other` dont l'entrée RSS ne contient qu'un extrait ou aucun corps. Medium,
Substack, les entrées RSS complètes et les articles archivés ne sont jamais
téléchargés ainsi. Une page acceptée est stockée avec
`content_kind = extracted` ; un échec conserve intact le contenu RSS de repli.

Les téléchargements sont limités à 20 tentatives par actualisation et quatre
requêtes simultanées, avec un délai maximal de 10 secondes, un corps limité à
2 Mio et cinq redirections HTTP(S) validées. Les destinations privées ou
locales sont rejetées. Après un échec, la page n'est retentée qu'au bout de sept
jours ; une modification de son URL la rend immédiatement éligible.

### Découverte du logo des sites

Après l'actualisation réussie d'un flux `Other`, InkRiver cherche d'abord
l'icône déclarée par RSS, Atom ou JSON Feed, puis les balises d'icône du site et
enfin `/favicon.ico`. Medium et Substack conservent toujours leurs marques
officielles. Cette recherche n'a jamais lieu au démarrage ni depuis la WebView.

Les téléchargements réutilisent les protections réseau public et de redirection
de l'extraction des articles. Chaque image est limitée à 512 Kio, contrôlée puis
normalisée en PNG transparent 64 × 64 avant sa mise en cache dans SQLite. Un
logo trouvé reste disponible hors ligne et n'est plus téléchargé tant que le
site du flux ne change pas. Un échec est retenté après sept jours ; un nouveau
site ou une nouvelle icône déclarée rend la recherche immédiatement éligible.
Les échecs restent silencieux et l'icône RSS générique demeure le repli.

Organisation principale :

```text
src/config.rs   lecture et validation de feeds.toml
src/cli.rs      arguments, commandes, rendu et codes de sortie
src/http.rs     téléchargement HTTP asynchrone
src/feed.rs     conversion RSS/Atom vers le modèle commun
src/content_extractor.rs  extraction hors ligne et nettoyage du contenu principal
src/page_http.rs  téléchargement borné des pages sur le réseau public
src/enrichment.rs  sélection, concurrence et persistance des extractions
src/feed_logo.rs  découverte et normalisation sûres des logos de sites
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
- l'interface responsive Android doit encore être validée sur un appareil réel
  ou un émulateur ;
- la génération du projet Android et de l'APK nécessite un JDK, un SDK et un NDK
  configurés ;
- `inkriver.db` du CLI se trouve encore dans le dépôt de développement.

L'étape suivante consiste à initialiser, exécuter et affiner l'application
Android sur un appareil. Dans l'application installée, SQLite reste la source
de vérité dans le répertoire AppData privé du bundle
`io.github.r0m1-b.inkriver`.

## Licence

InkRiver est distribué sous la [licence MIT](LICENSE).

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

Fermer les autres processus `inkriver` et les sessions `sqlite3` ouvertes sur
`inkriver.db`, puis réessayer. Ne supprimer les fichiers de la base qu'après avoir
arrêté ces processus.

### Une migration échoue

Conserver le message d'erreur, sauvegarder la base et vérifier l'ordre ainsi que
le contenu des fichiers dans `migrations/`. En développement seulement, une
réinitialisation complète permet de repartir d'un schéma vierge.

### Une erreur GLIBC mentionne `/snap/core20`

Un terminal intégré à une installation snap de VS Code peut injecter ses propres
variables GTK/GIO. Elles mélangent alors les bibliothèques du snap et celles du
système. Lancer InkRiver depuis un terminal Ubuntu normal. Pour un diagnostic
ponctuel depuis le terminal intégré, retirer notamment `GTK_PATH`,
`GIO_MODULE_DIR`, `GDK_PIXBUF_MODULE_FILE`, `GSETTINGS_SCHEMA_DIR` et `LOCPATH`
de l'environnement avant `npm run tauri dev`.
