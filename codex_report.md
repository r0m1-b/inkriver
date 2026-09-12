# Rapport des interventions Codex

Ce fichier conserve un historique synthétique des modifications réalisées avec
Codex et des raisons qui ont guidé les choix. Les nouvelles entrées sont ajoutées
à la fin du document.

## 2026-08-04 — État initial : extraction du service de collecte

### Objectif

Séparer la collecte des flux du point d'entrée CLI et rendre l'agrégation
testable sans connexion à Medium ou Substack.

### Actions

- création de `src/service.rs` ;
- déplacement de la conversion des flux, du chargement HTTP, de l'agrégation et
  du tri dans ce module ;
- ajout d'une injection de chargeur pour les tests hors ligne ;
- réduction de `src/main.rs` au chargement de la configuration, à la collecte et
  à l'affichage ;
- ajout de tests unitaires pour chacune des fonctions introduites ;
- suppression d'un `clone()` inutile dans un test existant.

### Justification

Le cœur métier ne doit pas dépendre du CLI. L'injection du chargeur permet de
tester la chronologie complète avec les fixtures locales, rapidement et de
manière déterministe.

### Vérifications

- `cargo test` : 20 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme.

Commit associé : `9b7a38b` (`Extract feed collection service`).

## 2026-08-04 — Mise en place du journal persistant

### Objectif

Conserver, au-delà de la conversation courante, une trace des changements et de
leur motivation.

### Actions

- création de `codex_report.md` comme journal append-only ;
- création de `AGENTS.md` pour imposer la mise à jour du rapport lors des futurs
  travaux qui modifient le dépôt ;
- définition d'un format commun : objectif, actions, justification,
  vérifications et commit éventuel.

### Justification

Une instruction stockée à la racine du dépôt est relue lors des futures sessions
de travail. Le rapport versionné complète l'historique Git en expliquant le
pourquoi des changements, pas seulement leur contenu.

### Vérifications

Aucun test applicatif n'a été exécuté : cette étape ajoute uniquement de la
documentation et des instructions de travail.

## 2026-08-04 — Tolérance à l'échec individuel d'un flux

### Objectif

Empêcher un abonnement indisponible d'annuler les articles récupérés depuis les
autres flux configurés.

### Actions

- ajout préalable d'un test reproduisant le défaut : premier flux en erreur,
  second flux valide ;
- constat de l'échec du test avec l'ancienne propagation immédiate par `?` ;
- introduction de `CollectionReport`, qui sépare les articles récupérés des
  erreurs de collecte ;
- introduction de `FeedCollectionError`, qui conserve l'identifiant, l'URL et
  le message d'erreur du flux concerné ;
- poursuite de la boucle de collecte après une erreur individuelle ;
- affichage des erreurs sur la sortie d'erreur du CLI sans masquer les articles
  disponibles ;
- adaptation des tests existants à la nouvelle API et ajout d'un test du format
  d'erreur.

### Justification

Un lecteur multi-sources doit rester utilisable lorsqu'une publication est
temporairement indisponible. Un rapport explicite évite de perdre les succès
tout en conservant des erreurs exploitables par le CLI et, plus tard, par
l'interface Tauri.

### Vérifications

- test TDD initial : échec attendu avec `Astronomy feed unavailable` ;
- tests ciblés du module `service` : 8 réussis ;
- `cargo test` : 22 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit du correctif
`Keep articles when a feed fails`.

## 2026-08-04 — Identifiants d'articles uniques entre les flux

### Objectif

Éviter qu'un même GUID publié par deux abonnements différents produise une
collision dans la chronologie et, plus tard, dans SQLite.

### Actions

- ajout préalable d'un test créant deux flux avec le même identifiant d'entrée ;
- constat de la collision avec l'ancien modèle, qui copiait directement le GUID ;
- ajout de l'identifiant configuré de l'abonnement au modèle `Feed` ;
- propagation de `FeedConfig.id` lors de la construction d'un flux téléchargé ;
- génération des identifiants d'articles sous la forme
  `feed_id::entry_id` ;
- attribution d'identifiants stables aux deux flux de test ;
- adaptation des tests qui vérifiaient auparavant les GUID bruts.

### Justification

Un GUID RSS n'est garanti unique qu'à l'intérieur de son flux. Le préfixer avec
l'identifiant stable déclaré dans `feeds.toml` crée un espace de noms par
abonnement et prépare une future clé de stockage. Cette stratégie suppose que
chaque abonnement possède un `id` de configuration unique.

### Vérifications

- test TDD initial : échec attendu, les deux articles ayant
  `substack-astronomie-1` ;
- test ciblé après correction : identifiants
  `first::substack-astronomie-1` et `second::substack-astronomie-1` ;
- `cargo test` : 23 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit du correctif
`Namespace article IDs by feed`.

## 2026-08-04 — Conservation du contexte des erreurs de flux

### Objectif

Rendre les erreurs exploitables en conservant le statut HTTP exact et l'étape
du chargement qui a échoué, sans produire d'affichage depuis le cœur métier.

### Actions

- ajout préalable de tests exigeant les statuts HTTP `404 Not Found` et
  `500 Internal Server Error` dans les messages ;
- ajout préalable d'un test exigeant l'identification de l'étape « requête
  HTTP » pour une URL invalide ;
- suppression des messages génériques et des `println!` dans le module HTTP ;
- introduction de `FeedLoadStage` pour distinguer requête HTTP, lecture du
  corps, parsing du flux et validation des métadonnées ;
- introduction de `FeedLoadError`, qui associe une étape à son message source ;
- intégration de cette erreur structurée dans `FeedCollectionError` ;
- ajout de tests pour les libellés d'étapes et le formatage complet des erreurs.

### Justification

Une erreur doit pouvoir être présentée différemment par le CLI ou Tauri sans que
le cœur écrive directement dans le terminal. Conserver le statut et l'étape
permet également de diagnostiquer rapidement si un problème vient du réseau, du
contenu de la réponse ou du document RSS.

### Vérifications

- tests TDD initiaux : échecs attendus sur les messages génériques et l'absence
  d'étape HTTP ;
- tests ciblés HTTP : 3 réussis ;
- tests ciblés du service : 10 réussis ;
- `cargo test` : 25 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit du correctif
`Preserve feed error context`.

## 2026-08-08 — Injection du contenu lors du chargement d'un flux

### Objectif

Tester hors ligne le véritable pipeline de chargement d'un flux, depuis son
contenu RSS jusqu'au modèle `Feed`, sans contourner le parsing XML ni la
validation des métadonnées.

### Actions

- ajout préalable de tests pour un RSS valide, un XML invalide, un flux sans
  titre et un flux sans lien principal ;
- constat de l'échec initial, le point d'injection attendu
  `load_feed_with_content_loader` n'existant pas encore ;
- extraction d'un pipeline commun recevant le contenu téléchargé par une
  fonction injectée ;
- utilisation de ce même pipeline par le chargement HTTP de production ;
- conservation des étapes d'erreur distinctes pour la requête, la lecture du
  corps, le parsing et les métadonnées.

### Justification

Injecter un objet `Feed` déjà construit rendait les tests rapides, mais ne
validait pas la partie la plus fragile du chargement. L'injection au niveau du
contenu garde les tests déterministes tout en exerçant le parsing et la
conversion réellement utilisés en production.

### Vérifications

- test TDD initial : échec de compilation attendu, la fonction d'injection du
  contenu n'étant pas encore définie ;
- tests ciblés du chargement : 5 réussis, aucun échec ;
- `cargo test` : 29 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Inject feed content loading`.

## 2026-08-08 — Garantie du tri des articles sans date

### Objectif

Garantir que les articles dépourvus de date de publication apparaissent après
tous les articles datés dans la chronologie commune.

### Actions

- ajout d'un test de régression mélangeant trois articles datés et deux
  articles sans date ;
- vérification explicite des deux partitions obtenues après le tri ;
- conservation de l'implémentation existante, qui satisfait déjà cette
  politique grâce à l'ordre de `Option` inversé par `Reverse`.

### Justification

Le comportement existait, mais le précédent test de tri n'utilisait que des
articles datés et ne le protégeait donc pas contre une régression. Modifier le
code de production alors que le cas demandé fonctionne déjà aurait ajouté de
la complexité sans corriger de défaut.

### Vérifications

- test ajouté en premier : réussite immédiate, confirmant que le comportement
  était déjà conforme ; aucun échec TDD n'a été artificiellement provoqué ;
- test ciblé du tri des dates absentes : 1 réussi, aucun échec ;
- `cargo test` : 30 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Cover undated article sorting`.

## 2026-08-08 — Politique explicite des champs d'article absents

### Objectif

Supprimer les valeurs sentinelles du cœur métier et exploiter le résumé RSS ou
Atom lorsqu'un flux ne fournit pas le corps complet d'un article.

### Actions

- ajout préalable de tests pour le repli vers `entry.summary`, avec contenu
  totalement absent ou contenu présent sans corps ;
- ajout préalable d'un test exigeant `None` lorsque le titre, l'URL et le
  contenu sont absents ;
- transformation de `Article.title`, `Article.url` et `Article.content` en
  `Option<String>` ;
- définition de la priorité du contenu : corps complet, puis résumé, puis
  absence explicite ;
- déplacement des libellés `(untitled)` et `(no URL)` dans le seul formateur
  CLI, avec un test dédié ;
- adaptation des tests existants au modèle optionnel et vérification que le
  corps complet reste prioritaire sur le résumé.

### Justification

Les textes `"No title"`, `"No content"`, `"No body"` et l'URL vide mélangeaient
données et présentation, tout en empêchant une future interface de distinguer
une absence réelle. Les valeurs optionnelles conservent l'information du flux
et laissent chaque interface Linux ou Android choisir son propre affichage.

### Vérifications

- premier test TDD : échec attendu, `"No content"` étant renvoyé à la place du
  résumé réel ;
- second état rouge : échec de compilation attendu, les champs du modèle étant
  encore des `String` au lieu de `Option<String>` ;
- tests ciblés de conversion d'article : 4 réussis, aucun échec ;
- `cargo test` : 34 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Model missing article fields explicitly`.

## 2026-08-08 — Validation des identifiants de flux

### Objectif

Empêcher les identifiants de flux invalides de produire des espaces de noms
d'articles vides ou des collisions entre abonnements.

### Actions

- ajout préalable d'un test couvrant un identifiant vide et un identifiant
  composé uniquement d'espaces ;
- ajout préalable d'un test couvrant deux flux avec le même identifiant ;
- introduction de `validate_feed_ids`, appelée après la désérialisation de
  toute configuration ;
- détection des doublons en un passage avec un `HashSet` ;
- retour de messages distincts pour un identifiant blanc et un doublon.

### Justification

L'identifiant configuré préfixe désormais chaque identifiant d'article. Il doit
donc être présent et unique avant toute collecte. Effectuer cette validation au
chargement empêche une configuration ambiguë d'atteindre le réseau ou le futur
stockage SQLite.

### Vérifications

- tests TDD initiaux : 2 échecs attendus, les configurations blanche et
  dupliquée étant auparavant acceptées ;
- tests ciblés de validation : 2 réussis, aucun échec ;
- `cargo test` : 36 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Validate feed identifiers`.

## 2026-08-08 — Identité stable et déduplication des articles

### Objectif

Produire des identifiants reproductibles lorsque les flux omettent un GUID et
éviter qu'une même entrée apparaisse plusieurs fois dans l'agrégation.

### Actions

- ajout préalable d'un test chargeant deux fois un article sans GUID ni lien
  d'entrée et exigeant le même identifiant ;
- ajout préalable d'un test exigeant l'URL de l'entrée comme repli lorsqu'un
  objet `Entry` ne possède pas d'identifiant ;
- ajout préalable d'un test agrégeant deux occurrences du même article ;
- configuration du parseur `feed-rs` avec l'URL du flux comme URL de base, ce
  qui rend déterministe l'identifiant généré depuis le titre quand GUID et lien
  sont absents ;
- génération du repli sous la forme `feed_id::url::entry_url` pour les entrées
  brutes sans identifiant ;
- déduplication en un passage avec un `HashSet`, en conservant la première
  occurrence de chaque identifiant namespacé avant le tri.

### Justification

Sans URL de base, `feed-rs` générait un UUID aléatoire pour certaines entrées et
un rafraîchissement pouvait donc recréer le même article. L'identifiant du flux
reste dans la clé afin que deux abonnements distincts ne se masquent pas. Le
repli URL fournit une identité lisible et stable lorsqu'il est nécessaire.

### Vérifications

- tests TDD initiaux : 3 échecs attendus, avec deux UUID différents, un
  identifiant terminé par `::` et deux articles agrégés au lieu d'un ;
- nouveaux tests ciblés : stabilité, repli URL et déduplication réussis ;
- `cargo test` : 39 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Deduplicate articles with stable identities`.

## 2026-08-08 — Centralisation de la conversion des plateformes

### Objectif

Définir une seule correspondance exhaustive entre les plateformes déclarées
dans la configuration et les sources utilisées par le modèle métier.

### Actions

- ajout préalable d'un test exigeant la conversion de `Medium`, `Substack` et
  `Other` vers les trois variantes correspondantes de `Source` ;
- implémentation de `From<Platform> for Source` dans le module de
  configuration ;
- remplacement du `match` local du service par `Source::from(platform)`.

### Justification

Une implémentation standard de `From` rend la conversion réutilisable et oblige
le compilateur à signaler toute nouvelle variante qui ne serait pas traitée.
Le service reste ainsi centré sur la construction du flux plutôt que sur une
règle de correspondance recopiable.

### Vérifications

- test TDD initial : échec de compilation attendu, aucune implémentation de
  `From<Platform>` n'existant pour `Source` ;
- test ciblé de conversion : 1 réussi, aucun échec ;
- `cargo test` : 40 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Centralize platform source conversion`.

## 2026-08-08 — Assainissement du HTML des articles

### Objectif

Empêcher qu'un contenu RSS ou Atom non fiable exécute du code actif lorsqu'il
sera affiché dans une WebView Linux ou Android.

### Actions

- ajout préalable d'un test injectant dans le corps complet un script, un
  gestionnaire `onclick` et une URL `javascript:` ;
- ajout préalable d'un test injectant dans le résumé de repli un gestionnaire
  `onerror` et une balise `style` ;
- ajout de la dépendance directe `ammonia`, résolue en version 4.1.4 dans
  `Cargo.lock` ;
- nettoyage du contenu après le choix entre corps complet et résumé, afin que
  les deux chemins suivent la même politique ;
- vérification de la conservation du texte, des balises sûres et des liens
  HTTPS.

### Justification

La fonctionnalité optionnelle d'assainissement de `feed-rs` n'était pas
activée et ne nettoie volontairement pas les valeurs typées comme texte brut.
Or le modèle `Article` ne conserve pas le type MIME. Assainir à la frontière de
conversion garantit donc que tout `Article.content` destiné à une WebView est
inerte, y compris lorsqu'il provient du résumé.

### Vérifications

- tests TDD initiaux : 2 échecs attendus, le HTML dangereux étant conservé ;
- tests ciblés d'assainissement : 2 réussis, aucun échec ;
- `cargo test` : 42 tests réussis, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Sanitize article HTML content`.

## 2026-08-08 — Résolution déterministe de `feeds.toml`

### Objectif

Permettre au CLI de trouver sa configuration de développement même lorsqu'il
est lancé depuis un répertoire autre que la racine du projet.

### Actions

- ajout préalable d'un test exigeant un chemin absolu, terminé par
  `feeds.toml` et ancré sur le répertoire du manifeste Cargo ;
- introduction de `default_config_path`, qui construit ce chemin à partir de
  `CARGO_MANIFEST_DIR` au lieu du répertoire courant du processus ;
- utilisation directe du `PathBuf` obtenu par `main` lors du chargement de la
  configuration.

### Justification

Le chemin relatif précédent dépendait du CWD choisi par le terminal, un script
ou un lanceur graphique. Le manifeste Cargo fournit une ancre déterministe pour
le CLI actuel et préserve le fonctionnement de `cargo run`. Les futures
applications Tauri et Android pourront fournir leurs propres répertoires de
configuration sans réintroduire cette dépendance au CWD.

### Vérifications

- test TDD initial : échec de compilation attendu, le résolveur de chemin
  n'existant pas encore ;
- test ciblé du chemin par défaut : 1 réussi, aucun échec ;
- `cargo test` : 43 tests réussis au total, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt`.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Resolve config independently of working directory`.

## 2026-08-08 — Collecte HTTP asynchrone

### Objectif

Supprimer le chemin réseau bloquant avant son intégration à une interface
Tauri, afin que les attentes réseau ne monopolisent pas le thread d'interface.

### Actions

- ajout préalable d'un test de contrat exigeant que `collect_articles` retourne
  un `Future` ;
- retrait de la feature `blocking` de `reqwest` ;
- passage de `check_feed_url`, de la lecture du corps HTTP, du chargeur de flux
  et de la collecte publique en `async`/`await` ;
- adaptation du chargeur injecté pour accepter des futurs et conservation des
  tests hors ligne déterministes avec `std::future::ready` ;
- ajout de Tokio avec les seules features `macros` et `rt`, puis conversion du
  CLI vers un runtime mono-thread ;
- possession d'une copie légère de `FeedConfig` par chaque futur de chargement
  pour éviter des futurs boxés liés à des emprunts ;
- adaptation des tests réseau et d'agrégation en tests Tokio asynchrones.

### Justification

L'API asynchrone de `reqwest` rend la main au runtime pendant la requête et la
lecture de la réponse. Le cœur peut désormais être attendu directement depuis
une commande Tauri sans encapsuler du HTTP bloquant. La collecte reste
séquentielle pour préserver l'ordre, la tolérance aux erreurs et la simplicité
du comportement actuel.

### Vérifications

- test TDD initial : échec de compilation attendu, `CollectionReport`
  n'implémentant pas `Future` ;
- test de contrat async : 1 réussi, aucun échec ;
- `cargo test` : 44 tests réussis au total, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- `cargo fmt --check` : formatage conforme après application de `cargo fmt` ;
- recherche dans le code et l'arbre des features : aucune API
  `reqwest::blocking` ni feature `reqwest/blocking` restante.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Make feed collection asynchronous`.

## 2026-08-08 — Flux sans description et persistance future Tauri

### Objectif

Verrouiller le comportement d'un flux RSS sans description et consigner le
choix de persistance de l'application installée avant l'introduction de SQLite
et de Tauri.

### Actions

- ajout d'une fixture RSS en mémoire sans élément `description` ;
- ajout d'un test passant par `load_feed_with_content_loader` et vérifiant que
  le flux est accepté avec une description vide ;
- documentation dans `TODO.md` de SQLite comme source de vérité des abonnements
  et articles de l'application installée ;
- choix de `AppData/io.github.r0m1.reader/reader.db` comme emplacement futur de
  la base et de `io.github.r0m1.reader` comme identifiant de bundle Tauri ;
- maintien de `feeds.toml` pour le CLI actuel et comme futur format
  d'import/export explicite, sans synchronisation automatique avec SQLite ;
- réservation de `AppConfig` aux éventuels réglages distincts de la base.

### Justification

La description d'un flux est optionnelle dans le modèle métier et doit donc
avoir un comportement de repli stable. Les abonnements et états de lecture sont
des données applicatives modifiées par l'interface : les conserver dans SQLite
sous `AppData` évite deux sources de vérité concurrentes et fonctionne avec les
répertoires privés fournis par Tauri sur Linux et Android.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo check` : compilation réussie ;
- `cargo test` : 45 tests réussis au total, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- le nouveau test injecte une chaîne RSS en mémoire et n'effectue aucun accès
  réseau.

Ces changements ont été générés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Cover missing feed descriptions and document Tauri storage`.

## 2026-08-08 — Préparation du modèle pour SQLite

### Objectif

Introduire SQLx et rendre explicites les valeurs métier qui seront persistées
avant de créer le schéma de stockage.

### Actions

- ajout de SQLx 0.9 avec les fonctionnalités Tokio, SQLite, Chrono et migrations ;
- ajout de `feed_id` au modèle `Article` et alimentation de ce champ lors de la
  conversion des entrées RSS ;
- ajout de représentations textuelles stables et de conversions contrôlées pour
  `Source` et `Platform` ;
- adaptation des constructions d'articles et ajout de tests de conversion et de
  rejet des valeurs inconnues.

### Justification

Une clé étrangère explicite évite de déduire le flux depuis l'identifiant
concaténé de l'article. Des conversions centralisées empêchent également la
base de dépendre des noms de variantes Rust produits par le débogueur.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo test` : 49 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

## 2026-08-29 — Checkpoint compact v2 (suite de SYNC-011)

### Objectif

Rendre les instantanés indépendants de l'historique complet afin de préparer la
suppression sûre des événements obsolètes.

### Actions et choix

- passage des nouveaux instantanés au format interne v2, avec conservation de
  la lecture du format contigu v1 ;
- sélection transactionnelle des seules créations d'abonnements, versions LWW
  gagnantes, pierres tombales et dépendances non résolues ;
- conservation séparée des frontières contiguës complètes par journal ;
- import atomique des événements clairsemés et avancement explicite des curseurs
  jusqu'aux frontières authentifiées ;
- maintien de l'absence de corps d'article et des limites existantes ;
- aucune suppression activée tant que l'état « confirmé » ne peut pas être
  distingué sans ambiguïté d'un checkpoint omis.

### Vérifications

- tests ciblés : reconstruction v2, réduction de 51 événements à 2, avancement
  du curseur clairsemé, reprise des segments suivants et compatibilité v1 :
  succès ;
- `cargo test --workspace -q` : succès, 242 tests passés et un test de corpus
  local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce sixième lot SYNC-011.

## 2026-08-29 — Confirmation distante des checkpoints (suite de SYNC-011)

### Objectif

Éliminer l'ambiguïté entre un checkpoint récupérable et un checkpoint omis
avant de brancher la première suppression.

### Actions et choix

- remplacement du retour optionnel par trois états explicites : prêt à publier,
  confirmé inchangé et indisponible ;
- distinction des limites dépassées : nombre d'événements retenus, état
  sérialisé et document chiffré ;
- vérification distante systématique d'un checkpoint inchangé par téléchargement,
  authentification AEAD, validation de format et comparaison d'empreinte ;
- republication atomique lorsque le fichier distant confirmé localement a
  disparu ou a été corrompu ;
- extraction d'un booléen interne de confirmation destiné au prochain lot de
  compaction, sans suppression dans ce lot.

### Vérifications

- tests ciblés des trois motifs d'indisponibilité, de l'état inchangé et de la
  réparation distante après suppression puis corruption : succès ;
- `cargo test --workspace -q` : succès, 243 tests passés et un test de corpus
  local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès après correction de la portée d'un verrou de test.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce septième lot SYNC-011.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Prepare domain models for SQLite storage`.

## 2026-08-08 — Schéma SQLite et ouverture du stockage

### Objectif

Créer le socle persistant de l'application et garantir que le même schéma est
appliqué aux bases fichier et aux bases de test en mémoire.

### Actions

- ajout d'une migration initiale créant `feeds`, `articles`, leurs contraintes,
  leur relation et leurs index ;
- ajout du module `storage` ouvrant ou créant une base avec les clés étrangères
  actives puis exécutant les migrations embarquées ;
- ajout d'un script de build pour recompilier lorsque les migrations changent ;
- ajout de `tempfile` pour vérifier la création réelle d'une base isolée ;
- ajout d'une fermeture explicite du pool SQLite.

### Justification

Les migrations embarquées permettront au futur binaire Tauri d'initialiser sa
base sans distribuer séparément des fichiers SQL. Une connexion unique est
utilisée pour les bases en mémoire afin que tous les accès de test partagent la
même base.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo test storage::tests` : 2 tests réussis, aucun échec ;
- `cargo test` avant l'ajout de la fermeture explicite : 51 tests réussis au
  total, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Add migrated SQLite storage foundation`.

## 2026-08-08 — Import transactionnel des abonnements

### Objectif

Faire de chaque rafraîchissement CLI un import explicite de `feeds.toml` tout
en conservant les abonnements retirés et leur futur historique.

### Actions

- ajout de `Storage::import_feeds` dans une transaction SQLite ;
- désactivation préalable de l'ensemble connu puis upsert des abonnements de la
  configuration comme actifs ;
- ajout du modèle de lecture `StoredFeed` et de `Storage::list_feeds` ;
- ajout d'un index unique limité aux URL actives ;
- ajout de tests d'insertion, de mise à jour, de désactivation sans suppression
  et de rollback atomique en cas de conflit.

### Justification

La désactivation conserve l'identité et l'historique d'un flux retiré. La
transaction évite de laisser tous les flux inactifs si une entrée invalide
interrompt l'import après sa première requête.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo test storage::tests::import_feeds` : 3 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Import feed subscriptions transactionally`.

## 2026-08-08 — Persistance et lecture des articles

### Objectif

Enregistrer les articles collectés sans doublon ni perte de données et fournir
la liste chronologique nécessaire au lecteur hors ligne.

### Actions

- ajout du modèle `StoredArticle`, séparant les données distantes des états
  locaux ;
- ajout d'un upsert transactionnel fondé sur l'identifiant stable de l'article ;
- conservation des anciennes valeurs non nulles lorsqu'un rafraîchissement
  fournit une entrée moins complète ;
- ajout de la lecture de tous les articles, datés du plus récent au plus ancien
  puis non datés ;
- ajout de tests de round-trip, déduplication, mise à jour sans perte, tri et
  atomicité d'un lot invalide.

### Justification

Les flux RSS peuvent raccourcir ou omettre certains champs au fil du temps. Ne
pas remplacer une donnée locale utile par `NULL` améliore la lecture hors ligne,
tandis que la transaction empêche les lots partiellement persistés.

### Vérifications

- `cargo fmt` puis contrôle implicite du formatage ;
- `cargo test storage::tests` : 9 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Persist and order collected articles`.

## 2026-08-08 — États locaux de lecture et de favori

### Objectif

Permettre au lecteur de modifier ses états locaux indépendamment des données
reçues depuis les flux et garantir leur stabilité au rafraîchissement.

### Actions

- ajout de `Storage::set_read` et `Storage::set_favorite` ;
- retour d'un booléen distinguant un article ciblé d'un identifiant absent ;
- ajout de tests d'activation, de désactivation et d'identifiant inconnu pour
  chacun des deux états ;
- ajout d'un test de régression vérifiant qu'un upsert distant conserve les deux
  états tout en actualisant le titre.

### Justification

Les états `is_read` et `is_favorite` appartiennent à l'utilisateur et ne doivent
jamais être dérivés ni remis à zéro par le contenu distant.

### Vérifications

- `cargo fmt` : formatage appliqué ;
- `cargo test storage::tests` : 12 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Preserve local article states`.

## 2026-08-08 — Orchestration du rafraîchissement persistant

### Objectif

Relier l'import des abonnements, la collecte réseau et le stockage sans sacrifier
les données hors ligne lorsqu'un flux échoue.

### Actions

- ajout du module `refresh` et de son API asynchrone publique ;
- import des abonnements avant collecte puis upsert des seuls articles obtenus ;
- conservation des erreurs de collecte par flux dans le rapport retourné ;
- injection du résultat de collecte dans les tests afin d'éviter tout accès
  réseau ;
- ajout de tests couvrant les succès partiels, l'échec réseau avec cache, le
  second rafraîchissement sans doublon, la conservation des états locaux et la
  désactivation d'un abonnement sans perte d'historique.

### Justification

Les erreurs réseau sont attendues dans un lecteur hors ligne et ne constituent
pas une erreur de stockage. Séparer le rapport de collecte des erreurs SQLite
permet au CLI puis à Tauri de présenter les premières tout en arrêtant proprement
sur les secondes.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo test refresh::tests` : 4 tests réussis, aucun échec et aucun accès
  réseau.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Orchestrate persistent feed refresh`.

## 2026-08-08 — Branchement du CLI sur le stockage local

### Objectif

Faire utiliser la base persistante au programme courant et continuer à afficher
le cache lorsqu'une collecte signale des erreurs réseau.

### Actions

- ajout du chemin de développement `CARGO_MANIFEST_DIR/reader.db` ;
- ouverture de SQLite, exécution du rafraîchissement puis relecture de tous les
  articles stockés dans le CLI ;
- séparation de la restitution vers stdout et stderr pour la tester sans lancer
  de requête réseau ;
- ajout des fichiers SQLite et de leurs journaux à `.gitignore` ;
- ajout de tests du chemin de base et de l'affichage simultané d'une erreur de
  collecte et d'un article conservé.

### Justification

Relire SQLite après le rafraîchissement, au lieu d'afficher seulement le lot
téléchargé, rend le comportement hors ligne visible dès le CLI et valide le
contrat dont l'interface Tauri aura besoin.

### Vérifications

- `cargo fmt` : formatage appliqué ;
- `cargo test --bin reader` : 3 tests réussis, aucun échec et aucun accès réseau.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Read cached articles from the CLI`.

## 2026-08-08 — Clôture et audit de l'étape SQLite

### Objectif

Vérifier l'étape 6 contre son scénario d'acceptation complet et aligner la
feuille de route avec l'état réel du dépôt.

### Actions

- passage de tous les éléments SQLite de `TODO.md` à l'état terminé et
  clarification de l'import TOML déclenché par chaque rafraîchissement CLI ;
- mise à jour des dépendances et des acquis du cœur Rust dans la feuille de
  route, puis ajout de ce fichier au suivi Git ;
- renforcement du test de scénario : utilisation d'une base fichier temporaire,
  fermeture et réouverture entre les rafraîchissements connecté, répété et hors
  ligne ;
- vérification dans ce test de l'absence de doublon, de la conservation des
  états lu/favori, de la désactivation d'un abonnement retiré, de la persistance
  de son historique et du rapport d'erreur réseau ;
- exclusion de `target`, du fichier personnel `feeds.toml` et des fichiers de
  travail SQLite du suivi Git.

### Justification

Une base en mémoire unique ne prouvait pas à elle seule la persistance entre
deux lancements. Fermer puis rouvrir le fichier temporaire transforme le test en
preuve directe du scénario attendu sans dépendre d'Internet ni modifier la base
de développement de l'utilisateur.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo check` : compilation réussie ;
- `cargo test` : 67 tests réussis au total, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement ;
- audit du scénario : chaque exigence est couverte par le test persistant du
  module `refresh` et par le test de restitution hors ligne du CLI.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Complete and document SQLite milestone`.

## 2026-08-08 — Documentation de prise en main

### Objectif

Fournir un guide autonome pour installer, configurer, utiliser et maintenir le
prototype CLI, en détaillant particulièrement le cycle de vie de SQLite.

### Actions

- ajout de `README.md` avec la présentation et les prérequis Ubuntu ;
- documentation de `feeds.toml`, du lancement et du comportement hors ligne ;
- explication de la création automatique de `reader.db`, de son inspection, de
  sa sauvegarde, de sa réinitialisation et des migrations ;
- ajout des commandes de développement, de l'architecture, des limites
  actuelles et des principaux cas de dépannage.

### Justification

SQLite est embarqué par SQLx : distinguer clairement la création automatique de
la base du client `sqlite3` optionnel évite une installation inutile. Les
avertissements de sauvegarde et de perte des états locaux rendent également la
procédure de réinitialisation explicite.

### Vérifications

- comparaison du guide avec `Cargo.toml`, `main.rs`, `storage.rs`, la migration
  initiale et `.gitignore` ;
- `cargo tree -e features -i libsqlite3-sys` : fonctionnalité SQLite `bundled`
  confirmée ;
- aucun test exécuté à ce stade, le changement étant uniquement documentaire.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun commit créé pour cette intervention.

## 2026-08-08 — Priorisation des constats de l'audit

### Objectif

Reporter dans la feuille de route les quatre améliorations techniques relevées
par l'audit du cœur Rust et les classer selon leur priorité.

### Actions

- ajout dans `TODO.md` de deux correctifs P2 sur l'identité des articles et la
  séparation entre chronologie et contenu détaillé ;
- ajout de deux améliorations P3 sur la validation de `feeds.toml` et la
  collecte HTTP concurrente avec un client partagé ;
- formulation de chaque constat comme une tâche vérifiable sans transformer les
  fonctionnalités encore absentes du prototype en anomalies.

### Justification

Les risques de doublons et de chargement intégral de l'archive peuvent affecter
les données ou la consommation de ressources et sont donc classés P2. La
validation anticipée et l'optimisation des téléchargements améliorent surtout
l'expérience et la montée en charge du prototype ; elles sont classées P3.

### Vérifications

- `cargo fmt --all -- --check` : formatage Rust conforme ;
- `git diff --check` : aucune erreur d'espacement ;
- relecture de la section ajoutée : quatre tâches présentes, dont deux P2 et
  deux P3.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Stabilize the reader CLI`.

## 2026-08-08 — Identité stable des articles sans GUID

### Objectif

Empêcher qu'un changement de titre modifie l'identifiant d'un article dont le
flux ne fournit pas de GUID.

### Actions

- remplacement du générateur implicite de `feed-rs` par un marqueur interne
  signalant les identifiants absents ;
- application de la stratégie GUID éditeur, puis URL canonique sans fragment,
  puis empreinte SHA-256 des champs stables disponibles ;
- ajout de `sha2` comme dépendance directe ;
- ajout de tests unitaires et de chargement RSS pour le GUID, l'URL canonique,
  le changement de titre et le repli sans URL.

### Justification

Le générateur par défaut de `feed-rs` mélange l'URL et le titre. En distinguant
explicitement un GUID fourni d'un identifiant synthétique, Reader maîtrise la
stabilité de ses clés SQLite.

### Vérifications

- `cargo fmt --check` : formatage conforme ;
- `cargo test feed::tests` : 17 tests réussis, aucun échec ;
- `cargo test service::tests::load_feed_` : 8 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Stabilize the reader CLI`.

## 2026-08-08 — Chronologie légère et détail à la demande

### Objectif

Éviter de charger le contenu HTML de toute l'archive pour afficher seulement la
liste des articles.

### Actions

- ajout du modèle léger `ArticleSummary` sans corps d'article ;
- ajout de `Storage::list_article_summaries` avec le même ordre chronologique ;
- ajout de `Storage::get_article` pour charger un détail complet par identifiant ;
- factorisation de la conversion des lignes SQLite complètes ;
- ajout de tests pour les métadonnées, les états locaux, l'ordre, le contenu
  détaillé et l'identifiant absent.

### Justification

La chronologie sera l'opération la plus fréquente du CLI puis de Tauri. Séparer
son modèle du détail limite les données lues et établit l'API nécessaire au
panneau de lecture futur.

### Vérifications

- `cargo fmt` : formatage appliqué ;
- `cargo test storage::tests` : 14 tests réussis, aucun échec.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Stabilize the reader CLI`.

## 2026-08-08 — Commandes CLI et rafraîchissement observable

### Objectif

Séparer le réseau de la lecture hors ligne et exposer une interface en ligne de
commande stable avant l'intégration Tauri.

### Actions

- ajout de Clap et des commandes `refresh`, `list`, `show`, `mark-read`,
  `mark-unread`, `favorite` et `unfavorite` ;
- ajout des options globales `--config` et `--database`, avec affichage de
  l'aide lorsqu'aucune commande n'est fournie ;
- sélection d'un article par identifiant stable ou position dans la liste ;
- conversion du HTML en texte terminal avec `html2text` et marquage automatique
  comme lu lors de `show` ;
- ajout de catégories d'erreurs explicites et d'un code `2` pour les
  rafraîchissements partiellement réussis ;
- ajout des compteurs de flux actifs, articles reçus, insérés et mis à jour ;
- réduction de `main.rs` au parsing, à l'écriture des sorties et au code retour ;
- ajout de tests unitaires pour chaque commande et de tests du vrai binaire pour
  l'aide et la lecture hors ligne.

### Justification

Les commandes de consultation ne chargent ni `feeds.toml` ni le réseau. Le code
retour distinct d'un succès partiel rend les pannes visibles aux scripts sans
annuler les articles correctement persistés.

### Vérifications

- `cargo test storage::tests` : 15 tests réussis, aucun échec ;
- `cargo test refresh::tests` : 4 tests réussis, aucun échec ;
- `cargo test cli::tests` : 12 tests réussis, aucun échec ;
- `cargo test --test cli` : 2 tests du binaire réussis, aucun échec ;
- `cargo fmt --check` : formatage conforme ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Stabilize the reader CLI`.

## 2026-08-08 — Clôture de l'étape 7

### Objectif

Aligner la documentation sur le CLI stabilisé et vérifier l'ensemble du projet
avant le passage à Tauri.

### Actions

- mise à jour du README avec les sept commandes, les sélecteurs, les chemins
  personnalisés, les codes de sortie et les opérations entièrement hors ligne ;
- adaptation des procédures de création, réinitialisation et réactivation de la
  base au nouveau `refresh` explicite ;
- passage à l'état terminé des quatre éléments de l'étape 7 et des deux
  correctifs P2 effectivement traités, sans modifier les P3 restants ;
- traduction des descriptions de commandes affichées par Clap ;
- audit des critères : aide sans rafraîchissement implicite, succès partiel avec
  code `2`, sélection numéro ou ID, détail rendu en texte, états persistants et
  lecture sans configuration ni réseau.

### Justification

Les exemples doivent refléter le nouveau contrat explicite : seul `refresh`
consulte la configuration et le réseau, tandis que la consultation du cache
reste disponible indépendamment.

### Vérifications

- `cargo run -- --help` : toutes les commandes et options sont présentes ;
- `cargo fmt --check` : formatage conforme ;
- `cargo check` : compilation réussie ;
- `cargo test` : 85 tests réussis au total, aucun échec ;
- `cargo clippy --all-targets --all-features -- -D warnings` : aucun
  avertissement.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Stabilize the reader CLI`.

## 2026-08-08 — Modèle de contenu et abonnements applicatifs

### Objectif

Préparer le cœur Rust et SQLite à une application installée autonome, sans
synchronisation implicite avec `feeds.toml`.

### Actions

- ajout de `ContentKind` (`full`, `excerpt`, `missing`, `unknown`) et d'une
  migration conservant prudemment les contenus historiques comme `unknown` ;
- classification du corps complet, du résumé de repli et du contenu absent dès
  la conversion RSS/Atom ;
- ajout, normalisation HTTP(S), détection de plateforme et identifiant UUID des
  abonnements ;
- activation et désactivation sans suppression d'historique, avec rejet des
  URL actives dupliquées ;
- séparation du rafraîchissement CLI avec import TOML et du rafraîchissement
  applicatif à partir des seuls flux SQLite actifs ;
- ajout de tests de migration, de persistance, de réactivation et de
  rafraîchissement hors réseau.

### Justification

SQLite reste la source de vérité de l'application. Le type de contenu permet à
l'interface de proposer le lien original uniquement lorsque le flux ne livre
pas avec certitude un article complet.

### Vérifications

- `cargo check -p reader` : réussi ;
- `cargo test -p reader` : 96 tests réussis, aucun échec ;
- `cargo clippy -p reader --all-targets --all-features -- -D warnings` : aucun
  avertissement.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Add the Tauri Linux reader`.

## 2026-08-08 — Application Tauri et interface de lecture Linux

### Objectif

Créer l'interface graphique de l'étape 8 en réutilisant le cœur Rust existant.

### Actions

- transformation du projet en workspace et ajout d'une application Tauri 2
  sous `app/`, avec le bundle `io.github.r0m1.reader` ;
- initialisation de `reader.db` dans AppData et exposition de commandes typées
  pour les articles, les états locaux, le rafraîchissement et les abonnements ;
- ajout de DTO camelCase, dates RFC 3339 et erreurs structurées ;
- ajout d'une capability minimale et du plugin opener limité au contrat
  HTTP(S) vérifié par l'interface ;
- création d'une interface Vanilla TypeScript à deux panneaux avec cache au
  démarrage, lecteur isolé, favoris, gestion des abonnements et états
  chargement/vide/erreur/succès partiel ;
- ajout de onze tests Vitest couvrant les principaux parcours de l'interface et
  de tests Rust pour les adaptateurs Tauri.

### Justification

La couche Tauri reste une adaptation mince : SQLx et les règles métier restent
dans `reader`, ce qui préserve leur réutilisation future sous Android. Le HTML
nettoyé est rendu dans une iframe sans scripts pour ajouter une isolation à la
sanitisation du cœur.

### Vérifications

- `npm install` : 155 paquets installés, aucune vulnérabilité signalée ;
- `npm run typecheck` : réussi ;
- `npm test` : 11 tests réussis, aucun échec ;
- `npm run build` : bundle Vite produit avec succès ;
- `cargo metadata --no-deps --format-version 1` : workspace et dépendance locale
  reconnus ;
- `cargo check --workspace`, les tests Tauri et le build natif : non exécutables
  jusqu'au bout, car `pkg-config`, WebKitGTK 4.1 et librsvg ne sont pas installés
  et leur installation demande le mot de passe `sudo` ;
- paquets `.deb` et AppImage : non produits pour la même raison.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Add the Tauri Linux reader`.

## 2026-08-08 — Documentation de l'étape 8

### Objectif

Documenter l'installation, le stockage et les commandes de développement de
l'application Linux sans déclarer terminée la validation native encore bloquée.

### Actions

- ajout au README des prérequis Tauri Ubuntu, commandes npm, chemins AppData,
  procédures de construction et de réinitialisation ;
- mise à jour de la structure et des limites du projet ;
- mise à jour de l'étape 8 dans le TODO avec les éléments implémentés et une
  dernière case explicite pour la validation `.deb`/AppImage.

### Vérifications

- `git diff --check` : aucune erreur d'espacement avant la mise à jour finale ;
- relecture des commandes et chemins documentés effectuée ;
- aucune exécution native possible tant que les dépendances système manquent.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Add the Tauri Linux reader`.

## 2026-08-08 — Validation native et paquets Linux de l'étape 8

### Objectif

Clore la validation Tauri après installation des prérequis Ubuntu et produire
les deux formats installables retenus.

### Actions

- ajout d'une icône Reader déterministe en SVG et PNG, puis déclaration
  explicite de l'icône dans la configuration du bundle ;
- compilation et exécution des tests du workspace complet ;
- lancement réel de Vite et du binaire Tauri avec création de la base AppData ;
- isolation des variables GTK/GIO injectées par le snap VS Code pendant le
  smoke test afin d'utiliser exclusivement les bibliothèques Ubuntu ;
- production d'un paquet Debian et d'une AppImage x86-64 ;
- clôture de la dernière case de validation de l'étape 8 dans le TODO et ajout
  du dépannage snap au README.

### Justification

Le SVG conserve une source éditable et le PNG fournit l'asset carré exigé par
Tauri et AppImage. Le smoke test avec l'environnement GTK système correspond au
lancement normal depuis un terminal ou depuis un paquet installé.

### Vérifications

- `cargo check --workspace` : réussi ;
- `cargo test --workspace` : 103 tests réussis, aucun échec ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  aucun avertissement ;
- `cargo fmt --check` : formatage conforme ;
- `npm run tauri dev` : Vite et le binaire natif démarrés, base créée dans
  `~/.local/share/io.github.r0m1.reader/reader.db` ;
- `npm run tauri build -- --bundles deb` : paquet Debian produit ;
- `npm run tauri build -- --bundles appimage` : AppImage produite ;
- AppImage lancée avec extraction temporaire : application restée active sans
  erreur jusqu'à son arrêt volontaire ;
- `file`, `dpkg-deb --info` et `sha256sum` : formats, métadonnées et artefacts
  contrôlés.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Add the Tauri Linux reader`.

## 2026-08-10 — Registre des retours du premier test utilisateur

### Objectif

Conserver les demandes fonctionnelles observées pendant les premiers essais de
l'interface Linux avant de poursuivre vers Android.

### Actions

- création de `FEATURE_REQUESTS.md` avec des demandes identifiables et leur
  comportement attendu ;
- enregistrement de la suppression d'abonnement, du marquage lu/non lu, de
  l'ouverture externe des liens et du détail des erreurs de rafraîchissement ;
- ajout de la conversion des URL de profil Medium, issue du test précédent ;
- ajout dans le TODO d'une étape intermédiaire renvoyant vers ce registre.

### Justification

Un registre séparé permet de préciser les besoins et les décisions encore
ouvertes sans transformer la feuille de route principale en spécification
détaillée. La suppression reste volontairement indécise quant au devenir des
articles associés.

### Vérifications

- relecture du document et de son lien depuis `TODO.md` effectuée ;
- aucun test exécuté, car cette intervention modifie uniquement la
  documentation.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Implement RSS feed deletion`.

## 2026-08-10 — Alignement de la branche principale

### Objectif

Préparer la branche principale destinée au premier push GitHub.

### Actions

- commit de la licence MIT et des README bilingues ;
- vérification que `main` est un ancêtre de `dev` ;
- avancement en fast-forward de la référence locale `main` jusqu'à `dev`, sans
  commit de fusion ;
- maintien de `dev` comme branche de travail courante.

### Justification

La branche publiée par défaut doit contenir l'état complet et validé du projet,
tandis que la conservation de `dev` permet de poursuivre le développement.

### Vérifications

- `main` et `dev` pointent toutes deux sur le même commit ;
- arbre de travail Git propre ;
- aucun remote configuré et aucun push effectué.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `fa443b9` (`[AI] Add MIT license and bilingual
documentation`) avant alignement des branches ; ce rapport reste hors de Git.

## 2026-08-10 — Préparation des fichiers de travail privés

### Objectif

Conserver `AGENTS.md` et `codex_report.md` uniquement dans l'environnement local
et les retirer complètement de l'historique destiné à GitHub.

### Actions

- ajout de `AGENTS.md`, `codex_report.md`, `script.sh` et `script.sh~` aux
  exclusions Git locales au projet ;
- conservation de copies locales des deux documents de travail pendant la
  réécriture ;
- réécriture de toutes les branches et suppression des références, reflogs et
  anciens objets Git contenant ces fichiers.

### Justification

Ces documents servent au pilotage local et ne font pas partie du produit
présenté publiquement. Une simple suppression dans le dernier commit les
laisserait consultables dans l'historique GitHub.

### Vérifications

- `git log --all -- AGENTS.md codex_report.md` : aucun commit trouvé ;
- recherche des deux chemins dans les objets des branches et tags : aucun
  résultat ;
- `git ls-files AGENTS.md codex_report.md script.sh script.sh~` : aucun fichier
  suivi ;
- aucune référence `refs/original`, aucun objet inaccessible signalé par
  `git fsck --no-reflogs --unreachable` et arbre de travail propre ;
- les quatre fichiers locaux apparaissent bien comme ignorés.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : la règle d'exclusion sera incluse dans le commit marqué IA
`[AI] Keep local working files private` ; ce rapport restera volontairement
hors de Git.

## 2026-08-10 — Anonymisation des métadonnées publiques

### Objectif

Remplacer les dernières données personnelles ou spécifiques dans les fichiers
publics et dans les métadonnées Git avant la publication sur GitHub.

### Actions

- remplacement du flux Substack réel utilisé comme exemple par une URL
  générique et vérification des exemples Medium ;
- harmonisation du bundle Tauri et de sa documentation avec le pseudonyme
  GitHub `r0m1-b` ;
- configuration locale de Git avec le pseudonyme GitHub et son adresse
  `noreply` ;
- réécriture des auteurs, auteurs de commit et anciennes occurrences présentes
  dans toutes les branches, y compris l'URL du tout premier prototype ;
- suppression des références de sauvegarde, expiration des reflogs et purge des
  objets Git devenus inaccessibles.

### Justification

L'adresse `noreply` associe les commits au compte GitHub sans publier l'adresse
Proton personnelle. Réécrire l'historique est nécessaire car une modification
limitée au dernier commit laisserait les anciennes métadonnées consultables.

### Vérifications

- `npm run typecheck` et `npm test` : succès, 14 tests frontend ;
- `npm run tauri build -- --debug --no-bundle` : succès avec le nouvel
  identifiant ;
- tous les auteurs et committers des branches et tags utilisent `r0m1-b` et
  l'adresse GitHub `noreply` ;
- aucune occurrence de l'adresse Proton, de `kosmotheos` ou de l'ancien bundle
  dans les fichiers ou métadonnées de l'historique publiable ;
- aucune référence `refs/original`, aucun objet inaccessible signalé et arbre
  de travail propre ;
- configuration Git locale relue et conforme.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : le commit intermédiaire marqué IA a été absorbé par la
réécriture, car ses changements sont désormais présents depuis leur première
apparition dans l'historique ; ce rapport reste hors de Git.

## 2026-08-10 — Documents de planification conservés localement

### Objectif

Retirer les documents de planification de la future vue GitHub sans perdre les
copies de travail locales ni réécrire leur ancien historique.

### Actions

- restauration de `TODO.md` et `FEATURE_REQUESTS.md` après leur suppression
  locale préalable ;
- retrait des deux fichiers de l'index avec `git rm --cached` ;
- ajout des chemins au `.gitignore` ;
- suppression du lien vers `TODO.md` dans le README public.

### Justification

Le retrait de l'index, contrairement à `git rm` seul, conserve les documents
sur la machine. Leur contenu n'étant pas sensible, une purge de l'historique
n'est pas nécessaire.

### Vérifications

- présence des deux fichiers sur le disque et absence dans `git ls-files` ;
- reconnaissance des deux chemins par `git check-ignore` ;
- contrôle du diff Git à effectuer avant commit.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun commit créé à ce stade ; ce rapport reste hors de Git.

## 2026-08-10 — README bilingue

### Objectif

Présenter le projet en anglais sur GitHub tout en conservant la documentation
française complète.

### Actions

- renommage de la version française en `README.fr.md` ;
- création d'un `README.md` anglais traduisant l'intégralité des sections,
  commandes, avertissements SQLite, limites et procédures de dépannage ;
- ajout d'un sélecteur English/Français réciproque en tête des deux fichiers ;
- maintien de l'anglais comme README déclaré dans les métadonnées Cargo.

### Justification

GitHub affiche automatiquement `README.md`. Une version anglaise principale
rend le projet accessible à l'écosystème Rust et Tauri sans retirer la
documentation française existante.

### Vérifications

- les deux README possèdent 46 délimiteurs de blocs de code et des structures
  de sections correspondantes ;
- les liens de langue et de licence pointent vers des fichiers présents ;
- recherche de texte français résiduel dans `README.md` : seul le libellé du
  sélecteur « Français » est attendu ;
- `cargo metadata --no-deps --format-version 1` référence bien `README.md` ;
- `git diff --check` : succès ;
- aucun test applicatif supplémentaire exécuté, cette étape ne modifiant que la
  documentation ; les validations Cargo de la licence avaient déjà réussi.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : changements inclus dans le commit marqué IA
`[AI] Add MIT license and bilingual documentation` ; ce rapport reste hors de
Git.

## 2026-08-10 — Adoption de la licence MIT

### Objectif

Définir explicitement les conditions de réutilisation du projet avant sa
publication sur GitHub.

### Actions

- ajout du texte officiel de la licence MIT dans `LICENSE`, avec le titulaire
  public `r0m1-b` et l'année 2026 ;
- ajout de l'identifiant SPDX `MIT` aux manifestes Cargo du cœur et de
  l'application Tauri ainsi qu'au paquet frontend ;
- déclaration du README dans les métadonnées du crate principal ;
- ajout d'une section Licence au README.

### Justification

La licence MIT correspond au choix explicite du propriétaire et autorise une
réutilisation permissive tout en imposant la conservation de la notice et en
excluant les garanties.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo metadata --no-deps --format-version 1` : les deux paquets déclarent
  `MIT` et le crate principal référence `README.md` ;
- `npm pkg get license` : retourne `MIT` ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : changements inclus dans le commit marqué IA
`[AI] Add MIT license and bilingual documentation` ; ce rapport reste hors de
Git.

## 2026-08-10 — Politique de suppression d'un abonnement

### Objectif

Lever l'ambiguïté sur le devenir des données lors de la suppression définitive
d'un flux.

### Actions

- décision d'effacer avec le flux tous ses articles, favoris et états de
  lecture ;
- ajout d'une confirmation utilisateur décrivant cette perte de données ;
- exigence d'une suppression transactionnelle et de la mise à jour immédiate
  de la chronologie et du panneau de lecture.

### Justification

Cette politique correspond à une suppression réelle, distincte de la
désactivation qui reste disponible pour conserver l'historique.

### Vérifications

- relecture de FR-001 effectuée ;
- aucun test exécuté, car cette intervention modifie uniquement la
  documentation.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun commit créé à ce stade.

## 2026-08-10 — Consultation des favoris

### Objectif

Conserver la demande d'un espace dédié aux articles favoris sans la placer dans
le chemin critique des prochains correctifs.

### Actions

- ajout de FR-006 avec une priorité basse ;
- description de la liste, de l'état vide, de la lecture et de la mise à jour
  immédiate des favoris ;
- maintien du choix onglet, page ou filtre comme décision produit ultérieure.

### Justification

Le besoin est stable, mais sa forme de navigation doit rester compatible avec
la future interface Android et n'a pas à retarder les retours prioritaires.

### Vérifications

- relecture de FR-006 effectuée ;
- aucun test exécuté, car cette intervention modifie uniquement la
  documentation.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun commit créé à ce stade.

## 2026-08-10 — FR-001 : suppression définitive d'un abonnement

### Objectif

Permettre de supprimer un abonnement depuis l'application Linux avec tous ses
articles et leurs états locaux, tout en conservant la désactivation comme
opération non destructive.

### Actions

- ajout de `Storage::delete_feed`, qui vérifie l'abonnement puis supprime ses
  articles et le flux dans une transaction SQLite ;
- conservation de la contrainte `ON DELETE RESTRICT` comme garde-fou et ajout
  d'un résultat indiquant le nombre d'articles supprimés ;
- exposition de la commande Tauri `delete_feed` avec DTO camelCase et erreurs
  structurées ;
- ajout de l'action destructive dans la gestion des abonnements, avec
  confirmation explicite, prévention des doubles actions, mise à jour de la
  chronologie et fermeture du panneau de lecture concerné ;
- ajout des types et de l'appel API TypeScript, du style visuel et des tests
  Rust, Tauri et Vitest couvrant succès, annulation, erreur et rollback ;
- passage de FR-001 à l'état `terminée` et documentation de la différence entre
  désactivation et suppression dans le README et le TODO.

### Justification

La suppression explicite des articles avant le flux évite une migration de
schéma et permet de compter les articles effacés. La transaction garantit
qu'une erreur ne laisse pas une suppression partielle ; un test avec un trigger
SQLite en échec vérifie directement ce rollback.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 14 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel Tauri exécuté dans cette intervention automatisée.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : cette entrée est incluse dans le commit marqué IA
`[AI] Implement RSS feed deletion`.

## 2026-08-10 — Renommage complet en InkRiver

### Objectif

Adopter le nom public InkRiver avant la première publication GitHub et éviter
une divergence entre la marque, les binaires et les identifiants techniques.

### Actions

- renommage du crate et du binaire CLI en `inkriver`, du paquet Tauri en
  `inkriver-app` et de sa bibliothèque en `inkriver_app_lib` ;
- mise à jour des imports Rust, tests d'intégration, noms frontend et textes de
  l'interface ;
- adoption du bundle `io.github.r0m1-b.inkriver` et de `inkriver.db` pour le CLI
  et l'application installée ;
- ajout des nouveaux fichiers SQLite au `.gitignore` sans retirer les anciens,
  afin de ne pas exposer une base locale historique ;
- mise à jour des README bilingues et des documents de planification locaux ;
- remplacement du monogramme `R` par `IR` dans l'icône SVG, le PNG dérivé et
  l'interface.

### Justification

Un renommage complet avant la première release évite de figer l'ancien bundle
et les anciens noms de paquets. Aucune migration automatique des bases de
développement existantes n'est ajoutée à ce stade.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 14 tests frontend ;
- `npm run build` : succès ;
- `npm run tauri build -- --debug --no-bundle` : succès, binaire
  `target/debug/inkriver-app` produit ;
- audit des anciens identifiants techniques : seules les règles d'exclusion de
  l'ancienne base locale subsistent volontairement ;
- inspection visuelle de l'icône PNG régénérée : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `73b3666` (`[AI] Rename project to InkRiver`) ; ce rapport
reste hors de Git.

## 2026-08-10 — Flux de branches et chantiers de présentation GitHub

### Objectif

Préserver une branche `main` stable pour les releases tout en poursuivant le
développement sur `dev`, et consigner les améliorations nécessaires à une
présentation GitHub plus professionnelle.

### Actions

- bascule sur la branche locale `dev` puis avancement en fast-forward jusqu'à
  la version publiée sur `main` ;
- documentation dans les README bilingues du rôle de `dev`, de `main` et des
  éventuelles branches temporaires ;
- ajout au TODO local des captures d'écran et d'une CI GitHub Actions couvrant
  Rust, Clippy et le frontend.

### Justification

Ce flux réduit les manipulations quotidiennes tout en maintenant une séparation
claire entre intégration et versions figées. Les contrôles CI proposés reprennent
les validations déjà utilisées localement.

### Vérifications

- `dev` avancée en fast-forward sans divergence ni commit de fusion ;
- contrôle documentaire des README et du TODO : succès ;
- `git diff --check` : succès ;
- aucun test applicatif exécuté, les changements suivis étant exclusivement
  documentaires.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `e01431a` (`[AI] Document the development branch workflow`) ;
ce rapport reste hors de Git.

## 2026-08-12 — FR-002 : état de lecture explicite

### Objectif

Permettre de corriger depuis le panneau de lecture l'état lu ou non lu d'un
article, en particulier après son marquage automatique à l'ouverture.

### Actions

- ajout de l'état courant et de l'action inverse dans le panneau de lecture ;
- blocage de l'action pendant l'écriture et conservation de l'état précédent en
  cas d'échec ;
- mise à jour coordonnée du détail sélectionné et de la chronologie après
  confirmation de SQLite ;
- extension du test Tauri aux transitions lu puis non lu ;
- ajout de tests frontend pour le marquage automatique, les deux transitions,
  l'absence d'écriture inutile, l'attente et l'erreur ;
- documentation du comportement dans les README et passage de FR-002 à l'état
  terminé dans les documents locaux.

### Justification

La commande Tauri et le stockage acceptaient déjà les deux valeurs. La nouvelle
action réutilise cette interface et n'applique l'état en mémoire qu'après une
écriture réussie, afin que l'affichage ne diverge pas de SQLite.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 18 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- test manuel de persistance après relance non exécuté dans cette intervention
  automatisée.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `a8b6e46` (`[AI] Add explicit read state controls`) ; ce
rapport reste hors de Git.

## 2026-08-12 — Actions rapides dans la chronologie

### Objectif

Permettre de modifier les états favori et lu directement depuis la liste des
articles, sans ouvrir leur contenu.

### Actions

- remplacement de la ligne-bouton par un bouton de sélection et deux boutons
  d'action frères, afin d'éviter les boutons HTML imbriqués ;
- ajout d'une étoile et d'une enveloppe SVG toujours visibles, représentant
  l'état courant et accompagnées de libellés accessibles ;
- mutualisation des écritures favori et lu entre la chronologie et le panneau
  de lecture ;
- suivi indépendant des écritures par article et par action, avec blocage des
  doubles clics et conservation de l'état en cas d'erreur ;
- conservation d'un article ouvert remis à non lu jusqu'à sa réouverture après
  consultation d'un autre article ;
- ajout de tests frontend pour les états, actions inverses, synchronisation,
  concurrence, erreurs et absence d'ouverture ;
- mise à jour des README et des documents locaux FR-002/TODO.

### Justification

Les commandes Tauri existantes suffisent. Attendre la confirmation de SQLite
avant de modifier l'affichage évite toute divergence, tandis que les ensembles
d'identifiants en cours d'écriture n'immobilisent pas le reste de la liste.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 24 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'interface Tauri exécuté dans cette intervention
  automatisée.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `6e129ab` (`[AI] Add timeline article state actions`) ; ce
rapport reste hors de Git.

## 2026-08-12 — Conservation du défilement de la chronologie

### Objectif

Éviter le retour en haut de la liste lors des rendus de l'interface et maintenir
la ligne de l'article nouvellement ouvert dans la zone visible.

### Actions

- mémorisation du `scrollTop` de la chronologie avant chaque reconstruction du
  DOM puis restauration sur le nouvel élément ;
- défilement de la nouvelle sélection avec `scrollIntoView` et l'alignement
  `nearest`, uniquement après une ouverture effective ;
- ajout de tests vérifiant la conservation de la position pendant une action
  asynchrone et le positionnement de l'article sélectionné.

### Justification

Le rendu remplace entièrement le contenu de la racine et recréait donc l'élément
scrollable à la position zéro. Restaurer sa position traite tous les boutons,
tandis que le défilement ciblé reste réservé au changement de sélection afin de
ne pas perturber une consultation libre de la liste.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 26 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- comportement validé manuellement par l'utilisateur dans l'interface Tauri.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `571806e` (`[AI] Preserve timeline scroll position`) ; ce
rapport reste hors de Git.

## 2026-08-12 — FR-003 : ouverture externe des liens d'article

### Objectif

Empêcher les liens du contenu de remplacer l'article dans son iframe et les
ouvrir de manière fiable dans le navigateur système.

### Actions

- neutralisation avant rendu de tous les liens dans le `srcdoc`, avec délégation
  des liens externes et tentative de gestion locale des ancres internes ;
- résolution des liens relatifs par rapport à l'URL de l'article et ouverture
  des seules URL HTTP(S) avec le plugin Tauri `opener` ;
- interception déléguée des clics dans l'iframe, y compris sur un élément enfant
  d'une ancre, avec affichage des erreurs sans perdre l'article ;
- maintien du sandbox sans scripts, ajout d'une CSP restrictive et autorisation
  `allow-same-origin` limitée à l'interception par le parent ;
- réduction de la capability Tauri aux URL HTTP(S), sans autorisation de chemin,
  `mailto:` ou `tel:` ;
- ajout de tests frontend pour les liens absolus, relatifs, fragments, protocoles
  refusés, échec du navigateur et préparation sécurisée du document ;
- correction après le premier essai manuel du gestionnaire `load`, qui pouvait
  être consommé par le document vide précédant le `srcdoc` et laisser naviguer
  l'iframe vers une page blanche ;
- remplacement après le second essai manuel de ce branchement dépendant du
  chargement par une écriture synchrone du document isolé, suivie de
  l'installation directe du gestionnaire sur ce document ;
- neutralisation des ancres internes sans navigation native de l'iframe ; leur
  défilement programmatique reste inopérant sous WebKitGTK et est consigné comme
  travail ultérieur ;
- abandon après le troisième essai manuel de l'accès au DOM de l'iframe, non
  fiable dans WebKitGTK, au profit d'un script interne protégé par nonce et
  d'un pont `postMessage` dont la source est vérifiée par le parent ;
- retour à une origine opaque avec `sandbox="allow-scripts"`, sans
  `allow-same-origin`, le script de confiance ne pouvant transmettre que les
  clics et gérer les ancres du document isolé ;
- passage de FR-003 à l'état terminé et mise à jour des README et du TODO local.

### Justification

Le navigateur système possède les sessions et capacités nécessaires aux pages
Medium/Substack, contrairement à l'iframe sandboxée. La neutralisation des
liens avant le chargement évite toute navigation même avant l'installation du
gestionnaire de clics.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 32 tests frontend ;
- `npm run build` : succès ;
- `npm run tauri build -- --debug --no-bundle` : succès ;
- `git diff --check` : succès ;
- test manuel dans l'application Tauri : les liens externes s'ouvrent dans le
  navigateur sans remplacer l'article ; les ancres internes restent sans effet.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `7931ca4` (`[AI] Open article links in the system browser`) ;
ce rapport reste hors de Git.

## 2026-08-12 — Clôture fonctionnelle de FR-003

### Objectif

Considérer l'ouverture externe des liens comme terminée malgré l'absence de
défilement des ancres internes sous WebKitGTK.

### Actions

- suppression du reliquat concernant les ancres internes dans le TODO local ;
- documentation dans FR-003 de cette limite comme compromis accepté, à
  réévaluer uniquement si l'usage révèle un cas gênant.

### Justification

Les ancres internes servent principalement à atteindre des notes dans le même
article et ne bloquent pas le besoin principal : ouvrir les contenus externes,
notamment payants, dans le navigateur système.

### Vérifications

- contrôle des documents locaux : FR-003 reste marquée terminée et aucun travail
  obligatoire sur les ancres internes ne subsiste ;
- aucun test exécuté, cette intervention ne modifie pas le code suivi.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun ; les documents concernés restent hors de Git.

## 2026-08-12 — FR-006 : consultation des articles favoris

### Objectif

Permettre de retrouver et de lire les articles favoris sans quitter l'interface
principale ni effectuer d'accès réseau.

### Actions

- ajout des onglets `Tous` et `Favoris` au-dessus de la chronologie, avec le
  nombre courant de favoris ;
- filtrage du cache d'articles déjà chargé depuis SQLite, en conservant le
  panneau de lecture et l'ordre chronologique fourni par le cœur Rust ;
- ajout d'un état vide explicite et synchronisation immédiate après l'ajout ou
  le retrait d'une étoile ;
- ajout de trois tests frontend couvrant le filtrage et l'ouverture hors ligne,
  l'état vide et les transitions immédiates depuis le panneau de lecture ;
- mise à jour des README bilingues, du TODO et de la fiche FR-006 locale.

### Justification

Des onglets dans la colonne existante évitent une nouvelle route et réutilisent
le même panneau de lecture. Ce modèle compact pourra être repris dans la future
navigation Android. Aucun nouveau point d'API n'est nécessaire puisque les
résumés SQLite contiennent déjà l'état favori.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 35 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté à ce stade.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `9b2dba0` (`[AI] Add the favorites article view`) ; ce rapport
reste hors de Git.

## 2026-08-12 — Icônes des sources dans les articles

### Objectif

Rendre la provenance des articles identifiable visuellement dans la chronologie
et le panneau de lecture.

### Actions

- ajout de pictogrammes SVG intégrés pour Medium, Substack et les flux RSS
  génériques dans les badges de source ;
- conservation systématique du libellé texte et masquage des SVG décoratifs aux
  technologies d'assistance ;
- adaptation du style des badges aux thèmes clair et sombre existants ;
- ajout d'un test frontend couvrant les trois pictogrammes, leurs libellés et
  leur attribut d'accessibilité ;
- mise à jour des README bilingues.

### Justification

Des SVG intégrés restent nets à toutes les résolutions, héritent des couleurs du
badge et n'ajoutent aucune dépendance ni requête de ressource. Le texte évite de
faire reposer l'identification de la source sur l'icône seule.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 36 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté à ce stade.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `a24d186` (`[AI] Add recognizable source icons`) ; ce rapport
reste hors de Git.

## 2026-08-12 — Ajustement visuel des icônes de source

### Objectif

Améliorer la lisibilité des pictogrammes Medium, Substack et RSS après le
premier essai visuel.

### Actions

- séparation du pictogramme et de la pastille contenant le libellé ;
- passage du dessin à 20 px dans une zone autonome de 22 px ;
- ajout de couleurs propres aux plateformes dans les thèmes clair et sombre ;
- renforcement du test frontend pour vérifier que l'icône ne fait plus partie
  de la pastille texte.

### Justification

La zone autonome évite la compression du pictogramme et crée une séparation
visuelle nette, tout en conservant le libellé demandé.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 36 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté par Codex.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `a24d186` (`[AI] Add recognizable source icons`) ; ce rapport
reste hors de Git.

## 2026-08-12 — Remplacement par les marques Medium et Substack

### Objectif

Remplacer les pictogrammes évocateurs par les formes de marque reconnaissables
de Medium et Substack.

### Actions

- vérification des pages officielles de marque Medium et Substack ;
- récupération des tracés SVG Medium et Substack depuis Simple Icons v16.21.0,
  version épinglée, puis intégration directe dans le bundle ;
- application du noir ou blanc à la marque Medium selon le thème et conservation
  de l'orange `#ff6719` de Substack ;
- maintien de l'icône générique pour les autres flux RSS et des libellés séparés ;
- renforcement du test frontend pour identifier précisément les deux tracés ;
- documentation de la provenance et de la propriété des marques dans les README.

### Justification

Les formes de marque sont plus immédiatement reconnaissables que les dessins
maison. Leur intégration locale évite une dépendance d'exécution et conserve le
fonctionnement hors ligne.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 108 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 36 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté par Codex.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `a24d186` (`[AI] Add recognizable source icons`) ; ce rapport
reste hors de Git.

## 2026-08-12 — Abandon de FR-005

### Objectif

Retirer la conversion automatique des URL de profil Medium du périmètre prévu.

### Actions

- suppression complète de la fiche FR-005 du registre local des demandes ;
- retrait de la conversion des URL Medium du TODO local ;
- clôture de la rubrique des premiers retours utilisateurs, les demandes encore
  conservées étant terminées ;
- ajout explicite de FR-004 à la liste récapitulative terminée.

### Justification

La saisie directe d'une URL de flux Medium reste suffisante pour le projet et
ne justifie pas la complexité supplémentaire de reconnaissance et conversion
des différentes formes d'URL de profil.

### Vérifications

- recherche des références actives à FR-005 et aux URL de profil Medium : aucune
  référence restante hors historique du rapport ;
- aucun test exécuté, cette intervention ne modifie que des documents locaux de
  suivi ignorés par Git.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun ; les documents concernés restent hors de Git.

## 2026-08-12 — FR-004 : page de gestion et erreurs persistantes

### Objectif

Séparer l'ajout d'un abonnement de son administration et rendre les erreurs de
rafraîchissement détaillées, durables et consultables par flux.

### Actions

- ajout d'une migration SQLite stockant le titre, la description, l'auteur, la
  date du dernier succès et la dernière erreur de chaque flux ;
- conservation des métadonnées de flux dans le rapport de collecte, conversion
  des descriptions HTML en texte lisible et enregistrement des succès et échecs
  après chaque rafraîchissement ;
- calcul de la dernière publication depuis les articles en cache et repli vers
  l'auteur de l'article le plus récent lorsque le flux ne déclare aucun auteur ;
- effacement de l'erreur précédente lors du prochain succès du flux, sans perdre
  l'historique en cas de nouvel échec ;
- extension des modèles de stockage, DTO Tauri et types TypeScript avec les
  métadonnées et l'erreur structurée horodatée ;
- création d'une page principale `Abonnements` affichant les détails, les états
  actif/inactif, les erreurs et les actions de désactivation ou suppression ;
- réduction de la fenêtre modale au seul formulaire d'ajout et ajout d'une
  navigation `Articles` / `Abonnements` ;
- rechargement des abonnements après chaque actualisation afin d'afficher
  immédiatement le dernier état enregistré ;
- mise à jour des README bilingues, de FR-004 et du TODO local.

### Justification

SQLite doit être la source de vérité de l'application installée : conserver le
dernier résultat par abonnement rend le diagnostic disponible hors ligne et
après redémarrage. Une page dédiée offre assez d'espace pour les métadonnées et
se transpose naturellement en écran Android, tandis que la fenêtre d'ajout reste
courte et ciblée.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 111 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 38 tests frontend ;
- `npm run build` : succès ;
- `npm run tauri build -- --debug --no-bundle` : succès, binaire Tauri créé ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté par Codex.

Les tests ajoutés couvrent les métadonnées RSS/Atom, la migration, la
persistance après réouverture de SQLite, l'effacement d'une erreur après succès,
les DTO, la page de gestion, la fenêtre d'ajout isolée et le rechargement après
une actualisation partielle.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `8501103` (`[AI] Add subscription management details`) ; ce
rapport reste hors de Git.

## 2026-08-12 — Ajout des demandes de suppression d'articles

### Objectif

Conserver dans la feuille de route les deux besoins de gestion de la durée de
vie des articles.

### Actions

- ajout de la suppression manuelle d'un article aux évolutions futures ;
- ajout d'une politique configurable de suppression des articles lus après une
  durée déterminée ;
- signalement explicite du traitement des favoris comme décision à prendre lors
  de la conception de la suppression automatique.

### Justification

La suppression manuelle et la rétention automatique répondent à des usages
différents et doivent rester deux fonctionnalités indépendantes. Les favoris
méritent une règle explicite pour éviter une perte de contenu inattendue.

### Vérifications

- contrôle visuel de la section `Évolutions ultérieures` du TODO ;
- aucun test exécuté, cette intervention ne modifie que des documents locaux de
  suivi ignorés par Git.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun ; les documents concernés restent hors de Git.

## 2026-08-14 — Ajout de l'archivage et du lien source au backlog

### Objectif

Enregistrer deux nouvelles demandes concernant l'organisation de la
chronologie et l'accès à la publication d'origine.

### Actions

- ajout de l'archivage d'un article au TODO et création de FR-007 ;
- distinction explicite entre archivage et suppression, l'article archivé
  restant conservé dans SQLite ;
- ajout d'un lien source toujours visible au TODO et création de FR-008 ;
- consignation des décisions encore ouvertes pour l'archivage et des règles de
  sécurité attendues pour l'ouverture du lien source.

### Justification

Des fiches séparées permettent de préciser ultérieurement l'expérience
d'archivage sans la confondre avec la suppression, tandis que le lien source
peut réutiliser le mécanisme sécurisé d'ouverture dans le navigateur déjà mis
en place pour FR-003.

### Vérifications

- contrôle visuel des nouvelles entrées du TODO et du registre des demandes ;
- aucun test exécuté, cette intervention ne modifie que des documents locaux de
  suivi ignorés par Git.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun ; les documents concernés restent hors de Git.

## 2026-08-14 — Création du logo master InkRiver

### Objectif

Transformer la planche `logo_raw.png` fournie par l'utilisateur en un logo
d'application définitif, lisible sur Linux, Android et GitHub.

### Actions

- sélection de la variante carrée bleu nuit comme direction principale ;
- génération assistée par IA d'un symbole simplifié réunissant une feuille, un
  signal RSS orange et une rivière blanche ;
- suppression locale du fond chroma et production d'un PNG avec transparence ;
- enregistrement non destructif du master dans
  `branding/inkriver-logo.png`, sans remplacer l'icône Tauri existante.

### Justification

La variante carrée offre la meilleure lisibilité aux petites tailles et peut
servir de source commune aux icônes desktop et mobiles. L'asset est conservé
séparément afin de permettre sa validation avant de décliner les tailles et de
l'intégrer à l'application.

### Vérifications

- inspection visuelle de la planche source et du résultat sur fond clair ;
- validation du fichier : PNG RGBA de 1254 × 1254 pixels ;
- validation de la transparence : coins transparents et 763 542 pixels
  transparents sur 1 572 516 ;
- aucun test logiciel exécuté, cette intervention ajoute uniquement un asset
  graphique sans modifier le code.

Ce logo a été créé avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Intégration du logo dans l'application

### Objectif

Rendre le logo InkRiver validé visible dans l'interface et l'utiliser comme
icône native de l'application.

### Actions

- remplacement de l'ancienne icône Tauri « IR » par une déclinaison 1024 px du
  master validé ;
- ajout d'une déclinaison web 256 px dans `app/public` ;
- affichage du logo dans le bandeau principal et déclaration comme favicon ;
- suppression de l'ancien SVG « IR » devenu obsolète ;
- ajout d'un test frontend vérifiant la présence et le caractère décoratif du
  logo dans l'en-tête.

### Justification

Un master séparé conserve la meilleure définition, tandis que les copies
optimisées répondent aux besoins respectifs de Tauri et de l'interface. Le texte
« InkRiver » adjacent fournit déjà le nom accessible, donc l'image du bandeau
utilise un texte alternatif vide pour éviter une répétition aux lecteurs
d'écran.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 39 tests frontend ;
- `npm run build` : succès ;
- `npm run tauri build -- --debug --no-bundle` : succès, binaire natif créé ;
- aucun test visuel manuel exécuté par Codex.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Ajustement du logo et du slogan

### Objectif

Améliorer la présence visuelle de la nouvelle identité dans le bandeau de
l'application.

### Actions

- augmentation exacte de 10 % du logo dans l'en-tête, de 44 à 48,4 pixels ;
- remplacement de « Medium + Substack » par
  « All your feeds. One flow. » ;
- extension du test du bandeau pour vérifier le slogan affiché.

### Justification

Le logo gagnait à occuper davantage l'espace disponible dans le bandeau. Le
slogan retenu décrit aussi mieux l'ambition multi-source d'InkRiver.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 39 tests frontend ;
- `npm run build` : succès.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Essai du logo à 64 pixels

### Objectif

Donner davantage de présence au logo dans le bandeau de l'application.

### Actions

- passage de la largeur et de la hauteur du logo de 48,4 à 64 pixels ;
- conservation de la hauteur actuelle du bandeau pour évaluer cette taille sans
  modifier le reste de la mise en page.

### Vérifications

- `npm run build` : succès.

Ce changement a été créé avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Commit de l'identité visuelle InkRiver

### Objectif

Versionner le logo validé et son intégration à l'application sur la branche
`dev`.

### Actions

- ajout du master et de la déclinaison web du logo ;
- remplacement de l'icône Tauri et suppression de l'ancien SVG « IR » ;
- inclusion du bandeau à 64 pixels, du slogan et du test frontend associé ;
- exclusion volontaire de `logo_raw.png`, conservé comme planche de travail
  locale.

### Vérifications

- `git diff --cached --check` : succès avant commit ;
- arbre suivi propre après commit ; seul `logo_raw.png` reste non suivi.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `c420ec5` (`[AI] Add InkRiver branding`) ; ce rapport reste
hors de Git.

## 2026-08-14 — FR-008 : lien vers la source de l'article

### Objectif

Rendre la page d'origine identifiable et accessible depuis tout article, même
si le flux fournit son contenu complet.

### Actions

- ajout d'une ligne permanente `Source` dans les métadonnées du panneau de
  lecture ;
- affichage du domaine et du port pour les URL HTTP(S), avec l'URL complète en
  infobulle et une ouverture par le plugin Tauri existant ;
- ajout d'états non interactifs pour les URL absentes ou non prises en charge ;
- conservation du bouton `Lire l'original` uniquement pour les extraits et les
  contenus manquants ou hérités ;
- ajout des styles et de quatre scénarios d'interface, ainsi que d'un test du
  helper de validation ;
- mise à jour des README bilingues, de FR-008 et du TODO local.

### Justification

Les données et la commande d'ouverture existaient déjà : une évolution limitée
au frontend évite toute migration ou modification d'API. La même validation
HTTP(S) protège le lien source et le bouton existant, tandis que le domaine rend
la destination visible avant l'action.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 43 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test visuel manuel exécuté par Codex.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `2d80227` (`[AI] Add article source links`) ; ce rapport reste
hors de Git.

## 2026-08-14 — Grand logo dans l'état vide du lecteur

### Objectif

Remplacer l'ancien monogramme « IR » affiché sans article sélectionné par le
verrou complet InkRiver présent dans la planche de marque.

### Actions

- extraction déterministe du grand logo, du nom InkRiver et de son slogan à
  partir de `logo_raw.png`, sans régénération ni altération du dessin ;
- ajout de l'asset 650 × 500 dans `app/public/inkriver-wordmark.png` ;
- remplacement du monogramme par cette image dans l'état vide du lecteur ;
- dimensionnement responsive jusqu'à 460 pixels, avec fond clair lisible dans
  les thèmes clair et sombre ;
- ajout d'un test frontend vérifiant l'asset, son texte alternatif et le message
  d'invitation à sélectionner un article.

### Justification

Un recadrage direct préserve fidèlement le mot-symbole et évite les variations
de texte qu'introduirait une nouvelle génération. Le fond clair de la planche
garantit la lisibilité du bleu nuit dans les deux thèmes.

### Vérifications

- inspection visuelle du recadrage ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 44 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été réalisés avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Bouton d'actualisation à icône

### Objectif

Alléger visuellement le bandeau en remplaçant le libellé permanent du bouton
d'actualisation par une icône explicite.

### Actions

- remplacement du texte visible par une icône SVG de rafraîchissement ;
- ajout d'une infobulle native `Actualiser` et d'un libellé accessible ;
- passage du libellé accessible à `Actualisation en cours` pendant le chargement ;
- animation de rotation de l'icône pendant l'opération, désactivée lorsque
  l'utilisateur préfère réduire les animations ;
- ajout d'un test vérifiant l'absence de texte visible, l'infobulle, les
  attributs ARIA et la présence du SVG.

### Justification

Une action compacte libère de l'espace dans le bandeau sans sacrifier la
compréhension au survol ni l'accessibilité au clavier et aux lecteurs d'écran.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 45 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Actions de lecture sous forme d'icônes

### Objectif

Alléger le panneau de lecture en remplaçant les boutons textuels de lecture et
de favori par les mêmes repères visuels que dans la chronologie.

### Actions

- remplacement de `Marquer comme lu/non lu` par une icône enveloppe ;
- remplacement de `Ajouter/Retirer des favoris` par une icône étoile ;
- conservation de l'état courant par la forme, la couleur et `aria-pressed` ;
- déplacement des libellés d'action dans les infobulles et `aria-label` ;
- adaptation des tests de lecture, de favori et d'enregistrement en cours aux
  boutons sans texte visible.

### Justification

Réutiliser les icônes de la chronologie rend l'interface plus compacte et
cohérente, tout en conservant une explication au survol et une information
complète pour les technologies d'assistance.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 45 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade.

## 2026-08-14 — Commit des finitions visuelles du lecteur

### Objectif

Versionner ensemble les derniers ajustements cohérents de l'état vide et des
commandes du lecteur.

### Actions

- inclusion du grand verrou InkRiver dans l'état sans sélection ;
- inclusion du bouton d'actualisation à icône ;
- inclusion des actions enveloppe et étoile dans le panneau de lecture ;
- exclusion volontaire de `logo_raw.png`, conservé comme planche locale.

### Vérifications

- `git diff --cached --check` : succès avant commit ;
- arbre suivi propre après commit ; seul `logo_raw.png` reste non suivi.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `2069704` (`[AI] Polish reader controls and empty state`) ; ce
rapport reste hors de Git.

## 2026-08-14 — Archivage manuel et rétention automatique des articles

### Objectif

Retirer durablement de la chronologie les articles archivés volontairement et
limiter automatiquement le cache des anciens articles lus, sans permettre à un
rafraîchissement de les recréer.

### Actions

- ajout d'une migration SQLite enregistrant l'état, la date et le motif
  `manual` ou `retention` de l'archivage ;
- exclusion des pierres tombales de toutes les lectures et mises à jour locales,
  et protection de leur identifiant contre les réinsertions distantes ;
- suppression du corps HTML et passage du type de contenu à `missing` lors de
  chaque archivage, tout en conservant les métadonnées techniques ;
- ajout de la règle fixe de 30 jours pour les seuls articles lus, non favoris et
  datés, exécutée au démarrage de Tauri et après chaque rafraîchissement CLI ou
  Tauri ;
- ajout de la commande Tauri, de l'API TypeScript et de l'action d'archivage dans
  le panneau de lecture, avec icône, infobulle et confirmation obligatoire ;
- ajout du compteur d'archivages automatiques aux bilans Tauri et CLI ;
- mise à jour des README bilingues, de FR-007, création de FR-009 et mise à jour
  du TODO local.

### Justification

Une pierre tombale légère permet de faire disparaître définitivement l'article
de l'interface sans qu'un flux encore inchangé le restaure. La date de
publication constitue une limite stable ; le test strict à plus de 30 jours et
les exclusions des favoris, non-lus et articles sans date préviennent une perte
automatique inattendue. Aucun `VACUUM` automatique n'est lancé, car libérer des
pages SQLite ne garantit pas une réduction immédiate du fichier et ne doit pas
ralentir les rafraîchissements.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 117 tests Rust au total ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` : un
  premier passage a signalé un tuple de test trop complexe, remplacé par un
  alias local ; second passage réussi sans avertissement ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 49 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l'application Tauri exécuté par Codex.

Les tests couvrent la migration d'une base existante, l'archivage manuel, la
confirmation et les erreurs d'interface, la règle exacte de rétention et ses
exceptions, le démarrage Tauri, le rafraîchissement répété sans résurrection,
la purge du contenu, la suppression physique des tombstones avec leur flux et
la sérialisation camelCase du compteur.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade ; le rapport reste hors de Git.

## 2026-08-14 — Modale InkRiver de confirmation d'archivage

### Objectif

Remplacer la confirmation native d'archivage par une fenêtre cohérente avec
l'identité visuelle de l'application.

### Actions

- ajout d'une modale dédiée présentant l'icône d'archive, le titre de l'article,
  les conséquences de l'action et deux choix clairement distincts ;
- stylisation adaptée aux thèmes clair et sombre, avec une action destructive
  rouge et un fond assombri ;
- ajout de la fermeture par le bouton, la touche Échap ou un clic sur le fond ;
- placement initial du focus sur l'annulation, confinement de la navigation au
  clavier dans la modale et restitution du focus au bouton d'archive ;
- conservation de la confirmation native existante pour la suppression d'un
  abonnement, qui n'est pas concernée par cette intervention ;
- adaptation des tests de succès, d'échec et d'annulation de l'archivage.

### Justification

Une modale intégrée permet d'expliquer l'irréversibilité sans rompre visuellement
avec le lecteur. Le choix d'annulation reçoit le focus par défaut afin de limiter
les validations accidentelles d'une action destructive.

### Vérifications

- `npm run typecheck` : succès après ajout d'une annotation explicite demandée
  par le typage strict ;
- `npm test` : succès, 50 tests frontend ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test Rust exécuté, le changement concerne uniquement l'interface déjà
  couverte par l'API d'archivage existante ;
- aucun test visuel manuel exécuté par Codex.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : aucun à ce stade ; le rapport reste hors de Git.

## 2026-08-14 — Commit de l'archivage et de la rétention

### Objectif

Versionner ensemble l'archivage manuel, la rétention automatique et leur modale
de confirmation désormais validés dans l'application.

### Actions

- ajout explicite des douze fichiers de production, de tests, de documentation
  et de migration concernés ;
- exclusion de `logo_raw.png` et maintien hors de Git des documents locaux
  ignorés ;
- création d'un commit portant la mention d'assistance IA.

### Vérifications

- `git diff --check` : succès avant indexation ;
- `git diff --cached --check` : succès avant commit ;
- état de l'index contrôlé pour exclure `logo_raw.png`.

Ces changements ont été créés avec l'assistance d'une IA.

Commit associé : `f81ffc6` (`[AI] Add article archiving and retention`) ; ce
rapport reste hors de Git.

## 2026-08-16 — Prototype hors ligne d’extraction de contenu

### Objectif

Évaluer de façon reproductible la récupération du contenu principal d’articles
hors Medium et Substack à partir de pages HTML enregistrées localement.

### Actions

- ajout du module Rust `content_extractor`, basé sur `legible`, avec nettoyage
  final du HTML par la politique Ammonia déjà utilisée par InkRiver ;
- ajout d’un seuil conservateur de 2 000 caractères afin de rejeter les pages
  incomplètes, les aperçus et les murs d’abonnement trop courts ;
- ajout de fixtures synthétiques versionnées et de tests unitaires couvrant
  l’extraction, la résolution des liens, le nettoyage et les rejets ;
- ajout d’un test d’intégration ignoré par défaut pour les cinq pages du corpus
  local décrit dans `tests/pages/description.json` ;
- exclusion de `tests/pages/` de Git et retrait de ses répertoires de ressources
  `*_files`, déplacés dans `/tmp/inkriver-page-resources-backup-20260816` pour
  permettre une récupération temporaire ;
- documentation du corpus facultatif et de sa commande de test dans les deux
  README.

Le corpus réel reste local car il contient des pages tierces. Le seuil est
contrôlé après extraction, car le seuil homonyme de Legible guide son algorithme
mais ne garantit pas à lui seul la longueur du résultat. Aucun accès réseau ni
branchement au rafraîchissement ou à SQLite n’a été ajouté à cette étape.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace --offline` : succès ;
- `cargo test --workspace --offline` : succès, 120 tests passés et le test du
  corpus ignoré comme prévu ;
- `cargo test --test extraction_corpus -- --ignored --nocapture` : succès, les
  cinq pages locales respectent leurs résultats attendus ;
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` :
  succès ;
- `git diff --check` : succès ;
- aucun test npm ni test manuel de l’interface, aucune partie frontend n’ayant
  été modifiée.

Ces changements ont été créés avec l’assistance d’une IA.

Commit associé : aucun à ce stade.

## 2026-08-16 — Extraction réseau bornée des articles incomplets

### Objectif

Relier l’extracteur hors ligne au rafraîchissement pour compléter les articles
incomplets des flux `Other`, sans dégrader le cache RSS ni contacter Medium,
Substack ou des articles archivés.

### Actions

- ajout de `ContentKind::Extracted` et d’une migration SQLite conservant les
  articles, leurs états locaux et leurs tombstones tout en ajoutant l’historique
  des tentatives d’extraction ;
- sélection des articles `excerpt` ou `missing` issus de flux `Other` actifs,
  avec priorité aux plus récents, lot maximal de 20 et délai de sept jours après
  un échec ;
- ajout d’un client HTTP dédié : quatre requêtes simultanées, délai de 10
  secondes, corps décompressé limité à 2 Mio, cinq redirections contrôlées,
  validation HTML et refus des destinations locales, privées ou non routables ;
- résolution DNS préalable, épinglage des adresses validées, désactivation des
  proxies implicites et nouvelle validation à chaque redirection ;
- conservation de l’extrait RSS et mémorisation du motif en cas d’échec ; un
  contenu extrait résiste aux extraits RSS suivants mais reste remplaçable par
  un corps RSS réellement complet ;
- exécution de la rétention avant l’extraction afin qu’un article archivé au
  cours du rafraîchissement ne soit jamais téléchargé ;
- propagation des compteurs d’extraction au CLI, au DTO Tauri et à l’interface,
  où `extracted` est rendu comme un contenu complet ;
- documentation du fonctionnement dans les README et mise à jour du TODO local.

Les échecs d’extraction restent non bloquants et distincts des erreurs de flux.
Le téléchargement traite les résultats au fil de l’eau afin de borner aussi la
mémoire au nombre de requêtes concurrentes.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 135 tests passés et le test facultatif du
  corpus ignoré comme prévu ; les tests HTTP ont utilisé uniquement des sockets
  locales éphémères ;
- `cargo test --test extraction_corpus -- --ignored` : succès, 1 test passé sur
  les cinq pages locales ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 50 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test manuel de l’interface ni accès à des pages Internet réelles n’a été
  exécuté pendant cette intervention.

Ces changements ont été créés avec l’assistance d’une IA.

Commit associé : aucun à ce stade.

## 2026-08-16 — Commit de l’extraction réseau

### Objectif

Versionner ensemble le prototype hors ligne et son intégration réseau validée
par l’utilisateur sur la branche `dev`.

### Actions

- ajout à l’index des 21 fichiers de code, migration, tests et documentation ;
- exclusion de `logo_raw.png`, du corpus tiers et des documents locaux ignorés ;
- création d’un commit portant explicitement la mention d’assistance IA.

### Vérifications

- `git diff --check` : succès avant indexation ;
- `git diff --cached --check` : succès ;
- contrôle de l’index et de l’état final pour confirmer les exclusions.

Ces changements ont été créés avec l’assistance d’une IA.

Commit associé : `12e72e9` (`[AI] Extract incomplete article pages securely`) ;
ce rapport reste hors de Git.

## 2026-08-16 — Défilement unifié du panneau de lecture

### Objectif

Faire défiler ensemble l’en-tête et le corps de l’article au lieu de conserver
un défilement interne distinct dans l’iframe.

### Actions

- ajout d’un message de hauteur émis par le document isolé à son chargement et
  lors de ses redimensionnements ;
- redimensionnement contrôlé de l’iframe par la fenêtre parente après validation
  de la source et de la valeur reçue ;
- désactivation du défilement interne de l’iframe afin que `.reader` reste
  l’unique zone de défilement du panneau droit ;
- ajout de tests couvrant le pont de hauteur, les valeurs invalides et les
  attributs empêchant le scroll imbriqué.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 51 tests passés ;
- `npm run build` : succès ;
- aucun test visuel manuel exécuté par Codex.

Ces changements ont été créés avec l’assistance d’une IA.

Commit associé : aucun à ce stade.

## 2026-08-16 — Commit du défilement unifié

### Objectif

Versionner le défilement commun de l’en-tête et du corps d’article validé par
l’utilisateur.

### Actions

- ajout à l’index des trois fichiers frontend concernés ;
- exclusion de `logo_raw.png` ;
- création d’un commit portant la mention d’assistance IA.

### Vérifications

- `git diff --check` : succès avant indexation ;
- `git diff --cached --check` : succès ;
- état de l’index vérifié avant le commit.

Ces changements ont été créés avec l’assistance d’une IA.

Commit associé : `5b08615` (`[AI] Unify article panel scrolling`) ; ce rapport
reste hors de Git.

## 2026-08-16 — Release InkRiver 0.2.0

### Objectif

Figer la milestone validée sous une version cohérente, la publier sur `main` et
créer son tag stable.

### Actions

- passage des versions Cargo, npm et Tauri de `0.1.0` à `0.2.0` ;
- ajout de `CHANGELOG.md` et actualisation des fonctionnalités présentées dans
  les README bilingues ;
- création et push du commit `6cb060d` sur `dev` ;
- fusion de `dev` dans `main` par le commit `8925cc5`, puis push de `main` ;
- création et push du tag annoté `v0.2.0`, pointant sur le commit de release ;
- génération du paquet `InkRiver_0.2.0_amd64.deb` et de l’AppImage
  `InkRiver_0.2.0_amd64.AppImage` ;
- retour sur `dev` après publication des références stables.

### Vérifications

- `cargo fmt --check`, `cargo check --workspace` et `git diff --check` : succès ;
- `cargo test --workspace` : succès, 135 tests passés ;
- `cargo test --test extraction_corpus -- --ignored` : succès ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck`, `npm test` (51 tests) et `npm run build` : succès ;
- `npm run tauri build` : succès, `.deb` de 11 Mio et AppImage de 85 Mio ;
- vérification distante : `origin/dev` pointe sur `6cb060d`, `origin/main` et le
  tag déréférencé `v0.2.0` pointent sur `8925cc5` ;
- sommes SHA-256 : `.deb`
  `4995d5dd3a7a90973e3e96b49922419ace3b535a46797527eda8e32f33e29760`,
  AppImage
  `44dc5c59f4b44179c8258dfc56f9cd1d0c2c531cb189394c664e06193fc53819`.

L’objet « GitHub Release » et l’envoi des deux artefacts n’ont pas été créés :
aucun client `gh`, `hub` ou jeton d’API GitHub n’est disponible localement.

Ces changements ont été créés avec l’assistance d’une IA.

Commits associés : `6cb060d` (`[AI] Prepare InkRiver 0.2.0`) et `8925cc5`
(`[AI] Release InkRiver 0.2.0`) ; tag `v0.2.0`. Ce rapport reste hors de Git.

## 2026-08-16 — Réalignement de dev après la release

### Objectif

Faire repartir le développement depuis le commit exact publié sur `main`.

### Actions et vérifications

- vérification que `main` descendait directement de `dev`, avec un seul commit
  de merge d’écart et aucun changement de contenu ;
- fast-forward de `dev` vers `8925cc5` avec `git merge --ff-only main` ;
- push de `dev` vers GitHub ; aucun nouveau commit n’a été créé ;
- `logo_raw.png` est resté hors de Git.

Ces opérations ont été réalisées avec l’assistance d’une IA.

Commit associé : `8925cc5` (`[AI] Release InkRiver 0.2.0`), désormais commun à
`main`, `dev` et au tag `v0.2.0`. Ce rapport reste hors de Git.

## 2026-08-20 — FR-001 : notifications d’action temporaires

### Objectif

Éviter que les notifications vertes d’action occupent durablement l’interface,
tout en laissant suffisamment de temps pour les lire.

### Actions

- centralisation de l’affichage et de la suppression des notifications de
  succès ;
- disparition automatique après huit secondes et ajout d’un bouton de
  fermeture accessible ;
- suspension du délai au survol ou lorsque la notification possède le focus ;
- conservation du comportement persistant des bandeaux d’erreur rouges ;
- ajout de tests Vitest pour la fermeture manuelle, le délai, sa suspension au
  survol et au focus, ainsi que la persistance des erreurs ;
- passage de FR-001 à l’état `terminée` dans le registre local des demandes.

Le délai est suivi avec son temps restant afin qu’un survol ou une navigation
au clavier ne redémarre pas artificiellement les huit secondes complètes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 55 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Commit créé ensuite : `017fdb2` (`[AI] Add floating action notifications`).

## 2026-08-20 — Notifications flottantes

### Objectif

Empêcher les notifications d’action de décaler la chronologie et le panneau de
lecture lors de leur apparition.

### Actions et choix

- retrait de la zone de notification du flux de la grille principale ;
- positionnement d’une pile flottante centrée sous la barre supérieure ;
- présentation des succès et erreurs sous forme de cartes avec bordure, coins
  arrondis et ombre légère ;
- conservation des fondus, du délai, de la pause au survol et au focus, ainsi
  que de la fermeture manuelle des notifications vertes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 55 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-20 — Animation des notifications d’action

### Objectif

Adoucir l’apparition et la disparition des notifications vertes de FR-001.

### Actions et choix

- ajout d’un fondu de 180 ms accompagné d’un déplacement vertical discret ;
- conservation temporaire du bandeau dans le DOM pendant son animation de
  sortie, aussi bien après expiration qu’après fermeture manuelle ;
- désactivation de l’animation lorsque le système demande une réduction des
  mouvements ;
- adaptation des tests de durée pour vérifier les états d’entrée et de sortie.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 55 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Les trois interventions FR-001 ci-dessus ont finalement été regroupées dans le
commit `017fdb2` (`[AI] Add floating action notifications`) sur `dev`.

## 2026-08-20 — FR-002 : bouton de retour en haut

### Objectif

Permettre de revenir rapidement à l’en-tête d’un article long sans perturber la
lecture ni réinitialiser le défilement par un rerendu de l’interface.

### Actions et choix

- ajout d’un bouton circulaire flottant avec une flèche vers le haut dans le
  panneau de lecture ;
- apparition après une hauteur d’écran et disparition sous 75 % de cette
  hauteur, afin d’éviter les oscillations autour d’un seuil unique ;
- mise à jour directe de la classe du bouton depuis un écouteur de défilement
  passif, sans rerendre l’application ;
- retour en haut avec le défilement fluide natif et repli immédiat lorsque la
  réduction des mouvements est demandée ;
- ajout du fondu, des libellés accessibles et de la gestion du focus ;
- ajout de tests pour les seuils, le scroll, la réduction des mouvements et le
  changement d’article ;
- passage de FR-002 à l’état `terminée` dans le registre local des demandes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 58 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Commit créé ensuite : `fb96f64` (`[AI] Add article scroll-to-top action`).

## 2026-08-20 — FR-003 : actions en fin d’article

### Objectif

Rendre les actions principales accessibles après une longue lecture sans
obliger à revenir à l’en-tête.

### Actions et choix

- ajout d’un pied d’article contenant trois boutons ronds : favori, archivage
  et ouverture de la source ;
- affichage de l’URL complète dans l’infobulle et le libellé accessible du
  bouton source ; bouton explicite mais désactivé pour une URL absente ou non
  HTTP(S) ;
- réutilisation des actions, de la confirmation d’archivage et du plugin
  d’ouverture existants ;
- branchement de toutes les occurrences des actions dupliquées ;
- conservation de la position verticale et de la hauteur calculée de l’iframe
  lors des rerendus du même article ;
- retour du focus vers le bouton d’archivage du pied après annulation de la
  confirmation ;
- ajout de tests pour le rendu des trois actions, l’ouverture exacte de la
  source, les URL indisponibles, la synchronisation du favori et la conservation
  de la position de lecture ;
- passage de FR-003 à l’état `terminée` dans le registre local des demandes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 61 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Commit créé ensuite : `b8dbd73` (`[AI] Add end-of-article actions`).

## 2026-08-20 — FR-005 : filtre des articles non lus

### Objectif

Permettre de masquer temporairement les articles lus dans la chronologie sans
modifier le cache ni appeler le backend.

### Actions et choix

- ajout d’un bouton-filtre « Non lus » avec enveloppe, compteur, état visuel et
  attribut `aria-pressed` ;
- composition du filtre avec les vues « Tous » et « Favoris » ;
- ajout d’un état vide spécifique lorsque la vue filtrée ne contient aucun
  article ;
- maintien de l’article dans le lecteur lorsqu’il devient lu et disparaît de la
  liste filtrée ;
- réinitialisation du défilement de la chronologie lors du changement de filtre ;
- conservation du choix uniquement pendant la session, sans stockage ni nouvel
  appel backend ;
- ajout de tests pour l’activation réversible, le compteur, la combinaison avec
  les favoris et la lecture automatique d’un article filtré ;
- passage de FR-005 à l’état `terminée` dans le registre local des demandes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 64 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Commit créé ensuite : `3c55886` (`[AI] Add unread article filter`).

## 2026-08-20 — FR-007 : agrandissement des images

### Objectif

Permettre d’examiner une image d’article sans quitter InkRiver ni déclencher le
lien externe qui l’entoure éventuellement.

### Actions et choix

- ajout d’un message dédié entre l’iframe isolée et l’interface principale,
  traité avant les liens lorsque la cible est une image ;
- ajout d’une fenêtre d’image limitée au panneau de lecture, à 80 % de sa
  largeur et 85 % de sa hauteur, sans agrandissement forcé des petites images ;
- prise en charge des sources HTTPS, `data:image` et des chemins relatifs
  résolus depuis l’URL de l’article ; rejet silencieux des autres schémas et des
  URL HTTPS contenant des identifiants ;
- fermeture par croix translucide, Échap ou clic sur l’arrière-plan, avec fondu
  et léger changement d’échelle respectant la réduction des mouvements ;
- ajout du curseur de zoom, de l’activation par Entrée ou Espace, du focus sur
  la fermeture et de sa restitution à l’image via le pont de l’iframe ;
- fermeture automatique lors d’un changement d’article ou de l’ouverture de la
  gestion des abonnements ;
- ajout de tests pour la résolution des URL, la priorité sur les liens, le pont
  iframe, les trois fermetures, le clavier, les sources refusées et le changement
  d’article ;
- passage de FR-007 à l’état `terminée` dans le registre local des demandes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 68 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

Commit créé ensuite : `5c19fb1` (`[AI] Add article image lightbox`).

## 2026-08-22 — FR-008 : zoom du texte des articles

### Objectif

Offrir trois niveaux de taille lisibles et persistants pour le contenu des
articles, sans redimensionner le reste de l’interface ni perdre la progression
dans une longue lecture.

### Actions et choix

- ajout des niveaux Petit (16 px), Moyen (18 px) et Grand (22 px), avec Moyen
  comme valeur par défaut ;
- persistance globale dans `localStorage`, avec repli silencieux sur Moyen si la
  valeur est invalide ou si le stockage est indisponible ;
- ajout de deux groupes synchronisés, dans l’en-tête et le pied d’article, avec
  loupes moins/plus, libellé courant, infobulles et limites désactivées ;
- ajout d’une variable CSS et d’un message au pont de l’iframe pour modifier le
  corps de l’article sans recréer le document ;
- calcul de la progression relative avant le changement puis restauration après
  la nouvelle hauteur transmise par le `ResizeObserver` ;
- application immédiate de la préférence aux articles ouverts ensuite, sans
  commande Tauri, appel backend ni migration SQLite ;
- ajout de tests pour les trois niveaux, les deux groupes, la persistance, les
  valeurs invalides, le stockage indisponible, le pont iframe et le reflow ;
- passage de FR-008 à l’état `terminée` dans le registre local des demandes.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 72 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-22 — FR-009 : actualisation individuelle d’un abonnement

### Objectif

Permettre d’actualiser un seul abonnement actif depuis sa carte, sans traiter
ni déplacer silencieusement les données des autres flux.

### Actions et choix

- ajout d’une actualisation Rust ciblée qui valide l’existence et l’état actif
  du flux avant tout accès réseau ;
- limitation au flux demandé de la collecte, de l’archivage automatique et de
  l’extraction des pages, avec conservation du rapport existant ;
- ajout de la commande Tauri `refresh_feed`, des erreurs structurées
  `feed_not_found`, `feed_inactive` et `refresh_in_progress`, ainsi que d’un
  verrou commun non bloquant pour toutes les actualisations ;
- ajout d’une icône d’actualisation sur chaque carte, désactivée pour les flux
  inactifs et animée uniquement pour le flux en cours ;
- rechargement du cache et des métadonnées sans quitter la gestion des
  abonnements, avec notification de succès ou toast rouge détaillé, fermable et
  temporisé en cas d’échec ;
- maintien d’un ajout d’abonnement sans téléchargement automatique ;
- ajout de tests Rust, Tauri et Vitest couvrant le scope, les erreurs, le verrou,
  les états visuels, les notifications et l’absence d’appel global involontaire ;
- mise à jour des README bilingues et passage de FR-009 à l’état `terminée`.

Le ciblage est appliqué à toute la maintenance afin qu’une action présentée
comme individuelle n’archive ni n’enrichisse les articles d’un autre flux.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 125 tests du cœur et 15 tests Tauri passés,
  2 tests CLI passés et le corpus local optionnel ignoré comme prévu ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 75 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-22 — FR-009 : conservation du défilement des abonnements

### Objectif

Éviter que l’actualisation individuelle replace la page Abonnements en haut.

### Actions et choix

- capture de la position de défilement du conteneur de gestion avant chaque
  rerendu et restauration immédiate sur le nouveau conteneur ;
- ajout d’un test couvrant le rerendu de démarrage puis celui de fin
  d’actualisation ciblée.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 76 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-22 — Archivage rapide depuis la chronologie

### Objectif

Connecter le bouton d’archivage ajouté à chaque ligne d’article à la
confirmation et à la commande backend existantes, sans ouvrir l’article.

### Actions et choix

- ajout d’une action de chronologie portant explicitement l’identifiant de
  l’article ciblé ;
- refactorisation de la confirmation pour retrouver l’article dans le cache au
  lieu de dépendre de l’article sélectionné ;
- conservation de l’article en cours de lecture lorsqu’une autre ligne est
  archivée ;
- restauration du focus sur le bouton d’origine après annulation et ajout d’un
  libellé accessible comprenant le titre ;
- blocage des actions concurrentes de la ligne pendant l’archivage, adaptation
  de l’espace réservé aux trois icônes et suppression du journal de débogage ;
- ajout de tests pour l’ouverture sans lecture automatique, l’annulation, le
  ciblage distinct de l’article ouvert et l’échec de persistance.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 79 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-22 — FR-010 : logos des sites associés aux flux RSS

### Objectif

Identifier visuellement les flux `Other` avec le logo de leur site dans la
chronologie, le lecteur et la gestion des abonnements, tout en conservant un
fonctionnement hors ligne et le repli RSS.

### Actions et choix

- conservation du site principal et de l'icône déclarée par RSS, Atom ou JSON
  Feed dans les métadonnées de collecte ;
- ajout d'une migration SQLite pour le PNG normalisé, son site, les tentatives
  de découverte et leur dernier motif d'échec ;
- découverte bornée lors des seules actualisations réussies de flux `Other` :
  icône déclarée, balises HTML puis `/favicon.ico`, avec 20 flux maximum,
  quatre tâches simultanées et sept jours de délai après un échec ;
- réutilisation du téléchargement HTTP(S) protégé contre les destinations
  privées et les redirections interdites, avec limites de temps et de taille ;
- décodage de PNG, JPEG, GIF, WebP et ICO, rendu SVG sans ressources externes,
  puis normalisation transparente en PNG 64 × 64 ;
- exposition du cache au frontend sous forme de `data:image/png;base64`, sans
  duplication dans les DTO d'articles ni accès réseau depuis la WebView ;
- ajout du logo dans les trois vues, maintien des marques Medium/Substack et
  repli automatique vers l'icône RSS en cas d'absence ou d'erreur de rendu ;
- documentation de la fonctionnalité dans les README bilingues et classement
  de FR-010 comme terminée.

### Vérifications

- `cargo fmt --all --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 156 tests passés et le test de corpus
  explicitement ignoré par la suite ordinaire ;
- `cargo test --test extraction_corpus -- --ignored` : succès, 1 test passé sur
  le corpus local non suivi ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 81 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit n’a été
créé à ce stade.

## 2026-08-22 — Préparation de la release InkRiver 0.3.0

### Objectif

Préparer une release Linux stable incluant les paquets Debian et AppImage.

### Actions et choix

- passage à `0.3.0` des manifestes Rust, npm et Tauri ainsi que de leurs
  lockfiles ;
- ajout au changelog des fonctionnalités livrées depuis `v0.2.0` ;
- construction optimisée des deux cibles Linux configurées par Tauri ;
- vérification des métadonnées et du contenu du paquet Debian, du type ELF de
  l'AppImage et calcul des sommes SHA-256 des deux artefacts.

### Vérifications

- `cargo fmt --all --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 156 tests passés et 1 test de corpus
  explicitement ignoré par la suite ordinaire ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 81 tests passés ;
- `npm run build` : succès ;
- `npm run tauri build` : succès, génération de
  `InkRiver_0.3.0_amd64.deb` et `InkRiver_0.3.0_amd64.AppImage` ;
- `git diff --check` : succès.

Sommes SHA-256 :

- `.deb` : `2bc8173eb792f6420b83809de70cc0f7d9604b4842cbf159a6884b45b0809b21` ;
- AppImage : `7d137a4ec0002dcd05c61de6335f5e8e40e4f5f4d1c84568b2e5b725d50cd97a`.

Ces changements ont été créés avec l’assistance d’une IA. Commit associé :
`1964bd9` (`[AI] Prepare InkRiver 0.3.0`).

## 2026-08-22 — Finalisation de la release InkRiver 0.3.0

### Objectif

Vérifier le jalon publié et remettre les branches de développement au même
point de départ après la release.

### Actions et choix

- récupération des références distantes et vérification du tag annoté
  `v0.3.0` sur le merge `cd40f31` de la PR n° 3 ;
- avance rapide de `dev` sur ce même commit, puis mise à jour de `origin/dev` ;
- conservation des fichiers locaux non suivis hors de toute opération Git.

### Vérifications

- `git show v0.3.0` : tag annoté valide pointant sur `cd40f31` ;
- `git push origin dev` : succès, `dev`, `main` et `v0.3.0` partagent le même
  commit ;
- aucun test supplémentaire exécuté : le code est inchangé depuis les
  validations de préparation de la release.

Cette intervention a été réalisée avec l’assistance d’une IA. Aucun nouveau
commit de contenu n’a été créé.

## 2026-08-22 — Correctif CSP de l’iframe pour InkRiver 0.3.1

### Objectif

Rétablir dans les paquets Linux l’exécution du pont JavaScript isolé qui gère
la hauteur complète des articles, le zoom des images, les liens et la taille du
texte, sans autoriser arbitrairement les scripts inline.

### Actions et choix

- extraction du script-pont dans une constante stable et suppression de son
  nonce aléatoire, qui ne pouvait pas être ajouté à la CSP Tauri au moment du
  build ;
- calcul de l’empreinte SHA-256 exacte du pont et autorisation de cette seule
  empreinte dans la CSP héritée de Tauri et dans celle du document `srcdoc` ;
- ajout d’un test recalculant l’empreinte et vérifiant sa présence dans les deux
  CSP afin qu’une future modification non synchronisée échoue explicitement ;
- passage des manifestes Rust, npm et Tauri à la version corrective `0.3.1` et
  ajout de cette version au changelog ;
- construction des paquets Debian et AppImage, avec contrôle de la présence de
  la CSP corrigée dans le binaire final.

Le recours à une empreinte limite l’autorisation au pont attendu et évite
`script-src 'unsafe-inline'`, qui aurait affaibli toute l’application.

### Vérifications

- `cargo fmt --all --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès hors sandbox, 156 tests passés et 1 test de
  corpus local ignoré ; la première exécution isolée avait bloqué cinq serveurs
  HTTP de test locaux avec `Operation not permitted` ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 81 tests passés ;
- `npm run build` : succès ;
- `npm run tauri build` : succès, génération de
  `InkRiver_0.3.1_amd64.deb` et `InkRiver_0.3.1_amd64.AppImage` ;
- inspection du binaire final : empreinte CSP attendue présente ;
- `git diff --check` : succès.

Sommes SHA-256 :

- `.deb` : `afb4108db052d3c0d300d63b2955563884293a6ff7f3a6638caf8727b32e1f36` ;
- AppImage : `743b0af37c4644f0bf9bd02bf3f80052e617adb4f6b4228f03fe1921ae6c58f1`.

Le contrôle interactif du redimensionnement, du zoom et des liens dans
l’AppImage reste à effectuer dans la session graphique de l’utilisateur. Ces
changements ont été créés avec l’assistance d’une IA. Aucun commit associé à ce
stade.

## 2026-08-22 — Filtres exclusifs de la chronologie

### Objectif

Remplacer la combinaison peu pratique entre les vues et le filtre non lu par
trois choix simples et mutuellement exclusifs : `Tous`, `Favoris` et `Non lus`.

### Actions et choix

- remplacement des deux états frontend composables par une seule vue à trois
  valeurs ;
- intégration de `Non lus` au même groupe d’onglets accessible que `Tous` et
  `Favoris` ;
- calcul indépendant des listes : tous les articles, tous les favoris quel que
  soit leur état de lecture, ou tous les articles non lus ;
- conservation des compteurs globaux, des états vides et du lecteur ouvert
  lorsqu’un article disparaît de la vue après son marquage comme lu ;
- simplification des styles et suppression du bouton-toggle devenu inutile ;
- adaptation des tests, des README bilingues et du registre local des demandes.

Un état unique garantit visuellement et fonctionnellement qu’une seule vue peut
être sélectionnée à la fois, sans appel supplémentaire au backend.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 81 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : à exécuter après la mise à jour du rapport.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit associé
à ce stade.

## 2026-08-22 — Backlog de synchronisation multi-appareils

### Objectif

Transformer l’idée de synchronisation Linux/Android sans compte InkRiver en une
user story exploitable et en tickets de développement ordonnés.

### Actions et choix

- ajout de `FR-012` dans le registre local avec valeur utilisateur, périmètre,
  exclusions, critères d’acceptation et décisions d’architecture ;
- découpage en onze tickets couvrant successivement les règles de conflit, le
  journal SQLite, la production et la fusion des événements, les segments
  locaux, le chiffrement, WebDAV, l’appairage, l’interface, l’automatisation et
  la compaction ;
- définition des dépendances et d’un critère d’acceptation vérifiable pour
  chaque ticket ;
- rattachement de l’évolution correspondante du `TODO.md` à ce backlog ;
- maintien de SQLite comme source de vérité locale, avec échange de segments
  chiffrés immuables plutôt que synchronisation du fichier `reader.db`.

Le découpage commence par un export/import sans réseau afin de valider la
convergence avant d’ajouter WebDAV et les contraintes propres à Android.

### Vérifications

- relecture de la user story, des dépendances et des critères d’acceptation :
  effectuée ;
- aucun test logiciel exécuté : cette intervention modifie uniquement la
  documentation locale ignorée par Git.

Ces changements ont été créés avec l’assistance d’une IA. Aucun commit associé
à cette intervention.

## 2026-08-23 — Première interface mobile Android

### Objectif

Réaliser un premier jalon Android utilisable sans dupliquer le backend : une
chronologie puis un écran de lecture sur mobile, avec des interactions tactiles
et une navigation Retour cohérente.

### Actions et choix

- ajout d'un état de navigation mobile séparant chronologie et lecteur sous
  720 px, tout en conservant les deux panneaux sur ordinateur ;
- ajout d'une barre Retour mobile et branchement du bouton Retour Android avec
  fermeture prioritaire du zoom, des confirmations et des formulaires ;
- adaptation responsive de la chronologie, du lecteur et de son iframe, de la
  gestion des abonnements, des dialogues, des notifications et du zoom image ;
- passage des principales cibles tactiles à 44 px et prise en compte des zones
  sûres de l'écran ;
- configuration de Vite avec `TAURI_DEV_HOST`, ajout d'une configuration Tauri
  Android sans dimensions minimales desktop et séparation des capabilities
  desktop/mobile ;
- ajout de tests frontend sur la navigation mobile et l'ordre de fermeture des
  superpositions ;
- mise à jour des README bilingues et de la feuille de route locale avec les
  prérequis et commandes Android.

L'interface réutilise les commandes Tauri, le cœur Rust, les migrations et la
couche SQLite existants. Aucune base ou API propre à Android n'est introduite.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès hors sandbox, 156 tests passés et 1 test de
  corpus local ignoré ; la première exécution isolée avait bloqué cinq serveurs
  HTTP locaux avec `Operation not permitted` ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 83 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- `npm run tauri android init` : non exécuté jusqu'au bout, arrêt propre avant
  génération car Java, Android Studio, le SDK et le NDK ne sont pas installés
  sur la machine.

Une validation visuelle sur émulateur ou appareil réel et la construction de
l'APK restent donc à effectuer après installation de l'outillage Android. Ces
changements ont été créés avec l'assistance d'une IA. Aucun commit associé à ce
stade.

## 2026-08-23 — Correction TLS des actualisations Android

### Objectif

Empêcher l'actualisation Android de rester indéfiniment en cours après la
panique de `rustls-platform-verifier` lors de la première requête HTTPS.

### Actions et choix

- ajout conditionnel de `webpki-root-certs` pour Android et configuration de
  reqwest avec les racines publiques Mozilla embarquées ;
- centralisation du constructeur HTTP afin que la collecte RSS, l'extraction
  d'articles et la découverte de logos utilisent toutes la même configuration
  TLS Android ;
- maintien du magasin de certificats système sur les plateformes desktop ;
- ajout d'un délai maximal de 20 secondes par requête RSS et de 10 secondes
  pour la connexion, afin qu'une panne réseau soit remontée au lieu de laisser
  l'interface attendre sans limite ;
- ajout d'un test unitaire vérifiant que le client configuré peut être construit.

Ce choix évite l'initialisation JNI/Kotlin requise par le vérificateur de
plateforme tout en conservant une validation TLS stricte pour les flux publics.

### Vérifications

- `cargo fmt --check` : succès après formatage ;
- `cargo check --workspace` : succès ;
- `cargo check -p inkriver --target x86_64-linux-android` avec les outils du
  NDK explicitement configurés : succès ;
- `cargo test --workspace` : succès hors sandbox, 157 tests passés et 1 test de
  corpus local ignoré ; la première exécution isolée avait empêché cinq
  serveurs HTTP de test d'ouvrir un port local ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- construction Tauri Android de débogage : frontend et bibliothèque Rust
  Android compilés avec succès ; phase APK arrêtée car une autre session
  Android détenait déjà le verrou global.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`31e3e57` (`[AI] Add secure device pairing foundation`).

## 2026-08-24 — Barre d'actions persistante sur desktop

### Objectif

Conserver les actions de l'article visibles en haut du panneau de lecture
desktop pendant tout le défilement, après la suppression manuelle des actions
Favori et Archiver du pied de page.

### Actions et choix

- fixation de `.reader-actions` sous la barre principale sur les écrans desktop,
  avec fond translucide, séparation et ombre légère ;
- ajout de l'espace nécessaire au début de l'article afin que la barre ne
  masque pas ses métadonnées ;
- restauration du positionnement normal dans la règle responsive mobile ;
- simplification du pied de page pour ne conserver que l'ouverture de la
  source et nettoyage du routage d'archivage devenu inutile ;
- validation du choix d'afficher les plateformes par leur seule icône, sans
  libellé texte redondant, et adaptation du test correspondant ;
- adaptation des tests du pied de page au retrait des actions dupliquées.

Les modifications locales distinctes de l'utilisateur ont été préservées.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 83 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Actions allégées dans la chronologie mobile

### Objectif

Simplifier chaque ligne de la liste d'articles sur mobile en retirant l'action
Archiver, remplacée ensuite par un geste, tout en conservant les accès rapides
aux favoris et à l'état Lu/Non lu.

### Actions et choix

- masquage responsive du bouton `timeline-archive` sous 720 px, sans modifier
  sa présence sur desktop ni sa logique ;
- maintien des boutons Favori et Lu/Non lu, avec réservation de l'espace
  nécessaire à droite du titre.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 83 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Actions persistantes dans le lecteur mobile

### Objectif

Rendre les actions de lecture accessibles en permanence en haut de l'article
sur mobile et réduire la commande de retour à une flèche compacte.

### Actions et choix

- intégration d'une variante mobile complète de `reader-actions` dans la barre
  sticky du lecteur, à droite de la commande Retour ;
- remplacement du texte « Articles » par une flèche seule, avec infobulle et
  libellé ARIA « Retour aux articles » ;
- factorisation du rendu des actions desktop/mobile et branchement de toutes
  leurs occurrences aux actions de lecture, favori, archivage, source et zoom ;
- utilisation d'une icône compacte pour « Lire l'original » sur mobile afin de
  préserver l'espace horizontal ;
- maintien de la barre fixe desktop et masquage de sa variante dans la mise en
  page mobile ;
- restauration du focus vers le bouton d'archivage mobile après annulation de
  la confirmation.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 83 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Alignement du badge de source

### Objectif

Aligner verticalement le logo de plateforme déplacé par l'utilisateur sur la
ligne contenant la source de l'article.

### Actions et choix

- transformation de `.article-source` en conteneur flex centré verticalement ;
- ajout d'un espacement régulier entre le libellé, le badge et le domaine ;
- conservation du retour à la ligne pour les faibles largeurs mobiles.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 83 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Chronologie inspirée d'une boîte de réception

### Objectif

Renforcer l'identification visuelle des articles dans la chronologie desktop et
mobile en plaçant un grand logo de source à gauche du titre.

### Actions et choix

- restructuration de chaque ligne en deux colonnes : logo puis métadonnées ;
- affichage du logo en 48 px sur desktop et 54 px sur mobile, soit environ 50 %
  de la hauteur courante d'une ligne ;
- regroupement de l'auteur et de la date sur une même ligne compacte au-dessus
  du titre, avec ellipse sur les noms d'auteur trop longs ;
- affichage des titres sur deux lignes au maximum avec une ellipse CSS et
  conservation du titre complet dans l'infobulle desktop ;
- élargissement de la chronologie desktop de 370 à 480 px via une variable
  partagée avec la barre d'actions et le zoom d'image ;
- ajout d'un test structurel couvrant le positionnement du logo et le titre.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 84 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Pull-to-refresh mobile

### Objectif

Remplacer le bouton d'actualisation global de l'interface mobile par le geste
familier consistant à tirer la chronologie vers le bas lorsqu'elle est déjà en
haut.

### Actions et choix

- ajout d'un suivi tactile borné et amorti sur la chronologie mobile ;
- déclenchement uniquement depuis `scrollTop = 0`, après franchissement d'un
  seuil visuel de 72 px puis relâchement ;
- annulation des gestes horizontaux, des remontées et des gestes commencés
  ailleurs qu'en haut de la liste ;
- ajout d'un indicateur compact « Tirez », « Relâchez » puis
  « Actualisation en cours », avec animation désactivée lorsque la réduction
  des mouvements est demandée ;
- réutilisation du pipeline d'actualisation globale existant et protection
  contre les déclenchements concurrents ;
- masquage responsive du bouton d'actualisation global, sans changement sur
  desktop ni sur les boutons propres à chaque abonnement ;
- ajout d'un test tactile simulé couvrant le seuil, le déclenchement et le
  refus lorsque la liste n'est pas en haut.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 85 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Swipe-to-archive mobile

### Objectif

Permettre l'archivage direct d'un article depuis la chronologie mobile en
faisant glisser sa ligne vers la droite, sans dialogue de confirmation.

### Actions et choix

- ajout d'un panneau rouge avec icône et libellé révélé progressivement sous
  chaque ligne mobile ;
- suivi horizontal de la ligne avec validation uniquement au relâchement après
  50 % de sa largeur ;
- retour animé sous le seuil et sortie complète de l'écran au-dessus ;
- distinction de l'intention horizontale et abandon immédiat au profit du
  défilement lorsque le mouvement devient vertical ;
- exclusion des 24 px du bord gauche pour préserver le geste Retour Android,
  ainsi que de la zone des boutons d'action ;
- appel direct à l'archivage existant sans confirmation, avec maintien de la
  ligne technique en base ;
- blocage des interactions pendant l'écriture et restauration de la ligne avec
  l'erreur exacte si le backend échoue ;
- adaptation du fond des lignes aux thèmes clair et sombre afin que la zone
  rouge reste masquée avant le geste ;
- ajout de tests tactiles couvrant le bord protégé, le bouton Favori, le seuil,
  l'absence de confirmation, le succès et l'échec de persistance.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 87 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — FR-013 : sélection multiple mobile

### Objectif

Permettre de sélectionner plusieurs articles depuis la chronologie Android et
de leur appliquer atomiquement un état lu/non lu ou un archivage manuel.

### Actions et choix

- ajout d'un appui long mobile de 500 ms, annulé après un déplacement de plus
  de 10 px, sans ouverture ni marquage automatique de l'article ;
- remplacement du logo par un check et affichage d'une barre groupée avec
  sélection de tous les articles visibles, actions lu/non lu et archivage ;
- suspension du pull-to-refresh, du swipe-to-archive et des boutons individuels
  pendant la sélection, avec annulation par Retour ou changement de page ;
- adaptation de la confirmation d'archivage au nombre d'articles concernés ;
- ajout de commandes Tauri et de méthodes SQLite groupées qui dédupliquent les
  identifiants et annulent toute la transaction si un article est absent ou
  déjà archivé ;
- conservation de la sélection et affichage de l'erreur exacte en cas d'échec ;
- ajout de tests Rust et Vitest couvrant atomicité, gestes, filtres, actions,
  confirmation, erreurs et navigation ;
- mise à jour des README bilingues et de `FEATURE_REQUESTS.md`.

### Vérifications

- `cargo fmt --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 161 tests passés et un test de corpus local
  explicitement ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 95 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — FR-014 : progression de lecture mobile

### Objectif

Donner au lecteur mobile un repère permanent sur sa position et la longueur
restante dans un article long.

### Actions et choix

- ajout à droite du lecteur mobile d'une piste fine et non interactive ;
- calcul d'un curseur dont la hauteur représente la portion visible et dont la
  position suit le défilement réel du conteneur `.reader` ;
- mise à jour au scroll, au redimensionnement, au chargement de l'iframe et
  après une remise en page provoquée par le zoom du texte ;
- masquage automatique lorsque l'article ne nécessite aucun défilement ;
- respect des zones sûres Android et de la préférence de réduction des
  animations ;
- ajout d'un test couvrant le début, la moitié, la fin et un article court ;
- mise à jour des README bilingues et de `FEATURE_REQUESTS.md`.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 96 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Résumé compact des rafraîchissements

### Objectif

Rendre la notification verte affichée après un rafraîchissement beaucoup plus
rapide à lire.

### Actions et choix

- suppression des compteurs d'articles actualisés, extraits, différés et en
  échec d'extraction dans les notifications de succès ;
- affichage uniquement des nouveaux articles et des anciens articles archivés
  automatiquement, en omettant chaque compteur nul ;
- ajout d'un état « À jour » accompagné d'un check vert lorsqu'aucun article
  n'a été ajouté ou supprimé ;
- application du même résumé aux rafraîchissements globaux et individuels ;
- maintien en rouge des erreurs de flux afin de ne jamais afficher « À jour »
  après un rafraîchissement incomplet ;
- adaptation des tests de notifications et de temporisation.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 96 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-24 — Icône InkRiver pour Android

### Objectif

Remplacer l'icône Tauri générique du lanceur Android par l'identité visuelle
InkRiver dans les prochains APK.

### Actions et choix

- génération avec la CLI officielle Tauri de toutes les tailles desktop,
  Android et iOS à partir du PNG InkRiver 1024 × 1024 ;
- remplacement direct des ressources Android `mipmap-*` du projet généré,
  notamment les icônes classique, ronde et adaptative ;
- ajout de `npm run icons` pour rendre l'opération reproductible après un
  nouveau `tauri android init` ;
- déclaration des formats desktop usuels dans `tauri.conf.json` ;
- documentation de la commande dans les README français et anglais ;
- contrôle visuel de l'icône Android `xxxhdpi` et de sa couche adaptative.

### Vérifications

- `npm run tauri icon -- src-tauri/icons/icon.png` : succès ;
- compilation Gradle `:app:processUniversalDebugResources` avec le JDK 17 :
  succès ;
- la première tentative avec le JDK système 25 a échoué lors de la
  configuration de Gradle, sans modification du projet ;
- `cargo check --workspace` : succès ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`7a62f58` (`* Add InkRiver application icons`).

## 2026-08-24 — Harmonisation des couleurs avec le logo InkRiver

### Objectif

Rapprocher l'identité visuelle des interfaces desktop et mobile des couleurs
bleu nuit et orange du logo InkRiver, tout en préservant le confort de lecture.

### Actions et choix

- centralisation de la palette de marque, des surfaces, des bordures et des
  textes dans les variables CSS globales ;
- adoption d'une barre principale bleu nuit et d'accents orange pour les
  actions, liens, filtres actifs et indicateurs de sélection ;
- remplacement des anciens beiges par des blancs cassés et bleus-gris légers ;
- création d'une déclinaison sombre cohérente avec la même identité ;
- adaptation des couleurs du document isolé des articles, y compris ses liens
  et ses indicateurs de focus ;
- conservation des couleurs sémantiques d'erreur, de succès et de favori.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 96 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Simplification du zoom de texte sur mobile

### Objectif

Alléger la barre d'actions du lecteur mobile en retirant les commandes de taille
de texte devenues secondaires sur un écran tactile.

### Actions et choix

- retrait des boutons loupe moins et loupe plus dans la barre mobile ;
- utilisation systématique de la taille Medium (18 px) pour le contenu des
  articles sur les fenêtres mobiles, indépendamment de la préférence persistée ;
- conservation des trois tailles et de leur préférence sur desktop ;
- suppression des règles CSS mobiles devenues inutiles ;
- ajout d'un test de régression vérifiant l'absence des commandes et la taille
  effective du texte sur mobile.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 97 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Allègement des actions du lecteur mobile

### Objectif

Rendre la barre d'actions mobile plus légère et renforcer la lisibilité de ses
trois commandes principales.

### Actions et choix

- suppression des bordures et fonds permanents des actions Lu/Non lu, Favori
  et Archiver sur mobile, sans réduire leur cible tactile de 44 × 44 px ;
- adoption d'une couleur orange pour la lecture, dorée pour les favoris et
  rouge pour l'archivage ;
- ajout d'un retour visuel discret et teinté lors de l'interaction ;
- ajout de variantes suffisamment lumineuses pour le thème sombre ;
- conservation du style rempli du bouton « Lire l'original », qui reste une
  action primaire distincte.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 97 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Indicateur mobile de rafraîchissement compact

### Objectif

Éviter que l'état du pull-to-refresh surcharge et recouvre la barre de filtres
pendant l'actualisation.

### Actions et choix

- retrait du texte visible « Actualisation en cours » une fois le geste
  déclenché, tout en conservant les indications pendant le tirage ;
- maintien d'un libellé accessible sur l'icône animée via un statut ARIA ;
- transformation de l'indicateur en pastille compacte ;
- déplacement de cette pastille vers la séparation entre les deux premiers
  articles de la chronologie mobile ;
- adaptation du test de pull-to-refresh à ce nouvel état compact.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 97 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Agrandissement de l'indicateur de rafraîchissement

### Objectif

Améliorer la lisibilité de l'icône tournante du rafraîchissement mobile.

### Actions et choix

- passage de l'icône animée de 20 à 22 px, soit une augmentation de 10 % ;
- conservation de la taille actuelle de l'indicateur affiché pendant le geste.

### Vérifications

- `npm run build` : succès ;
- `git diff --check` : succès ;
- tests non réexécutés, le changement étant exclusivement une taille CSS et
  les 97 tests frontend ayant réussi à l'étape précédente.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Navigation mobile simplifiée des abonnements

### Objectif

Remplacer la navigation mobile textuelle redondante par des actions compactes
et accélérer la première actualisation d'un nouvel abonnement.

### Actions et choix

- masquage sur mobile des boutons Articles et Abonnements de la barre
  supérieure, sans modifier la navigation desktop ;
- ajout à droite du logo d'un grand bouton `+` ouvrant directement le formulaire
  et d'une roue dentée ouvrant la gestion des abonnements ;
- ajout dans cette page d'une flèche de retour accessible vers les articles ;
- conservation de la page d'origine après l'ajout d'un flux ;
- déclenchement automatique d'une actualisation ciblée après validation du
  formulaire, avec réutilisation des retours de succès et d'erreur existants ;
- mise à jour des README bilingues et des demandes de fonctionnalités locales ;
- ajout de tests couvrant les nouvelles commandes, le retour et
  l'actualisation automatique.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 98 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Barre de retour fixe des abonnements

### Objectif

Maintenir le retour vers les articles accessible en permanence dans une longue
liste d'abonnements et corriger l'alignement de l'ancien bouton.

### Actions et choix

- transformation de la barre supérieure mobile sur la page Abonnements ;
- affichage exclusif d'une flèche de retour à gauche et du titre `Sources`
  parfaitement centré ;
- retrait de la flèche précédemment intégrée au contenu défilant et du libellé
  `Sources` devenu redondant dans l'en-tête de page ;
- conservation intacte de la barre et de la navigation desktop ;
- adaptation du test de navigation mobile à la barre fixe.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 98 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Titre de la barre des abonnements

### Objectif

Supprimer le libellé générique `Sources` et éviter la répétition du véritable
titre de la page sur mobile.

### Actions et choix

- remplacement du titre centré de la barre fixe par `Gestion des abonnements` ;
- masquage sur mobile du titre et de l'eyebrow du contenu devenus redondants ;
- conservation de l'en-tête complet sur desktop ;
- adaptation du test et de la description de FR-015.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 98 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Recentrage des README sur l'application graphique

### Objectif

Présenter fidèlement InkRiver comme une application Tauri Linux/Android et
clarifier le rôle désormais optionnel du CLI.

### Actions et choix

- réécriture de l'introduction et des fonctionnalités dans les README anglais
  et français pour placer l'application graphique au premier plan ;
- identification explicite du CLI comme outil de développement, de diagnostic
  et d'automatisation non requis par Tauri ;
- clarification du partage du cœur et du schéma, mais pas des bases SQLite ni
  des configurations d'abonnements ;
- déplacement conceptuel de `feeds.toml`, des commandes Cargo et des problèmes
  TOML dans les sections propres au CLI optionnel ;
- correction de la création des bases pour commencer par les répertoires
  AppData des applications installées ;
- actualisation des limites Android et du workflow réel de génération puis
  d'installation d'un APK de debug avec `adb`.

### Vérifications

- `git diff --check` : succès ;
- aucune commande de test exécutée, les changements concernant uniquement la
  documentation.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Logos compacts dans la liste des articles

### Objectif

Réduire les logos parfois flous de la chronologie et libérer la largeur du
titre des articles.

### Actions et choix

- déplacement du logo dans la ligne auteur/date, immédiatement à gauche de
  l'auteur ;
- adoption de la taille de 22 px déjà utilisée par `source-identity` dans le
  header d'article ;
- suppression de l'ancienne colonne de logo de 48 px sur desktop et de 54 px
  sur mobile afin que le titre commence au bord gauche du contenu ;
- conservation de la date, de la barre d'actions et de la coche de
  multisélection ;
- adaptation du test de structure de la liste à la nouvelle disposition.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 98 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Validation des logos compacts sur Android

### Objectif

Appliquer et contrôler dans l'émulateur Android la nouvelle disposition déjà
validée sur desktop.

### Actions et choix

- conservation de la règle responsive mobile supprimant l'ancienne colonne de
  logo de 54 px ;
- redéploiement de l'application de développement sur l'émulateur actif en
  réutilisant le serveur Vite de la session desktop ;
- contrôle visuel de la liste après installation : logos compacts à gauche de
  l'auteur, titres alignés au bord gauche, dates et actions inchangées ;
- maintien de la session Android active pour les essais manuels.

### Vérifications

- `npm run tauri android dev -- --config '{"build":{"beforeDevCommand":null}}' --no-dev-server-wait` : compilation, installation et lancement réussis ;
- capture d'écran ADB après redéploiement : nouvelle disposition mobile
  correctement affichée ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Verrouillage pendant le pull-refresh

### Objectif

Empêcher toute interaction avec l'application mobile pendant l'actualisation
déclenchée par un geste de pull-refresh.

### Actions et choix

- ajout d'un état distinct pour identifier une actualisation provenant du
  pull-refresh, sans changer le comportement des boutons d'actualisation ;
- pose d'un calque transparent couvrant toute l'application, y compris les
  iframes, pendant cette opération ;
- suspension du raccordement des gestionnaires d'interaction tant que le
  verrou est actif et signalement de l'état occupé via `aria-busy` ;
- retrait automatique du verrou dans le bloc de finalisation, en cas de succès
  comme d'erreur ;
- extension du test du pull-refresh pour vérifier la présence du verrou, le
  refus d'une navigation et le retour à l'état interactif.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 98 tests passés ;
- `npm run build` : succès ;
- pull-refresh et tentative de clic contrôlés dans l'émulateur Android actif ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-25 — Navigation mobile entre les articles

### Objectif

Permettre de parcourir les articles depuis le lecteur mobile sans revenir à la
chronologie entre chaque lecture.

### Actions et choix

- ajout d'une barre fixe inférieure `Précédent` / `Suivant`, absente sur
  desktop et respectant les zones sûres Android ;
- navigation dans l'ordre exact de la liste et de la vue active au moment de
  l'ouverture, y compris lorsqu'un article quitte ensuite la vue Non lus ;
- désactivation des boutons aux extrémités et pendant le chargement du nouvel
  article ;
- ajout du swipe horizontal dans le lecteur, avec seuil directionnel, priorité
  au défilement vertical et exclusion des gestes démarrant près des bords ;
- transmission sécurisée des swipes commencés dans l'iframe de contenu et
  mise à jour du hash CSP associé dans l'application et la configuration
  Tauri ;
- décalage du bouton de retour en haut, de la progression et du pied d'article
  afin que la nouvelle barre ne masque aucun contenu ;
- ajout d'un test couvrant boutons, swipes, extrémités, iframe et filtrage par
  vue.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 99 tests passés ;
- `npm run build` : succès ;
- contrôle visuel de la barre puis swipe réel au milieu d'un contenu iframe
  dans l'émulateur Android : passage à l'article suivant réussi ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-26 — Contrat de synchronisation multi-appareils (SYNC-001)

### Objectif

Figer les données synchronisées, leurs identités logiques et les règles de
conflit avant d'introduire le journal SQLite et les migrations de
synchronisation.

### Actions et choix

- audit des mutations d'abonnements, d'états d'article, d'archivage, de
  rétention et des mises à jour techniques du cache ;
- ajout de `docs/synchronization.md`, qui définit les identités d'appareil,
  d'événement, d'abonnement et d'article ainsi que les événements métier de la
  première version ;
- séparation explicite des intentions utilisateur répliquées et des contenus,
  logos, erreurs, extractions et opérations de rétention qui restent locaux ;
- définition d'une fusion champ par champ ordonnée par horloge logique hybride,
  avec alias déterministes pour les ajouts hors ligne de la même URL ;
- définition de pierres tombales permanentes pour les suppressions
  d'abonnements et archivages manuels, et d'une nouvelle incarnation pour une
  réinscription volontaire après suppression ;
- ajout d'une matrice de conflits, du traitement des dépendances absentes, du
  bootstrap des installations existantes, des invariants et des exclusions de
  la version 1 ;
- choix d'un transport initial par répertoire local compatible avec Syncthing,
  avant l'intégration WebDAV pilotée par InkRiver ;
- passage de FR-012 à `en cours` et de SYNC-001 à `terminée` dans le backlog
  local, avec clarification des actions groupées.

### Vérifications

- revue croisée du contrat avec les opérations exposées par `src/storage.rs` et
  le schéma SQLite existant ;
- `git diff --check` et contrôle équivalent du nouveau fichier non suivi :
  succès ;
- aucun test applicatif exécuté, car cette étape ne modifie aucun code de
  production ni schéma de données.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`346bba1` (`[AI] Add synchronization journal foundation`).

## 2026-08-26 — Identité d'appareil et journal SQLite (SYNC-002)

### Objectif

Créer la fondation persistante et transactionnelle de la synchronisation sans
encore journaliser les opérations métier, qui relèvent de SYNC-003.

### Actions et choix

- ajout d'une migration non destructive qui rétroremplit `articles.entry_key`
  et crée les tables d'identité locale, événements, curseurs d'import,
  événements en attente, alias d'abonnements, versions de champs, tombstones et
  identités logiques d'articles ;
- génération d'un UUID d'appareil unique au premier démarrage après migration,
  puis conservation de cette identité dans la base ;
- ajout des types publics `HybridLogicalClock`, `SyncIdentity` et `SyncEvent`
  dans `src/sync.rs` ;
- ajout d'une écriture transactionnelle du journal qui alloue simultanément la
  séquence et l'horloge hybride, sans les consommer lorsque l'insertion échoue ;
- ajout d'une lecture locale ordonnée après curseur, bornée à 1 000 événements
  par appel ;
- conservation d'un format de payload JSON temporairement ouvert, qui sera
  remplacé par les événements métier typés dans SYNC-003 ;
- alimentation de `entry_key` lors des futurs upserts RSS, indépendamment de
  l'identifiant SQLite préfixé par le flux ;
- passage de SYNC-002 à `terminée` dans le backlog local.

### Vérifications

- tests de migration : abonnements, articles, états lu/favori, archivage,
  extraction et logo conservés ; clé d'entrée correctement rétroremplie ;
- tests d'identité et de reprise après redémarrage : succès ;
- tests d'horloge avec recul de l'heure murale, écritures concurrentes,
  unicité, lecture paginée et rollback sans trou de séquence : succès ;
- `cargo fmt --all -- --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` hors sandbox : succès, 167 tests passés et 1 test de
  corpus local ignoré ; la première exécution isolée avait empêché cinq
  serveurs HTTP de test d'ouvrir un port local ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `git diff --check` et contrôle équivalent des nouveaux fichiers non suivis :
  succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`346bba1` (`[AI] Add synchronization journal foundation`).

## 2026-08-27 — Production transactionnelle des événements métier (SYNC-003)

### Objectif

Relier le journal de synchronisation aux mutations utilisateur sans modifier le
comportement des installations qui n'ont pas encore activé la synchronisation.

### Actions et choix

- ajout de payloads métier typés pour les abonnements, la lecture, les favoris
  et l'archivage, avec discriminateurs de protocole stables ;
- ajout d'un indicateur d'activation désactivé par défaut et d'un bootstrap
  atomique et idempotent des abonnements et états d'articles existants ;
- journalisation dans les transactions métier d'ajout, réactivation,
  activation, suppression d'abonnement, lecture, favori et archivage manuel ;
- production d'un événement par article pour les actions groupées, sans état
  partiel ni événement orphelin en cas d'échec ;
- conservation hors journal des imports CLI, actualisations réseau,
  métadonnées de cache et archivages automatiques ;
- enregistrement des alias, identités logiques et pierres tombales nécessaires
  à la future fusion déterministe ;
- mise à jour du contrat pour formaliser le changement de plateforme comme un
  registre indépendant et préciser l'activation explicite de la synchro.

### Vérifications

- tests d'activation absente, de bootstrap idempotent et de migration depuis le
  schéma SYNC-002 ;
- tests de cardinalité des événements unitaires et groupés, de rollback
  transactionnel, de réabonnement après suppression et d'absence d'événements
  pour les écritures techniques ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` hors sandbox : succès, 175 tests passés et 1 test de
  corpus local ignoré ; la première exécution isolée avait empêché cinq
  serveurs HTTP de test d'ouvrir un port local ;
- `cargo fmt --all -- --check` : succès ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `git diff --check` et contrôle équivalent de la nouvelle migration non
  suivie : succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`9b771e2` (`[AI] Journal synchronized business mutations`).

## 2026-08-27 — Fusion déterministe des événements (SYNC-004)

### Objectif

Importer des événements issus d'autres appareils de façon atomique,
idempotente et indépendante de leur ordre d'arrivée, sans générer d'écho dans
le journal local.

### Actions et choix

- ajout d'une API d'import bornée à 1 000 événements et d'un rapport composé
  uniquement de compteurs non sensibles ;
- validation préalable des enveloppes, versions de protocole, horloges,
  identifiants, URL normalisées, dates et tailles de champs ;
- détection des doublons identiques et rejet transactionnel des collisions sur
  une même identité `(device_id, sequence)` ;
- application de registres last-writer-wins selon l'ordre total de l'horloge
  hybride, avec intégration des versions produites par les mutations locales ;
- traitement permanent des suppressions d'abonnements et archivages manuels,
  y compris lorsque des événements plus récents arrivent ensuite ;
- résolution déterministe des alias d'abonnements concurrents, des nouvelles
  incarnations et des suppressions parentes concurrentes ;
- conservation et nouvelle tentative des événements dont les dépendances sont
  absentes, avec création d'articles locaux limités aux métadonnées ;
- avancement transactionnel de l'horloge locale et des curseurs contigus, sans
  appel aux opérations métier journalisées lors d'un import ;
- mise à jour du contrat et passage de SYNC-004 à `terminée` dans le backlog
  local.

### Vérifications

- tests ciblés : dépendances inversées, absence d'écho, doublons, collisions,
  rollback, réinscription, suppressions concurrentes et comparaison avec les
  versions locales ;
- tests exhaustifs des 24 permutations des registres lu/favori et des 24
  permutations incluant une archive permanente : succès ;
- `cargo test --workspace` hors sandbox : succès, 184 tests passés et 1 test de
  corpus local ignoré ;
- `cargo fmt --all -- --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `git diff --check` et contrôle équivalent du nouveau module non suivi :
  succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`a7b43a8` (`[AI] Implement deterministic event merging`).

## 2026-08-27 — Segments de synchronisation en répertoire local (SYNC-005)

### Objectif

Permettre un échange manuel, hors réseau et compatible avec un répertoire
Syncthing, sans copier la base SQLite et avant l'ajout du chiffrement.

### Actions et choix

- ajout d'un format JSON versionné sous `v1/<device UUID>/`, limité à 250
  événements contigus et 2 Mio par segment ;
- ajout d'une empreinte SHA-256 couvrant l'en-tête et les événements pour
  détecter les corruptions accidentelles, avec avertissement explicite que les
  segments restent lisibles et non authentifiés jusqu'à SYNC-006 ;
- ajout d'un curseur SQLite persistant afin d'exporter uniquement les nouveaux
  événements locaux ;
- publication sans écrasement, fichiers temporaires synchronisés et reprise du
  cas où le fichier existe déjà mais où le curseur n'avait pas été avancé ;
- validation préalable de tout le répertoire : versions, chemins, types de
  fichiers, tailles, JSON, sommes, plages, appareils et cohérence des événements ;
- import atomique borné à 1 000 événements et refus des versions inconnues,
  liens symboliques et entrées inattendues ;
- ajout d'un scénario complet Linux vers Android simulé puis retour Linux,
  avec exports et imports répétés jusqu'à convergence ;
- vérification qu'aucun corps HTML, fichier SQLite, WAL ou SHM n'est exporté ;
- passage de SYNC-005 à `terminée` et documentation du format et de ses limites.

### Vérifications

- sept tests ciblés d'export, découpage, reprise, validation atomique,
  corruption, limites, progression multi-lots et convergence : succès ;
- `cargo fmt --all -- --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : 191 tests réussis et un test de corpus local
  ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès après correction d'une suggestion de style ;
- `git diff --check` et contrôles équivalents des nouveaux fichiers non suivis :
  succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`2ce9fc6` (`[AI] Add encrypted-ready sync directory segments`).

## 2026-08-27 — Chiffrement authentifié des segments (SYNC-006)

### Objectif

Rendre les segments de synchronisation confidentiels et détecter toute
altération avant qu'un événement distant puisse modifier SQLite.

### Actions et choix

- ajout d'une clé de groupe aléatoire de 256 bits, masquée dans `Debug` et
  effacée de sa mémoire possédée lors de sa destruction ;
- chiffrement des charges utiles avec XChaCha20-Poly1305 et un nonce aléatoire
  de 192 bits par segment ;
- authentification de la version, du protocole, de l'empreinte de clé, de
  l'appareil et de la plage de séquences comme données associées ;
- remplacement du répertoire en clair par une enveloppe `v2` ne révélant ni
  titre, ni URL, ni état utilisateur ;
- ajout de curseurs d'export SQLite par empreinte de clé afin qu'une future
  rotation puisse republier l'historique sans réutiliser l'ancien curseur ;
- conservation de la publication immuable et de la reprise après interruption,
  y compris lorsque le nouveau chiffrement produit un nonce différent ;
- refus atomique des mauvaises clés, en-têtes ou contenus altérés ;
- mise à jour du contrat de synchronisation et passage de SYNC-006 à
  `terminée` dans le backlog local.

Le stockage sûr et l'appairage de la clé restent volontairement réservés à
SYNC-008 ; ce module exige que l'appelant lui fournisse la clé.

### Vérifications

- dix tests ciblés des segments, dont génération et masquage de clé,
  confidentialité, unicité des nonces, mauvaise clé, altérations authentifiées,
  reprise, limites et convergence : succès ;
- sept tests ciblant les migrations SQLite : succès ;
- `cargo fmt --all -- --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo check -p inkriver --target x86_64-linux-android` : succès avec le
  compilateur et l'archiveur du NDK explicitement configurés ; la première
  tentative ne fournissait que le linker et `aws-lc-sys` ne trouvait donc pas
  le compilateur C Android ;
- `cargo test --workspace` : 194 tests réussis et un test de corpus local
  ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `git diff --check` et contrôle équivalent de la nouvelle migration : succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`b96c599` (`[AI] Encrypt and authenticate sync segments`).

## 2026-08-27 — Transport de synchronisation WebDAV (SYNC-007)

### Objectif

Échanger les segments chiffrés via un serveur WebDAV sans serveur InkRiver,
avec publication immuable, reprises sûres et limites réseau explicites.

### Actions et choix

- séparation de la préparation, de la confirmation d'export et de l'import des
  segments afin que le système de fichiers et WebDAV utilisent le même cœur ;
- ajout d'une interface de transport qui ne reçoit ni clé de groupe ni contenu
  déchiffré ;
- confirmation du curseur SQLite uniquement après création distante ou après
  vérification cryptographique d'un segment immuable déjà présent ;
- création des collections version, clé et appareil avec `MKCOL` ;
- publication atomique par `PUT` temporaire puis `MOVE` avec `Overwrite: F`,
  et reprise du cas où la réponse du `MOVE` est perdue après son application ;
- découverte bornée avec `PROPFIND`, téléchargement des seuls segments absents
  par lots de 20 et concurrence maximale de quatre ;
- limites sur les délais, corps, entrées et chemins, avec redirections HTTP
  désactivées pour éviter de transmettre les identifiants à un autre endpoint ;
- mot de passe effacé avec son propriétaire et masqué dans les sorties `Debug` ;
- ajout d'un faux serveur WebDAV local couvrant authentification, collections,
  upload, move, listing, téléchargement, doublons et interruption ;
- passage de SYNC-007 à `terminée` et documentation des garanties et de la
  limite actuelle : secrets et interface restent prévus par SYNC-008/SYNC-009.

### Vérifications

- 32 tests ciblés de synchronisation : succès ;
- scénario WebDAV Linux vers Android simulé puis retour Linux, avec réponse de
  publication perdue et retry : succès ;
- tests des redirections refusées, réponses trop grandes, indisponibilité,
  confidentialité, limitation à quatre téléchargements et lots de 20 : succès ;
- `cargo fmt --all -- --check` : succès ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` : 200 tests réussis et un test de corpus local
  ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `cargo check -p inkriver --target x86_64-linux-android` avec le NDK : succès ;
- `git diff --check` et contrôles équivalents des deux nouveaux modules :
  succès.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`849c16d` (`[AI] Add the WebDAV sync transport`).

## 2026-08-28 — Socle des secrets et de l'appairage (SYNC-008)

### Objectif

Préparer l'appairage Linux/Android sans compte InkRiver, sans stocker la clé de
groupe ou le mot de passe WebDAV en clair dans SQLite.

### Actions et choix

- ajout d'un format d'invitation versionné, strictement borné et transportable
  par URI `inkriver://pair/`, avec génération d'un QR SVG entièrement hors
  ligne ;
- exclusion explicite du mot de passe WebDAV de l'invitation : il doit être
  fourni séparément sur le nouvel appareil ;
- ajout des workflows de création de groupe, d'export d'invitation et
  d'acceptation sur une installation vierge, avec refus d'écraser une
  configuration existante ;
- ajout d'une abstraction de coffre stockant en une seule entrée la clé de
  groupe et le mot de passe WebDAV, adossée à Secret Service sous Linux et à
  Android Keystore sous Android ;
- effacement explicite des buffers secrets transitoires et masquage des sorties
  `Debug` ;
- migration SQLite pour les seuls réglages non secrets et les métadonnées
  d'appareils ; renommage et révocation logique sans changement de l'identité
  UUID immuable ;
- filtrage des futurs segments provenant d'un appareil révoqué, tout en
  conservant son historique déjà fusionné ;
- documentation bilingue de la frontière de sécurité et du travail restant :
  affichage/scanner et écran Tauri seront raccordés avec SYNC-009 ; SYNC-008
  reste donc marquée `en cours`.

### Vérifications

- `cargo fmt --all -- --check` : succès après formatage ;
- `cargo check --workspace` : succès ;
- `cargo test --workspace` hors sandbox : 211 tests réussis et un test de
  corpus local ignoré ;
- tests ajoutés : round-trip et validation de l'invitation, QR autonome,
  absence de mot de passe WebDAV, coffre mémoire, appairage Linux vers Android
  simulé, refus d'écrasement, migration, renommage, révocation et rejet des
  segments révoqués ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `cargo check -p inkriver --target x86_64-linux-android` avec le compilateur et
  l'archiveur du NDK : succès ;
- aucun accès Internet réel dans les tests ; les serveurs HTTP/WebDAV sont
  locaux et contrôlés.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-28 — Interface d'appairage Linux/Android (fin de SYNC-008)

### Objectif

Rendre le socle d'appairage utilisable depuis Tauri afin qu'un appareil Android
vierge puisse rejoindre le groupe configuré sur Linux, sans compte InkRiver et
sans inclure le mot de passe WebDAV dans l'invitation.

### Actions et choix

- ajout des commandes Tauri d'état, création de groupe, génération
  d'invitation, adhésion, renommage et révocation logique d'un appareil, avec
  DTO camelCase et erreurs structurées ;
- ajout d'une boîte de dialogue dans la gestion des abonnements : formulaire
  de configuration initiale, QR confidentiel généré hors ligne, import manuel,
  liste et gestion des appareils ;
- intégration mobile du plugin officiel Tauri Barcode Scanner, limité aux QR
  codes et soumis à l'autorisation caméra Android ; l'import manuel reste le
  repli sur toutes les plateformes ;
- enregistrement du plugin uniquement sur mobile et ajout de sa capability
  minimale, sans accès caméra ni réseau depuis l'interface desktop ;
- conservation du mot de passe WebDAV dans le coffre natif et saisie séparée
  sur le nouvel appareil ; l'invitation transporte uniquement la clé de groupe
  et les réglages nécessaires ;
- initialisation idempotente du coffre natif afin d'éviter de remplacer
  plusieurs fois son backend global ;
- mise à jour des README bilingues et passage de SYNC-008 à `terminée` dans le
  backlog local. L'exécution du transport depuis l'interface reste réservée à
  SYNC-009.

### Vérifications

- `cargo fmt --all -- --check` et `cargo check --workspace` : succès ;
- `cargo test --workspace` hors sandbox : 212 tests réussis, corpus local
  optionnel ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck`, `npm test -- --run` (102 tests) et `npm run build` :
  succès ;
- compilation croisée de `inkriver-app` pour `x86_64-linux-android`, incluant
  le plugin de scan QR : succès ;
- `git diff --check` : succès ; aucun accès Internet réel dans les tests.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce stade.

## 2026-08-28 — Premier parcours manuel de synchronisation (début de SYNC-009)

### Objectif

Raccorder le transport WebDAV existant à Tauri et fournir un premier parcours
manuel qui conserve l'application et son cache utilisables en cas d'échec.

### Actions et choix

- commit autonome de la fin de SYNC-008 : `ee77d63`
  (`[AI] Complete secure device pairing`) ;
- ajout de `sync_runtime`, service du cœur qui charge la configuration SQLite,
  lit les secrets du coffre, vérifie l'empreinte de clé puis construit et lance
  le transport WebDAV ;
- ajout d'une variante à transport injecté pour tester l'orchestration sans
  réseau et couvrir configuration absente, secrets absents et clé incohérente ;
- ajout de la commande Tauri `synchronize_now`, d'un verrou non bloquant dédié,
  d'erreurs structurées et d'un DTO camelCase pour les compteurs du cycle ;
- ajout du bouton manuel dans la boîte de dialogue : la vue courante reste
  ouverte, les articles, abonnements et l'article sélectionné sont rechargés
  après fusion, tandis qu'une erreur détaillée laisse le cache intact ;
- passage de SYNC-009 à `en cours` et documentation bilingue de ce premier lot.
  La suppression de configuration, la persistance du dernier état et les
  finitions d'affichage restent à réaliser.

### Vérifications

- `cargo fmt --all -- --check`, `cargo check --workspace` et tests ciblés :
  succès ;
- `cargo test --workspace` hors sandbox : 216 tests réussis, corpus local
  optionnel ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck`, `npm test -- --run` (104 tests) et `npm run build` :
  succès ;
- compilation croisée de `inkriver-app` pour `x86_64-linux-android` : succès ;
- aucun accès Internet réel dans les tests ; les serveurs réseau sont locaux
  et contrôlés.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
au premier lot SYNC-009 à ce stade.

## 2026-08-28 — Finalisation de SYNC-009

### Objectif

Achever le parcours manuel de synchronisation Tauri en rendant son état durable,
diagnostiquable et supprimable localement, sans compromettre les données du
lecteur ni les fichiers WebDAV distants.

### Actions et choix

- ajout d'une migration SQLite dédiée à la dernière tentative, la dernière
  réussite, ses compteurs et la dernière erreur détaillée de synchronisation ;
- finalisation du service d'orchestration partagé : chargement et contrôle de
  la configuration, exécution du transport, persistance atomique du résultat
  et conservation de la réussite précédente en cas d'échec ultérieur ;
- ajout des commandes Tauri de synchronisation et de suppression de la
  configuration, protégées par un verrou non bloquant commun ;
- ajout dans l'interface de l'état en cours, du succès complet ou partiel, des
  compteurs durables, de l'erreur détaillée et d'une zone de suppression avec
  confirmation obligatoire ;
- la suppression retire les secrets natifs et les métadonnées locales
  d'appairage, mais conserve les abonnements, articles, états locaux, identité
  de l'appareil, journal de changements et fichiers WebDAV distants ;
- mise à jour des README bilingues et passage de SYNC-009 à `terminée` dans le
  backlog local.

### Vérifications

- `cargo fmt --all -- --check` et `cargo check --workspace` : succès ;
- `cargo test --workspace` : 219 tests réussis et un test de corpus local
  optionnel ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `npm run typecheck`, `npm test -- --run` (106 tests) et `npm run build` :
  succès ;
- compilation croisée de `inkriver-app` pour `x86_64-linux-android` : succès ;
- `git diff --check` : succès ; aucun accès Internet réel dans les tests.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`b0f100f` (`[AI] Expose manual synchronization in Tauri`).

## 2026-08-28 — Automatisation prudente de la synchronisation (SYNC-010)

### Objectif

Faire converger les appareils sans action explicite tout en conservant un mode
manuel, une consommation réseau bornée et un comportement adapté au cycle de
vie mobile.

### Actions et choix

- ajout d'une préférence opt-in propre à l'appareil, conservée dans le stockage
  local de la WebView avec repli sûr vers le mode manuel lorsqu'il est
  indisponible ;
- ajout d'un ordonnanceur TypeScript isolé et testable qui synchronise au
  démarrage ou au retour au premier plan et regroupe pendant cinq secondes les
  mutations locales rapprochées ;
- suspension des nouveaux départs lorsque la WebView est hors ligne ou masquée,
  puis reprise sur les événements réseau et de visibilité ;
- ajout d'un budget borné de quatre nouvelles tentatives, espacées de 30
  secondes, 2 minutes, 10 minutes et 30 minutes ;
- réutilisation de la commande et du verrou Tauri de SYNC-009, sans service
  Android en arrière-plan ni boucle déclenchée par les projections importées ;
- ajout du réglage dans la boîte de dialogue, rechargement silencieux des
  articles et abonnements après succès, documentation bilingue et passage de
  SYNC-010 à `terminée` dans le backlog local.

### Vérifications

- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `cargo test --workspace` hors sandbox : 219 tests réussis et un corpus local
  optionnel ignoré ; le premier lancement en sandbox avait correctement isolé
  huit refus d'ouverture de sockets de boucle locale ;
- `npm run typecheck`, `npm test -- --run` (116 tests) et `npm run build` :
  succès ;
- compilation croisée de `inkriver-app` pour `x86_64-linux-android` et
  `git diff --check` : succès ; aucun accès Internet réel dans les tests.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`8a15ca0` (`[AI] Add prudent automatic synchronization`).

## 2026-08-28 — Ré-audit de complétude de SYNC-010

### Objectif

Reprendre l'implémentation après interruption et vérifier chaque critère
d'acceptation avant de confirmer la fin de SYNC-010.

### Actions et choix

- comparaison des producteurs d'événements du cœur avec tous les parcours de
  mutation de l'interface et confirmation de leur raccordement au debounce ;
- identification d'une course : une modification locale effectuée pendant un
  cycle automatique pouvait ne pas programmer le cycle suivant ;
- ajout d'une tentative différée mémorisée par l'ordonnanceur, exécutée après
  libération du cycle courant, sans concurrence ;
- fermeture de la fenêtre entre la lecture asynchrone du statut et la prise du
  marqueur frontend `syncBusy`, le verrou Tauri restant l'autorité finale ;
- ajout de tests prouvant la reprise de la mutation concurrente, une concurrence
  maximale de un et l'absence de boucle après rechargement des projections.

### Vérifications

- `npm run typecheck`, `npm test -- --run` (117 tests) et `npm run build` :
  succès ;
- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `git diff --check` : succès. La suite Rust complète, inchangée depuis sa
  précédente exécution, reste à 219 tests réussis et un test local ignoré.

Ces changements ont été créés avec l'assistance d'une IA. Le correctif de
ré-audit est inclus dans le commit `8a15ca0`.

## 2026-08-28 — Socle des accusés de réception (début de SYNC-011)

### Objectif

Préparer une compaction démontrablement sûre sans supprimer prématurément les
segments nécessaires à un appareil en retard.

### Actions et choix

- ajout d'une migration SQLite pour une matrice d'accusés de réception par
  empreinte de clé, appareil observateur et appareil source ;
- ajout d'écritures monotones : une observation ancienne ne peut jamais faire
  reculer une séquence contiguë déjà connue ;
- ajout d'un calcul de frontière conservatrice exigeant une liste explicite et
  complète d'appareils : un accusé absent contribue zéro et une affirmation
  excessive est bornée à la dernière séquence source connue ;
- prise en compte directe des curseurs locaux et déduplication déterministe de
  la liste des appareils requis ;
- suppression de ces accusés lors du retrait de la configuration locale afin
  qu'un autre groupe ou une autre clé ne réutilise jamais un ancien seuil ;
- maintien volontaire de toute suppression à l'arrêt : une liste distribuée
  des membres, le transport authentifié des accusés et les instantanés doivent
  encore être conçus avant la compaction effective ;
- passage de SYNC-011 à `en cours` dans le backlog local.

### Vérifications

- tests ciblés de migration, monotonie, isolation de groupe, frontière bloquée
  et nettoyage de configuration : succès ;
- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` :
  succès ;
- `cargo test --workspace` hors sandbox : 221 tests réussis et un corpus local
  optionnel ignoré ;
- compilation croisée de `inkriver-app` pour `x86_64-linux-android` et
  `git diff --check` : succès ; aucun accès Internet réel dans les tests.

Ces changements ont été créés avec l'assistance d'une IA. Commit associé :
`e0b99ab` (`[AI] Add safe synchronization acknowledgements`).

## 2026-08-29 — Transport chiffré des accusés SYNC-011

### Objectif

Faire circuler entre appareils les positions contiguës préparées par le premier
lot de SYNC-011, sans autoriser encore aucune suppression d'historique.

### Actions et choix

- ajout d'un document d'accusé par appareil, séparé des journaux afin d'éviter
  une boucle infinie où chaque accusé produirait lui-même un nouvel événement à
  acquitter ;
- chiffrement et authentification XChaCha20-Poly1305 avec la clé de groupe,
  métadonnées de chemin authentifiées, taille limitée à 256 Kio et au plus 256
  journaux sources ;
- publication WebDAV par remplacement atomique, puis téléchargement borné des
  documents distants après l'import des segments ;
- validation complète avant écriture atomique dans SQLite, conservation
  monotone des positions et rejet des accusés provenant d'un appareil révoqué ;
- adaptation du plan de contrôle WebDAV et de l'import par répertoire afin que
  le sous-répertoire `acknowledgements` ne soit jamais interprété comme un
  journal d'appareil ;
- documentation bilingue et mise à jour du backlog local. Les instantanés, la
  liste de membres faisant autorité et toute suppression restent à réaliser.

### Vérifications

- tests ciblés de chiffrement, mauvaise clé, chemin incohérent, atomicité
  SQLite, échange Linux/Android simulé, frontière sûre et compatibilité de
  l'import par répertoire : succès ;
- test WebDAV local avec deux clients, remplacement atomique interrompu et
  deux documents d'accusé : succès ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès ;
- `cargo test --workspace` : 226 tests réussis (202 cœur, 2 CLI et 22 Tauri),
  un corpus local optionnel ignoré ; aucun accès Internet réel ;
- la vérification Cargo Android directe a atteint les plugins Tauri mais n'a
  pas pu se terminer : leur script de build refuse un répertoire `.tauri`
  préexistant dans le cache Cargo. La suite Linux et le cœur partagé sont verts.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce deuxième lot SYNC-011.

## 2026-08-29 — Instantanés de récupération (suite de SYNC-011)

### Objectif

Permettre à une base neuve ou à un appareil ayant perdu un segment de
reconstruire les projections avant toute activation de la compaction.

### Actions et choix

- ajout d'une migration mémorisant la dernière empreinte d'instantané publiée
  et importée par appareil et clé de groupe ;
- ajout d'un format versionné XChaCha20-Poly1305 contenant uniquement les
  préfixes contigus des journaux et leurs événements, sans corps d'article en
  cache ;
- authentification du créateur, de la clé, de l'empreinte d'état et du chemin,
  avec limites de 256 appareils, 10 000 événements, 5 Mio d'état et 8 Mio pour
  le document chiffré ;
- publication WebDAV atomique uniquement lorsque l'empreinte de l'état change ;
  un instantané dépassant les limites est omis sans faire échouer le cycle ;
- import transactionnel sur une base neuve et reprise des segments suivants ;
  détection d'un trou de séquence et réparation depuis l'instantané distant le
  plus récent disponible ;
- adaptation des transports simulés, du plan de contrôle WebDAV et de l'import
  par répertoire aux nouveaux fichiers `snapshots` ;
- correction du bornage multi-appareils : jusqu'à 256 instantanés peuvent être
  découverts sans bloquer le cycle, avec huit téléchargements prioritaires au
  maximum par synchronisation ;
- maintien explicite de toute suppression à l'arrêt : il manque encore une
  liste distribuée des membres faisant autorité.

### Vérifications

- tests ciblés : reconstruction sans contenu en cache, idempotence, mauvaise
  clé, corruption, absence de republication inchangée, instantané trop grand
  non bloquant, reprise avec segments suivants et réparation d'un appareil en
  retard après perte d'un segment : succès ;
- test WebDAV local à deux clients : démarrage depuis l'instantané, convergence,
  remplacement atomique interrompu et confidentialité du contenu : succès ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès ;
- `cargo test --workspace -q` : succès, 232 tests passés et un test de corpus
  local optionnel ignoré.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce troisième lot SYNC-011.

## 2026-08-29 — Diagnostic de synchronisation expurgé (suite de SYNC-011)

### Objectif

Fournir un premier export exploitable pour le support sans exposer les données
personnelles ni les secrets de synchronisation.

### Actions et choix

- ajout d'un document JSON versionné contenant l'état d'activation, les dates,
  les compteurs de journaux, appareils, accusés et instantanés, ainsi que le
  dernier rapport agrégé ;
- exclusion volontaire de la clé et de son empreinte, des identifiants WebDAV,
  des URL, noms et identifiants d'appareils, des abonnements, articles et corps
  en cache ; le message libre de la dernière erreur est également omis ;
- ajout de la commande hors ligne `sync-diagnostic`, exportable par redirection
  de sa sortie standard ;
- tests vérifiant le format JSON et l'absence de valeurs sensibles témoins.

### Vérifications

- `cargo test sync_diagnostic --workspace -q` : succès, 2 tests passés ;
- `cargo test --workspace -q` : succès, 234 tests passés et un test de corpus
  local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce quatrième lot SYNC-011.

## 2026-08-29 — Registre distribué des appareils (suite de SYNC-011)

### Objectif

Remplacer la liste locale issue de l'appairage par une autorité distribuée et
conservatrice utilisable pour calculer les futures frontières de compaction.

### Actions et choix

- ajout d'une migration SQLite stockant les membres par empreinte de groupe et
  leurs pierres tombales de révocation ;
- ajout d'un format de registre versionné, chiffré et authentifié avec
  XChaCha20-Poly1305, limité à 256 membres et 256 Kio ;
- publication atomique d'un document par appareil et fusion transactionnelle
  des registres avant tout import de données ;
- adoption d'un ensemble monotone en deux phases : une appartenance n'est
  jamais oubliée et une révocation ne peut jamais être annulée, même par un
  document plus récent ne la contenant pas ;
- application des révocations distribuées aux segments, instantanés et accusés,
  avec arrêt explicite lorsqu'un appareil apprend sa propre révocation ;
- ajout d'un calcul de frontière autoritaire qui dérive obligatoirement ses
  observateurs du registre actif et exclut uniquement les UUID révoqués ;
- isolation du nouveau répertoire de contrôle pour les imports par répertoire,
  les transports mémoire et WebDAV ;
- maintien volontaire de la compaction destructive à l'arrêt pendant son audit
  final.

### Vérifications

- tests ciblés du chiffrement, de la mauvaise clé, de la corruption, de
  l'atomicité SQLite, de la monotonie, de la révocation inter-appareils et de la
  frontière autoritaire : succès ;
- test WebDAV local de convergence et publication atomique : succès ;
- `cargo test --workspace -q` : succès, 240 tests passés et un test de corpus
  local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` et
  `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce cinquième lot SYNC-011.

## 2026-08-29 — Commit des lots de récupération SYNC-011

### Objectif

Figer ensemble les accusés distribués, les instantanés de récupération, le
diagnostic expurgé et le registre autoritaire après leur validation commune.

### Actions et vérifications

- sélection exclusive des 16 fichiers de production, migrations et README ;
- exclusion confirmée de `app/env.txt`, `logo_raw.png`, `src/call_graph.png` et
  `src/uml.png` ;
- `git diff --cached --check` : succès ;
- commit créé : `6fda8af` (`[AI] Add sync recovery and distributed roster`).

Ces changements ont été créés avec l'assistance d'une IA.

## 2026-08-29 — Découpage des lots restants de SYNC-011

### Objectif

Rendre visible le travail restant avant la clôture de SYNC-011 et fournir une
checklist exploitable pour les prochaines sessions d'implémentation.

### Actions et choix

- création de `SYNC-011.md` à la racine du dépôt ;
- rappel du socle de synchronisation et de récupération déjà disponible ;
- découpage du reliquat en six lots : compaction locale, suppression WebDAV,
  reprise après interruption, compatibilité de protocole, diagnostic graphique
  et documentation opérationnelle ;
- ajout de critères de sortie, d'une validation finale et d'un ordre recommandé ;
- maintien explicite de la règle conservatrice : aucune donnée n'est supprimée
  si le checkpoint, le registre ou les accusés ne permettent pas de prouver que
  l'opération est sûre.

### Vérifications

- relecture de la checklist et comparaison avec l'état courant de SYNC-011 ;
- `git diff --check` : succès ;
- aucun test de code exécuté, cette intervention étant exclusivement
  documentaire.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-08-29 — Compaction locale sûre (lot 8 de SYNC-011)

### Objectif

Réduire progressivement le journal SQLite sans compromettre la récupération ni
la convergence d'un appareil actif.

### Actions et choix

- ajout d'une compaction transactionnelle limitée à 1 000 événements par cycle ;
- calcul, dans la transaction de suppression, de la frontière sûre de chaque
  journal à partir du registre distribué actif et des accusés monotones ;
- conservation systématique des créations d'abonnements, gagnants LWW, pierres
  tombales et dépendances en attente ;
- déclenchement uniquement après publication ou nouvelle authentification du
  checkpoint distant ;
- ajout du compteur `compactedEvents` au rapport courant, à sa persistance
  SQLite, au diagnostic expurgé et aux DTO Rust/TypeScript ;
- ajout de la migration `202608290003_sync_compaction_report.sql` ;
- mise à jour des README bilingues et de la checklist `SYNC-011.md`.

### Vérifications

- tests ciblés de la limite, de l'idempotence, de la conservation du checkpoint
  et des projections, du blocage par un appareil actif et de l'absence de
  suppression après échec de publication : succès ;
- `cargo fmt --all -- --check` et `cargo check --workspace` : succès ;
- `cargo test --workspace` : succès, 223 tests cœur, 2 tests CLI et 22 tests
  Tauri passés ; un test de corpus local optionnel ignoré ;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck`, `npm test` (117 tests) et `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Commit des lots 6 à 9 de SYNC-011

### Objectif

Figer le checkpoint compact, sa confirmation distante, la compaction SQLite et
le nettoyage WebDAV avant les essais de reprise après interruption.

### Actions et vérifications

- sélection des sources Rust, migrations, DTO, tests, README et de
  `SYNC-011.md` ;
- exclusion de `app/env.txt`, `logo_raw.png`, `src/call_graph.png` et
  `src/uml.png` ;
- `git diff --cached --check` : succès ;
- commit créé : `cd17d88` (`[AI] Add safe sync compaction and recovery cleanup`).

Ces changements ont été créés avec l'assistance d'une IA.

## 2026-08-29 — Nettoyage sûr des segments WebDAV (lot 9 de SYNC-011)

### Objectif

Supprimer progressivement les segments distants devenus récupérables depuis un
checkpoint sans pouvoir priver un appareil actif de données nécessaires.

### Actions et choix

- ajout au contrat de transport d'une suppression de segment idempotente ;
- validation WebDAV stricte du chemin `v2/<clé>/<appareil>/<plage>.json`, avec
  succès également lorsque le fichier est déjà absent ;
- nouvelle vérification par téléchargement et authentification de tout
  checkpoint publié ou réparé avant d'autoriser une suppression ;
- transmission des frontières exactes du checkpoint à la compaction SQLite et
  au nettoyage distant ;
- suppression séquentielle d'au plus 20 segments par cycle, exclusivement dans
  le journal de l'appareil courant et uniquement si leur fin précède la
  frontière autoritaire ;
- conservation intégrale des segments traversant la frontière ;
- arrêt silencieux du nettoyage lors d'une erreur WebDAV, sans invalider
  l'import réussi, puis nouvelle tentative lors d'un cycle ultérieur ;
- ajout des compteurs `deletedSegments` et `deferredSegmentDeletions` aux
  rapports persistés, diagnostics et DTO, avec migration SQLite dédiée ;
- mise à jour des README bilingues et de `SYNC-011.md`.

### Vérifications

- tests ciblés : suppression idempotente, rejet des chemins hors segment,
  frontière au milieu d'un segment, propriété du journal, plafond de 20 et
  reprise après erreur WebDAV : succès ;
- `cargo test --workspace` : succès, 228 tests cœur, 2 tests CLI et 22 tests
  Tauri passés ; un test de corpus local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck`, `npm test` (117 tests) et `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Reprise après interruption et corruption (lot 10 de SYNC-011)

### Objectif

Prouver que chaque interruption entre checkpoint, compaction locale et
nettoyage distant laisse un état récupérable et convergent.

### Actions et choix

- ajout d'un scénario de redémarrage après publication du checkpoint mais avant
  compaction SQLite ;
- simulation d'une interruption au milieu de la transaction de compaction et
  vérification du rollback intégral ;
- simulation d'une coupure entre plusieurs suppressions de segments, suivie
  d'une reprise limitée aux fichiers restants ;
- vérification qu'un checkpoint distant manquant doit être réparé avant qu'un
  segment nouvellement sûr puisse être supprimé ;
- reconstruction d'une base neuve depuis le seul checkpoint après disparition
  de tous les anciens segments couverts ;
- rattachement explicite des tests existants de reprise des segments suivants,
  d'appareil actif sans accusé et de révocation définitive aux critères du lot ;
- passage de toutes les tâches du lot 10 à terminées dans `SYNC-011.md`.

### Vérifications

- tests ciblés des cinq nouveaux scénarios : succès ;
- `cargo test --workspace` : succès, 233 tests cœur, 2 tests CLI et 22 tests
  Tauri passés ; un test de corpus local optionnel ignoré ;
- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck`, `npm test` (117 tests) et `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Compatibilité des formats de synchronisation (lot 11 de SYNC-011)

### Objectif

Garantir qu'une mise à niveau conserve la lecture des checkpoints existants et
qu'un format futur inconnu ne puisse modifier silencieusement l'état local.

### Actions et choix

- ajout de fixtures JSON déterministes pour le checkpoint contigu v1 et le
  checkpoint compact v2, avec identifiants, dates et événements stables ;
- remplacement du test v1 construit par le code courant par un véritable test
  d'import de fixture, et ajout du même contrat d'import pour la fixture v2 ;
- ajout d'un test de checkpoint futur correctement chiffré, mais non pris en
  charge, vérifiant la conservation du flux, du curseur et de l'empreinte déjà
  importés dans SQLite ;
- authentification et validation systématiques d'un checkpoint avant le
  raccourci d'idempotence fondé sur son empreinte ;
- documentation de l'indépendance entre disposition WebDAV, enveloppes,
  payloads et protocole d'événements, ainsi que de la matrice actuellement
  acceptée pour les segments, accusés, registres et checkpoints ;
- définition d'une future migration parallèle et non destructive : nouveau
  chemin versionné, lecture des deux formats, preuve de compatibilité de tous
  les appareils actifs, puis nettoyage borné de l'ancien checkpoint seulement
  lorsqu'un autre reste récupérable ;
- clôture de toutes les tâches du lot 11 dans `SYNC-011.md` et ajout de liens
  depuis les README bilingues.

### Vérifications

- tests ciblés des checkpoints v1, v2 et futur inconnu : succès ;
- `cargo test --workspace` : succès hors sandbox, 235 tests cœur, 2 tests CLI
  et 22 tests Tauri passés ; un test de corpus local optionnel ignoré ; la
  première exécution confinée avait seulement empêché les serveurs de boucle
  locale des tests HTTP/WebDAV d'ouvrir leurs ports ;
- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `npm run typecheck`, `npm test` (117 tests) et `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Diagnostic exportable dans l'application (lot 12 de SYNC-011)

### Objectif

Permettre à l'utilisateur d'enregistrer depuis Linux ou Android un diagnostic
de synchronisation utile au support sans exposer ses secrets, ses identifiants
ni ses contenus.

### Actions et choix

- exposition du diagnostic expurgé existant par une commande Tauri en lecture
  seule, avec sérialisation JSON camelCase ;
- ajout dans la boîte de dialogue de synchronisation d'une action
  d'enregistrement, de ses états d'attente, de succès et d'erreur ;
- intégration des sélecteurs de fichiers natifs et de l'écriture de texte Tauri,
  avec permissions distinctes pour desktop et mobile et prise en charge des URI
  de contenu Android ;
- renforcement du test d'expurgation avec des secrets, UUID et noms d'appareil,
  URL, titres, auteurs et contenus en cache sentinelles ;
- vérification des compteurs de checkpoints, de compaction et de nettoyage
  distant dans le JSON exporté ;
- ajout des contrats TypeScript d'enregistrement desktop, d'URI Android et
  d'annulation, puis documentation de l'usage dans les README bilingues ;
- clôture des tâches du lot 12 dans `SYNC-011.md`.

### Vérifications

- `cargo fmt --all -- --check`, `cargo check --workspace` et
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` : succès ;
- `cargo test --workspace` : succès hors sandbox, 235 tests cœur, 2 tests CLI
  et 23 tests Tauri passés ; un test de corpus local optionnel ignoré ; la
  première exécution confinée avait uniquement empêché neuf serveurs HTTP et
  WebDAV de test d'ouvrir leurs ports de boucle locale ;
- `npm run typecheck`, `npm test` (122 tests) et `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Documentation opérationnelle de la synchronisation (lot 13 de SYNC-011)

### Objectif

Fournir des procédures de sauvegarde, de récupération et de diagnostic qui
reflètent les garanties et limites réelles de la synchronisation actuelle.

### Actions et choix

- ajout de guides d'exploitation anglais et français reliés aux README ;
- distinction explicite entre SQLite, coffre natif et stockage WebDAV, avec le
  périmètre récupérable de chacun ;
- documentation de la sauvegarde et restauration SQLite, de la reconstruction
  depuis un checkpoint WebDAV et du repeuplement séparé des corps d'articles ;
- procédures de perte, révocation et réinstallation d'appareil, en distinguant
  l'UUID neuf d'une restauration cohérente qui conserve l'ancien UUID ;
- matrice des pertes de clé, mot de passe, stockage distant et copies locales ;
- avertissement sur la révocation logique, qui ne remplace pas une rotation de
  clé de groupe encore non implémentée ;
- inventaire des métadonnées visibles par l'hébergeur malgré le chiffrement et
  rappel de l'obligation HTTPS ;
- tableau des bornes courantes de segments, checkpoints, appareils, listes,
  compaction, nettoyage et rétention locale ;
- commandes de diagnostic expurgé et de contrôle SQLite en lecture seule ;
- clôture des huit tâches du lot 13 dans `SYNC-011.md`.

### Vérifications

- contrôle des valeurs documentées contre les constantes et chemins du code :
  succès ;
- contrôle de l'existence des cibles des liens relatifs : succès ;
- `git diff --check` : succès ;
- aucun test de code exécuté, l'intervention étant exclusivement documentaire.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à ce lot.

## 2026-08-30 — Commit des lots 10 à 13 de SYNC-011

### Objectif

Figer la reprise après interruption, la compatibilité des formats, le
diagnostic graphique expurgé et la documentation opérationnelle.

### Actions et vérifications

- sélection explicite des 26 sources, fixtures, dépendances, permissions, tests
  et documents appartenant à SYNC-011 ;
- exclusion de `app/env.txt`, `logo_raw.png`, `src/call_graph.png` et
  `src/uml.png` ;
- `git diff --cached --check` : succès ;
- commit créé : `49cc582` (`[AI] Complete sync recovery and diagnostics`).

Ces changements ont été créés avec l'assistance d'une IA.

## 2026-09-07 — Relief de l'indicateur de refresh mobile (issue Taiga 16)

### Objectif

Faire ressortir visuellement l'icône animée du pull-to-refresh mobile afin
qu'elle paraisse flotter au-dessus de la chronologie pendant l'actualisation.

### Actions et choix

- modification exclusive de l'état mobile `pull-refresh.refreshing` ;
- ajout de la couleur d'accent, d'une surface opaque légèrement dégradée et
  d'un liseré clair ;
- combinaison d'un reflet interne, d'une ombre courte de contact et d'une ombre
  diffuse pour produire un relief discret ;
- élévation du plan d'affichage et décalage vertical de deux pixels, sans
  modifier l'indicateur pendant le geste ni les contrôles desktop.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 122 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès ;
- aucun test visuel automatisé disponible ; validation finale à effectuer sur
  émulateur ou appareil mobile.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Favoris visibles pendant la sélection mobile (issue Taiga 17)

### Objectif

Conserver l'état favori visible dans la chronologie pendant une sélection
groupée, sans permettre une modification accidentelle avant archivage.

### Actions et choix

- conservation des étoiles pleines et vides dans chaque ligne lorsque le mode
  sélection est actif ;
- désactivation réelle du contrôle favori et remplacement de son libellé
  d'action par un état accessible « favori » ou « non favori » ;
- masquage limité aux actions individuelles de lecture et d'archivage ;
- réduction de l'espace réservé aux actions de ligne pour tenir compte de la
  seule étoile restante ;
- extension du test de sélection mobile pour couvrir les deux états visuels et
  garantir qu'un clic ne déclenche aucune écriture.

### Vérifications

- test Vitest ciblé du mode sélection et du favori en lecture seule : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 122 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Position de lecture conservée entre les articles (issue Taiga 18)

### Objectif

Restaurer la position verticale propre à chaque article après une navigation
vers l'article précédent ou suivant.

### Actions et choix

- mémorisation en session de la progression relative du lecteur pour chaque
  identifiant d'article avant de quitter celui-ci ;
- restauration différée jusqu'à la réception de la hauteur réelle du contenu
  embarqué, afin de ne pas appliquer la position sur une iframe encore vide ;
- conservation de la progression plutôt que d'une distance en pixels pour
  supporter les variations de hauteur et rester compatible avec le redimensionnement
  du texte ;
- ajout d'un test mobile couvrant le parcours premier article, article suivant,
  puis retour au premier article.

### Vérifications

- test Vitest ciblé de la restauration entre deux articles : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 123 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Stabilisation de la restauration des articles longs (issue Taiga 18)

### Objectif

Éviter que la position mémorisée soit restaurée sur une hauteur provisoire de
l'iframe, ce qui ramenait les articles longs presque au début.

### Actions et choix

- ajout d'un marqueur de hauteur prête dans le bridge de l'article, émis lors
  de l'événement `load` après construction du contenu ;
- maintien du redimensionnement sur les mesures intermédiaires, mais report de
  la restauration jusqu'à la première mesure prête ;
- adaptation du test de régression pour injecter d'abord une petite hauteur
  provisoire, puis la hauteur finale ;
- recalcul et mise à jour du hash CSP du bridge dans le document embarqué et la
  configuration Tauri.

### Vérifications

- test Vitest ciblé de la navigation avec hauteur provisoire : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 123 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Affichage compact des abonnements

### Objectif

Simplifier la page de gestion des abonnements en masquant par défaut les
informations détaillées de chaque flux.

### Actions et choix

- remplacement de la carte toujours développée par une ligne compacte contenant
  l'icône, le nom, un témoin de santé vert ou rouge et une flèche d'expansion ;
- détermination du témoin de santé à partir de la présence d'une dernière erreur,
  avec un libellé accessible sans exposer son détail ;
- ajout d'un panneau indépendant par flux, fermé par défaut et contenant les
  métadonnées, l'état actif ou inactif, l'erreur complète et les actions existantes ;
- conservation de l'état ouvert de chaque flux pendant les rerenders et
  restitution du focus au bouton actionné ;
- adaptation responsive de la ligne et du panneau détaillé pour mobile.

### Vérifications

- test Vitest ciblé des états compact, sain, en erreur et développé : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 123 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Masquage temporaire de l'accès à la synchronisation

### Objectif

Retirer provisoirement le bouton de synchronisation de la page de gestion des
abonnements sans supprimer la fonctionnalité sous-jacente.

### Actions et choix

- ajout de l'attribut natif `hidden` au bouton, ce qui le retire de l'affichage
  et de la navigation clavier ;
- conservation du bouton et de son branchement pour permettre une réactivation
  ultérieure sans réimplémentation ;
- ajout d'un test vérifiant explicitement son état masqué.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 124 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Commit des améliorations UX des issues et abonnements

### Objectif

Figer sur la branche `dev` les améliorations mobiles des issues 16 à 18 et la
simplification de la gestion des abonnements.

### Actions et vérifications

- sélection exclusive des quatre fichiers applicatifs concernés ;
- exclusion des fichiers personnels non suivis présents dans l'espace de travail ;
- `git diff --cached --check` : succès ;
- commit créé : `28b6732` (`[AI] Refine reader and feed management UX`).

Ces changements ont été créés avec l'assistance d'une IA.

## 2026-09-07 — Filtrage de la chronologie par flux

### Objectif

Permettre d'afficher les articles d'un seul abonnement, sur mobile comme sur
desktop, tout en préparant le futur système de catégories et d'étiquettes.

### Actions et choix

- remplacement du logo d'en-tête par un bouton burger suivi du contexte
  courant, « InkRiver » ou le nom du flux sélectionné ;
- ajout d'un panneau de filtrage listant « Tous les flux » puis chaque
  abonnement avec son icône et son nom ;
- fermeture du panneau et retour à la chronologie lors du choix d'un flux,
  avec prise en charge du fond, de la touche Échap et de la navigation Retour ;
- application du périmètre de flux avant les vues Tous, Favoris et Non lus,
  y compris pour leurs compteurs et les actions groupées ;
- ajout d'un état vide propre à un flux sans article et réinitialisation sûre
  du filtre lorsque l'abonnement concerné est supprimé ;
- ajout de styles adaptatifs pour le panneau latéral et les intitulés longs.

### Vérifications

- tests Vitest du nouvel en-tête, du menu mobile, du filtrage par flux, des
  compteurs et du retour à tous les flux : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 125 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Confinement de l'en-tête et des filtres sur mobile

### Objectif

Empêcher un nom de flux long d'élargir l'interface et de repousser hors écran
les actions d'en-tête et les vues Tous, Favoris et Non lus.

### Actions et choix

- allocation fixe de la largeur nécessaire aux boutons d'ajout et de paramètres ;
- contraction forcée du bloc de marque et du titre dans l'espace restant, avec
  troncature par ellipse du nom de flux ;
- prise en compte des marges de sécurité latérales des appareils mobiles ;
- remplacement des colonnes implicites des trois filtres par des colonnes
  `minmax(0, 1fr)` et réduction de leur padding mobile ;
- confinement de la barre supérieure et de la barre des vues à la largeur de
  l'écran.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 125 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Catégories locales de flux et filtrage associé

### Objectif

Attribuer une catégorie facultative à chaque flux et permettre de filtrer la
chronologie par catégorie depuis le menu principal.

### Actions et choix

- ajout d'une migration créant des catégories à identifiant UUID stable et une
  référence nullable depuis les abonnements existants ;
- ajout des opérations de stockage, DTO et commandes Tauri pour lister les
  catégories et affecter, créer implicitement ou retirer celle d'un flux ;
- réutilisation insensible à la casse des catégories existantes et validation
  des noms à 80 caractères maximum ;
- ajout d'un champ avec suggestions dans le détail développé de chaque flux ;
- ajout d'une section Catégories sous la liste des flux dans le menu burger ;
- extension du périmètre de filtrage, de son titre, de ses compteurs et de ses
  états vides aux catégories ;
- conservation volontairement locale de ces nouvelles données jusqu'à la
  reprise du chantier de synchronisation.

### Vérifications

- `npm run typecheck` : succès ;
- `npm test` : succès, 126 tests passés ;
- `npm run build` : succès ;
- test Rust ciblé du stockage des catégories : succès ;
- test Rust ciblé de l'adaptateur Tauri : succès ;
- `cargo test` à la racine : succès, 236 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ; la première exécution confinée avait refusé les
  sockets de test locales, puis la relance autorisée a réussi ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo fmt --all --check` et `cargo clippy --all-targets -- -D warnings` dans
  les deux crates : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Catégorie facultative à la création d'un flux

### Objectif

Permettre d'affecter immédiatement une catégorie nouvelle ou existante lors de
l'ajout d'un abonnement.

### Actions et choix

- ajout d'un champ facultatif avec les catégories existantes en suggestions
  dans le formulaire de nouvelle source ;
- transmission du nom choisi dans la commande Tauri `add_feed` ;
- création ou réutilisation de la catégorie et retour du nouveau flux déjà
  catégorisé avant la fermeture de la boîte de dialogue ;
- rechargement conjoint des flux et catégories avant l'actualisation initiale ;
- adaptation du formulaire desktop et mobile à cette nouvelle ligne ;
- ajout de tests frontend et Tauri couvrant l'enregistrement implicite.

### Vérifications

- test Vitest ciblé de l'ajout avec et sans catégorie : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 127 tests passés ;
- `npm run build` : succès ;
- test Tauri ciblé de l'ajout catégorisé : succès ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans `app/src-tauri` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Respect de la barre de navigation Android

### Objectif

Empêcher la barre de navigation Android à trois boutons de recouvrir le bas de
l'application et de rendre inaccessibles les dernières entrées du menu latéral.

### Actions et choix

- ajout d'un gestionnaire natif des insets de la barre de navigation sur la
  WebView, afin que son contenu conserve le mode bord-à-bord sans passer sous
  les boutons système, y compris en paysage ;
- conservation ciblée de `MainActivity.kt` dans Git tandis que les autres
  fichiers Android générés restent ignorés ;
- activation de `viewport-fit=cover` et ajout d'une marge de défilement basse au
  menu des flux pour les plateformes qui exposent leurs zones sûres en CSS.

### Vérifications

- `env ANDROID_HOME=/home/r0m1/Android/Sdk ./gradlew :app:compileUniversalDebugKotlin` : succès ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 127 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-07 — Cartes statiques YouTube et X dans les articles

### Objectif

Remplacer les espaces laissés par les médias externes supprimés lors du
nettoyage HTML par des aperçus sûrs, lisibles et ouvrables dans l'application ou
le navigateur approprié.

### Actions et choix

- ajout d'un prétraitement commun aux contenus RSS et aux pages extraites qui
  reconnaît les principales variantes d'URL YouTube et les embeds X/Twitter ;
- conversion des vidéos YouTube en cartes statiques avec miniature distante et
  lien canonique, sans lecteur ni script tiers ;
- conservation des blockquotes X contenant déjà le texte du tweet et conversion
  des iframes X en cartes-liens de secours ;
- enrichissement visuel dans le document de lecture, compatible mobile et
  thèmes clair/sombre, tout en réutilisant l'ouverture externe sécurisée ;
- exclusion des miniatures vidéo de la visionneuse d'images afin qu'un clic
  ouvre directement la publication ;
- maintien de la suppression de toute iframe inconnue et vérification des noms
  de domaine pour refuser les hôtes ressemblants malveillants.

### Vérifications

- tests Rust ciblés du nettoyage, des formes d'URL, du flux RSS et de
  l'extraction de page : succès ;
- `cargo test` à la racine : succès, 242 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ; la première exécution confinée avait refusé les
  sockets locales, puis la relance autorisée a réussi ;
- `npm run typecheck` : succès ;
- `npm test` : succès, 129 tests passés ;
- `npm run build` : succès ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `cargo fmt --all --check` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-08 — Enrichissement statique des cartes X

### Objectif

Afficher le véritable contenu des tweets lorsque l'article source ne fournit
qu'une iframe avec un identifiant, tout en conservant une lecture hors ligne.

### Actions et choix

- validation du fonctionnement actuel de l'endpoint public oEmbed de X sans clé
  API et de la forme canonique d'URL ne nécessitant pas le nom de l'auteur ;
- récupération bornée et concurrente du HTML oEmbed uniquement pour les cartes
  de secours, avec demande explicite de suppression du script et du suivi ;
- nettoyage systématique de la réponse distante avant stockage, sans exécuter
  le widget JavaScript de X ;
- dégradation silencieuse vers le lien existant pour les tweets privés,
  supprimés ou indisponibles, sans transformer l'échec en erreur de flux ;
- enrichissement des nouveaux contenus RSS, des pages complètes extraites et
  des cartes déjà présentes en base lors d'une actualisation ;
- écriture conditionnelle en base pour ne pas écraser un rafraîchissement
  concurrent et conservation du texte enrichi pour les lectures hors ligne.

### Vérifications

- requête réelle vers `publish.x.com/oembed` : succès avec texte, auteur et date,
  y compris depuis une URL `x.com/i/status/...` ;
- tests ciblés de résolution, déduplication, repli hors ligne et mise à jour
  conditionnelle du cache : succès ;
- `cargo test` à la racine : succès, 245 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `cargo fmt --all --check` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-08 — Réparation des cartes X vides

### Objectif

Corriger les publications X dont l'article conserve un lien de statut dans une
`blockquote`, mais aucun texte visible, ce qui produisait une carte presque vide
sur Linux comme sur Android.

### Actions et choix

- normalisation des `blockquote` X/Twitter vides en cartes de secours
  résolubles par l'enrichissement oEmbed existant ;
- conservation inchangée des citations qui contiennent déjà le texte de la
  publication ;
- élargissement de la maintenance au rafraîchissement afin de sélectionner et
  réparer également les cartes vides déjà enregistrées dans la base locale ;
- maintien du repli vers un lien statique lorsqu'une publication est privée,
  supprimée, inaccessible ou que l'appareil est hors ligne.

### Vérifications

- tests unitaires ajoutés pour une citation X vide et pour la conservation
  d'une citation déjà renseignée : succès ;
- test de stockage ajouté pour la sélection et le remplacement conditionnel
  d'une carte X vide existante : succès ;
- `cargo test` à la racine : succès, 247 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `npm test -- --run` : succès, 129 tests passés ;
- `npm run build` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-08 — Correction du diagnostic de texte des cartes X

### Objectif

Corriger la carte X toujours vide observée sur Linux et Android malgré un nouvel
import du flux.

### Actions et choix

- reproduction à partir de l'article et du HTML exacts retrouvés dans une copie
  de la base Linux ;
- identification d'un faux positif de `html2text`, qui interprétait le saut de
  ligne d'un lien vide comme du contenu et empêchait tout appel oEmbed ;
- remplacement par une détection directe des vrais nœuds textuels, en ignorant
  les balises, les espaces et les variantes d'espace insécable HTML ;
- ajout du fragment exact, avec saut de ligne et attribut `rel`, au test de
  non-régression.

### Vérifications

- endpoint public oEmbed : le tweet exact renvoie bien son texte ;
- essai de bout en bout avec le HTML exact : contenu modifié et texte du tweet
  présent ;
- tests ciblés `article_html` : succès, 8 tests passés ;
- `cargo test` à la racine : succès, 247 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `cargo fmt --all --check` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-08 — Prise en charge des espaces Unicode invisibles dans les cartes X

### Objectif

Réparer définitivement les cartes X vides de l'article « The Epstein Files,
9/11, and Other Important Matters Obscured by the Iran War » du flux « The Unz
Review ».

### Actions et choix

- inspection du contenu exact dans une copie de la base Linux après
  rafraîchissement ;
- identification d'un espace de largeur nulle `U+200B` dans le lien de la
  publication, qui faisait encore considérer la citation comme renseignée ;
- exclusion des espaces invisibles et caractères de formatage Unicode usuels,
  sous leur forme littérale ou sous forme d'entité HTML, lors de la détection
  du contenu visible ;
- remplacement du cas de test approximatif par le fragment contenant le
  véritable caractère `U+200B`.

### Vérifications

- maintenance X exécutée sur une copie fraîche de la base : succès, la citation
  vide est remplacée par le texte, l'auteur et la date du tweet exact ;
- tests ciblés `article_html` : succès, 8 tests passés ;
- `cargo test` à la racine : succès, 247 tests unitaires et 2 tests CLI passés,
  1 corpus local ignoré ;
- `cargo test` dans `app/src-tauri` : succès, 23 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `cargo fmt --all --check` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Respect des barres système Android

### Objectif

Empêcher l'en-tête, le bas des listes et les commandes fixes d'InkRiver d'être
recouverts par la barre d'état ou la barre de navigation Android.

### Actions et choix

- conservation du mode edge-to-edge compatible avec les versions Android
  récentes ;
- remplacement du padding limité à la barre de navigation par des marges
  natives qui réduisent réellement la surface du WebView ;
- prise en compte conjointe des barres système, de leurs variantes latérales et
  des découpes d'écran ;
- mise à jour des marges seulement lorsque les insets changent afin d'éviter
  des recalculs de mise en page inutiles.

### Vérifications

- `npm test -- --run` : succès, 129 tests passés ;
- `npm run build` : succès ;
- `npm run tauri android build -- --debug` : succès après relance hors
  confinement pour autoriser le WebSocket local de Tauri ; APK universel et AAB
  debug générés ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Sections accordéon dans le menu de filtrage

### Objectif

Permettre de réduire ou développer séparément les listes « Flux » et
« Catégories » dans le menu latéral.

### Actions et choix

- remplacement des simples intertitres par deux boutons d'accordéon sur toute
  la largeur, accompagnés d'une flèche indiquant leur état ;
- intégration de « Tous les flux » à la section « Flux » ;
- ouverture des deux sections par défaut et conservation de leur état lors de
  la fermeture puis de la réouverture du menu ;
- ajout des attributs `aria-expanded` et `aria-controls`, ainsi que de zones
  réellement masquées avec `hidden` ;
- animation courte de la flèche sans modifier le comportement de sélection des
  filtres.

### Vérifications

- test de réduction indépendante et de conservation de l'état : succès ;
- `npm test -- --run` : succès, 130 tests passés ;
- `npm run build` : succès ;
- `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Accès permanent à « Tous les flux »

### Objectif

Garder le filtre global disponible même lorsque la section « Flux » est
réduite.

### Actions et choix

- déplacement de « Tous les flux » au-dessus des deux accordéons ;
- restauration de sa séparation visuelle avec les sections repliables ;
- ajout d'une assertion vérifiant qu'il ne se trouve jamais dans une zone
  masquée lorsque les deux accordéons sont fermés.

### Vérifications

- `npm test -- --run` : succès, 130 tests passés ;
- `npm run build` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Étiquettes locales des articles

### Objectif

Permettre d'associer plusieurs étiquettes libres aux articles, de les retirer
depuis le lecteur et de filtrer la chronologie par étiquette, sans étendre pour
le moment le protocole de synchronisation.

### Actions et choix

- ajout des tables SQLite `labels` et `article_labels`, avec relation
  plusieurs-à-plusieurs, unicité du nom normalisé et suppressions en cascade ;
- normalisation Unicode, réduction des espaces et réutilisation implicite
  d'une étiquette existante sans tenir compte de la casse ;
- ajout d'opérations transactionnelles acceptant plusieurs articles afin de
  préparer une future édition par lot, avec rejet atomique des sélections
  invalides ;
- chargement des étiquettes avec les résumés et le détail des articles, tout en
  les préservant lors des mises à jour issues des flux ;
- exposition des opérations locales par les commandes Tauri et l'API
  TypeScript, sans produire d'événement dans le journal de synchronisation ;
- ajout dans le lecteur d'un éditeur compact avec suggestions, création
  implicite et retrait par pastille ;
- ajout d'une troisième section accordéon « Étiquettes » dans le menu et d'un
  filtre exclusif compatible avec les vues Tous, Favoris et Non lus ;
- adaptation responsive et accessible des contrôles et des libellés.

### Vérifications

- `cargo test` à la racine : succès hors confinement, 249 tests unitaires et
  2 tests CLI passés, 1 corpus local ignoré ;
- `cargo test` dans `app/src-tauri` : succès, 24 tests passés ;
- `cargo clippy --all-targets -- -D warnings` dans les deux crates : succès ;
- `npm test -- --run` : succès, 132 tests passés ;
- `npm run build` : succès ;
- `cargo fmt --all` exécuté.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Bouton compact d'ajout d'étiquette

### Objectif

Alléger l'éditeur d'étiquettes en remplaçant le bouton textuel « Ajouter » par
un bouton « + ».

### Actions et choix

- remplacement du texte par un symbole plus dans un bouton carré compact ;
- conservation d'un titre et d'un libellé ARIA explicites pour l'accessibilité ;
- conservation de l'état occupé et de la désactivation pendant l'enregistrement.

### Vérifications

- test frontend complété pour contrôler le symbole et son libellé accessible ;
- `npm test -- --run` et `npm run build` exécutés avec succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Étiquettes accessibles en fin d'article

### Objectif

Permettre d'ajouter ou retirer des étiquettes immédiatement après la lecture,
sans devoir remonter au début de l'article.

### Actions et choix

- affichage du même éditeur d'étiquettes avant et après le contenu de l'article ;
- placement de l'éditeur inférieur avant les actions de pied de page ;
- génération d'identifiants distincts pour les champs et listes de suggestions
  afin de conserver un HTML valide ;
- branchement des événements sur les deux formulaires et adaptation responsive
  du bloc inférieur.

### Vérifications

- test d'ajout depuis le widget inférieur, de duplication visuelle des
  étiquettes et de retrait depuis ce même widget : succès ;
- `npm test -- --run` : succès, 132 tests passés ;
- `npm run build` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Suggestions d'étiquettes adaptées au mobile

### Objectif

Empêcher la liste de suggestions de se désolidariser du champ d'étiquette lors
du défilement ou de l'ouverture du clavier Android.

### Actions et choix

- remplacement du `datalist` natif, rendu hors du document par Android WebView,
  par une liste HTML intégrée au formulaire ;
- filtrage dynamique des étiquettes existantes pendant la saisie et sélection
  d'une suggestion sans soumettre prématurément le formulaire ;
- limitation de la hauteur de la liste relativement à la fenêtre et activation
  de son propre défilement ;
- ouverture sous le champ dans le widget supérieur et au-dessus du champ dans
  le widget inférieur afin de préserver l'espace disponible face au clavier ;
- ajout des rôles et attributs ARIA de liste et de champ combiné.

### Vérifications

- test du focus, du filtrage, de la sélection et de l'absence de `datalist`
  natif : succès ;
- `npm test -- --run` : succès, 133 tests passés ;
- `npm run build` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-11 — Maintien du champ d'étiquette au-dessus du clavier

### Objectif

Conserver le champ d'étiquette visible lorsque le clavier virtuel réduit la
zone d'affichage sur mobile.

### Actions et choix

- suivi de la fenêtre visuelle du WebView pendant que le champ possède le focus ;
- ajustement minimal du défilement du lecteur lors des événements de
  redimensionnement ou déplacement de la fenêtre visuelle ;
- réservation d'une marge inférieure tenant compte de la barre d'actions du
  lecteur afin de placer le champ au-dessus du clavier et des contrôles fixes ;
- nettoyage systématique des écouteurs à la perte du focus et avant chaque
  nouveau rendu pour éviter leur accumulation.

### Vérifications

- test mobile simulant le focus puis le déplacement de la limite visible :
  succès ;
- `npm test -- --run` : succès, 134 tests passés ;
- `npm run build` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-12 — Synchronisation des catégories de flux

### Objectif

Répliquer l'affectation et le retrait de la catégorie d'un flux entre appareils,
sans exposer les identifiants SQLite locaux ni changer la version du protocole
encore inutilisé en production.

### Actions et choix

- ajout de l'événement typé `subscription_category_set`, portant l'identifiant
  logique du flux et un nom de catégorie optionnel ;
- journalisation transactionnelle des changements locaux et inclusion des
  catégories déjà présentes dans le bootstrap d'activation ;
- projection distante avec attente des dépendances, réutilisation insensible à
  la casse des catégories locales et création d'un UUID propre à l'appareil si
  nécessaire ;
- résolution des changements concurrents par un registre LWW indépendant et
  prise en charge explicite du retrait de catégorie ;
- absence de journalisation retour lors d'un import, validation des noms reçus
  et documentation du nouveau périmètre de synchronisation ;
- tests du format sérialisé, du rollback atomique, du bootstrap, de toutes les
  permutations d'événements et de la convergence après retrait.

### Vérifications

- `cargo test` : succès, 251 tests unitaires et 2 tests d'intégration passés ;
  1 test de corpus local ignoré comme prévu ;
- `cargo test` dans `app/src-tauri` : succès, 24 tests passés ;
- `cargo clippy --all-targets -- -D warnings` : succès ;
- `cargo clippy --manifest-path app/src-tauri/Cargo.toml --all-targets -- -D warnings` : succès ;
- `cargo fmt --all` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.

## 2026-09-12 — Synchronisation des étiquettes d'articles

### Objectif

Répliquer les ajouts et retraits d'étiquettes d'articles entre appareils tout
en conservant des identifiants SQLite propres à chaque installation.

### Actions et choix

- ajout de l'événement typé `article_label_set`, contenant la référence logique
  de l'article, le nom normalisé de l'étiquette et son état d'appartenance ;
- création d'un registre LWW indépendant par article et nom d'étiquette en
  minuscules, afin que des étiquettes différentes ne se remplacent pas ;
- journalisation transactionnelle des opérations groupées, bootstrap des
  associations existantes et rollback conjoint des données et du journal ;
- projection des événements distants avec attente des dépendances, création ou
  réutilisation locale de l'étiquette et absence d'événement retour ;
- validation NFKC, des espaces et de la longueur des noms reçus ;
- déclenchement de la synchronisation automatique après ajout ou retrait depuis
  l'interface, puis mise à jour des documentations française et anglaise.

### Vérifications

- `cargo test` : succès, 253 tests unitaires et 2 tests d'intégration passés ;
  1 test de corpus local ignoré comme prévu ;
- `cargo test` dans `app/src-tauri` : succès, 24 tests passés ;
- Clippy sans avertissement sur le cœur et l'adaptateur Tauri ;
- `npm test -- --run` : succès, 135 tests passés ;
- `npm run build`, `cargo fmt --all` et `git diff --check` : succès.

Ces changements ont été créés avec l'assistance d'une IA. Aucun commit associé
à cette intervention.
