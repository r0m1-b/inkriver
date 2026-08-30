# Exploitation de la synchronisation InkRiver

Ce guide couvre sauvegarde, récupération, perte d'appareil, diagnostic,
confidentialité et limites actuellement implémentées. Il complète le
[contrat du protocole](synchronization.md). Les commandes doivent viser
l'installation concernée : la base du CLI et celle de l'application graphique
sont distinctes.

## Ce que chaque stockage permet de récupérer

| Stockage | Contenu | Absent |
| --- | --- | --- |
| `inkriver.db` | abonnements, cache d'articles et états locaux, journal et projections de synchronisation, UUID de l'appareil, configuration WebDAV non secrète | clé de groupe et mot de passe WebDAV |
| Coffre natif | clé de groupe de 32 octets et mot de passe WebDAV | articles, journal et registre des appareils |
| Répertoire WebDAV | événements, accusés, registres et checkpoints de récupération chiffrés | corps d'articles, logos, préférences d'interface et données métier en clair |

Seule une sauvegarde SQLite contient intégralement les corps d'articles en
cache. WebDAV peut reconstruire les abonnements et états utilisateur
synchronisés si la clé de groupe existe encore, mais les corps doivent être
retéléchargés depuis les flux. Restaurer uniquement SQLite préserve l'UUID de
l'appareil ; conserver ou restaurer l'entrée correspondante du coffre natif est
aussi nécessaire pour reprendre la configuration de synchronisation existante.

## Sauvegarder et restaurer SQLite

Fermer InkRiver avant de manipuler ses fichiers. Sous Linux, la base graphique
se trouve généralement ici :

```text
~/.local/share/io.github.r0m1-b.inkriver/inkriver.db
```

Le CLI de développement utilise `inkriver.db` à la racine du dépôt, sauf avec
`--database`. Android conserve sa base dans le stockage privé de l'application :
utiliser une sauvegarde de plateforme qui préserve l'ensemble des données de
l'application et son coffre natif seulement si le constructeur garantit
explicitement les deux. Ne pas supposer qu'une sauvegarde Android préserve les
clés Keystore ; la récupération WebDAV est la procédure portable. InkRiver ne
propose pas encore d'export SQLite brut sous Android.

Créer une sauvegarde Linux cohérente avec l'API de sauvegarde SQLite :

```bash
sqlite3 "$HOME/.local/share/io.github.r0m1-b.inkriver/inkriver.db" \
  ".backup '$HOME/inkriver-backup.db'"
sqlite3 "$HOME/inkriver-backup.db" "PRAGMA quick_check;"
```

`quick_check` doit afficher `ok`. Une copie ordinaire n'est sûre que si InkRiver
et tous les clients SQLite sont arrêtés. Ne jamais copier seulement le fichier
principal lorsqu'un fichier `-wal` peut encore contenir des transactions
validées.

Pour restaurer sous Linux :

1. fermer InkRiver et conserver séparément la base actuelle ;
2. valider la sauvegarde avec `PRAGMA quick_check` ;
3. remplacer `inkriver.db` pendant qu'aucun processus ne l'utilise ;
4. retirer les anciens `inkriver.db-wal` et `inkriver.db-shm` liés à la base
   remplacée ;
5. démarrer la même version d'InkRiver ou une version plus récente afin
   d'appliquer les migrations intégrées ;
6. vérifier abonnements et état de synchronisation avant toute modification.

Ne pas ouvrir avec une ancienne application une base déjà migrée par une
version plus récente. La base contient l'UUID : cloner la même sauvegarde sur
deux installations actives ferait écrire les deux sous la même identité de
journal et est interdit. Si l'UUID restauré a été révoqué, il reste révoqué ;
l'installation doit rejoindre le groupe à neuf avec un nouvel UUID.

## Reconstruire une installation depuis WebDAV

Utiliser cette procédure si SQLite est perdu mais qu'un appareil appairé de
confiance et le répertoire distant existent encore :

1. installer InkRiver afin de créer une base et un UUID neufs ;
2. sur un appareil survivant, ouvrir **Abonnements → Synchronisation** et créer
   une invitation ;
3. rejoindre le groupe avec le QR code ou l'invitation, puis saisir séparément
   le mot de passe WebDAV ;
4. lancer **Synchroniser maintenant**, plusieurs fois si l'interface signale
   encore du travail en attente ;
5. vérifier abonnements et états lu/favori/archivé ;
6. actualiser les flux pour repeupler les corps d'articles, jamais stockés sur
   WebDAV.

L'importeur authentifie un checkpoint de récupération, l'applique, puis traite
les segments immuables plus récents. Ne pas copier, renommer ni modifier
manuellement l'arborescence WebDAV. Une invitation conservée peut fournir la
clé de groupe, mais doit être protégée comme un mot de passe ; le mot de passe
WebDAV reste nécessaire.

## Appareil perdu, volé ou réinstallé

Depuis un appareil survivant fiable, sélectionner l'appareil absent dans la
boîte **Synchronisation**, choisir **Révoquer**, puis réussir une
synchronisation. Synchroniser ensuite les autres appareils actifs pour qu'ils
reçoivent le registre monotone. La révocation est définitive pour cet UUID : un
ancien registre ne peut pas le réactiver et ses futurs segments sont ignorés.

Il s'agit d'une révocation logique, pas d'une exclusion cryptographique. Un
appareil volé peut encore posséder la clé, des données déjà téléchargées et les
identifiants WebDAV. Changer le mot de passe chez l'hébergeur s'il est compromis,
puis reconfigurer les appareils fiables. La rotation de clé de groupe n'est pas
implémentée : un appareil ayant obtenu la clé ne peut pas encore être exclu des
chiffrements auxquels il a toujours accès.

Désinstaller l'application ou effacer ses données sans restauration complète
crée un nouvel UUID aléatoire. L'appairer comme nouvel appareil et ne jamais
tenter de réutiliser l'ancien UUID ; révoquer l'ancienne entrée si elle ne
reviendra pas. Restaurer une sauvegarde SQLite cohérente est différent : cela
conserve volontairement l'UUID sauvegardé.

## Scénarios de perte

| Situation | Récupération |
| --- | --- |
| Clé perdue sur un appareil | Réappairer depuis un appareil fiable qui possède encore la clé, après suppression de la configuration locale défectueuse si nécessaire. |
| Aucun appareil ni invitation protégée ne conserve la clé | Les chiffrements distants sont irrécupérables. Une base SQLite survivante peut initialiser un nouveau groupe ; sans SQLite non plus, l'état synchronisé est perdu. |
| Répertoire WebDAV perdu, SQLite survit | Conserver la base la plus complète, créer un groupe/répertoire WebDAV dédié neuf, réappairer les autres appareils et terminer la synchronisation avant de considérer cette copie comme récupérable. |
| WebDAV et toutes les bases SQLite perdus | InkRiver ne peut récupérer ni abonnements, ni états, ni cache. Les flux peuvent être ajoutés de nouveau, mais l'ancien historique lu/favori/archivé est perdu. |
| Mot de passe WebDAV perdu | Le réinitialiser chez l'hébergeur. Le mot de passe ne déchiffre pas les données : c'est la clé de groupe qui le fait. Les appareils fiables doivent ensuite recevoir le nouveau mot de passe. |

Avant de supprimer une configuration endommagée ou un répertoire distant,
préserver la base SQLite restante et exporter un diagnostic expurgé.

## Métadonnées visibles par l'hébergeur WebDAV

Les données métier sont chiffrées et authentifiées avec XChaCha20-Poly1305,
mais le chiffrement ne masque pas toutes les métadonnées de stockage et de
trafic. L'hébergeur peut observer :

- le compte, les adresses IP clientes, les horaires et volumes transférés ;
- l'empreinte stable de la clé de groupe utilisée comme nom de répertoire ;
- les UUID d'appareils dans les chemins des segments, checkpoints, accusés et
  registres ;
- les plages de séquences, versions d'enveloppe/protocole, tailles de
  chiffrements et donc des indications d'activité ou de compaction ;
- l'empreinte authentifiée de l'état d'un checkpoint, qui permet de corréler un
  checkpoint inchangé ou remplacé sans en lire l'état ;
- le nombre de documents d'appareils, checkpoints et segments conservés ;
- les téléversements temporaires puis les opérations atomiques `MOVE` et les
  suppressions.

Sans la clé, l'hébergeur ne peut lire les URL d'abonnement, métadonnées
d'articles, changements d'état, noms affichés d'appareils ni contenu des
registres. Les corps d'articles et identifiants WebDAV ne sont jamais inclus
dans les documents de synchronisation. HTTPS reste obligatoire : HTTP expose
l'authentification Basic et le trafic aux observateurs réseau, même si les
payloads InkRiver restent chiffrés.

## Limites et rétention implémentées

| Domaine | Borne actuelle |
| --- | --- |
| Segment immuable | au plus 250 événements et 2 Mio |
| Un cycle de synchronisation | au plus 20 segments téléchargés, quatre simultanés ; au plus huit checkpoints téléchargés |
| Traitement/compaction | au plus 1 000 événements importés/lus et 1 000 événements locaux compactés par passe bornée |
| Nettoyage distant | au plus 20 segments sûrs de l'appareil local par cycle |
| Checkpoint | au plus 10 000 événements retenus, 5 Mio d'état en clair et 8 Mio chiffrés |
| Appareils/plan de contrôle | au plus 256 membres, registres découverts, accusés, sources acquittées et frontières de checkpoint |
| Documents de contrôle | registres et accusés limités chacun à 256 Kio |
| Liste WebDAV | au plus 1 000 entrées et 1 Mio par `PROPFIND` ; connexion 10 s et requête 20 s |
| Rétention locale des articles | après 30 jours, seuls les articles datés, lus et non favoris sont archivés localement et libèrent leur corps en cache |

Si un checkpoint dépasse ses bornes, la synchronisation continue sans le
publier, mais la compaction et la suppression distante qui en dépendent restent
bloquées. Le travail borné progresse sur plusieurs cycles. Il n'existe pas de
rétention temporelle fixe du journal chiffré : les preuves sûres de checkpoint,
registre et accusés gouvernent le nettoyage.

## Diagnostic et vérifications

Dans l'application, utiliser **Abonnements → Synchronisation → Enregistrer le
diagnostic**. Le JSON exclut identifiants, URL, UUID, noms d'appareils et contenu
d'article. Le relire avant transmission.

Pour le CLI de développement, désigner explicitement la bonne base si besoin :

```bash
cargo run -- --database /chemin/vers/inkriver.db sync-diagnostic \
  > inkriver-sync-diagnostic.json
sqlite3 /chemin/vers/inkriver.db "PRAGMA quick_check;"
sqlite3 /chemin/vers/inkriver.db "PRAGMA foreign_key_check;"
```

Une base valide affiche `ok` pour `quick_check` et aucune ligne pour
`foreign_key_check`. Ces contrôles sont en lecture seule. Noter aussi la version
de l'application, la plateforme, l'heure du dernier succès et l'étape exacte de
l'erreur. Ne jamais joindre à un ticket la base brute, le coffre natif,
l'invitation d'appairage ou une liste WebDAV non expurgée.
