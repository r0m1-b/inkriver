# Reader — demandes de fonctionnalités

Ce document rassemble les améliorations produit à préciser et à traiter avant
de poursuivre l'adaptation Android. Leur présence ici ne signifie pas encore
qu'une solution technique ou une politique de migration a été choisie.

États utilisés : `proposée`, `planifiée`, `en cours`, `terminée`.

## FR-001 — Supprimer un abonnement RSS

**État :** terminée
**Priorité :** avant Android

### Besoin

Pouvoir supprimer un flux depuis la gestion des abonnements, au lieu de pouvoir
seulement le désactiver.

### Comportement attendu

- proposer une action « Supprimer » distincte de « Désactiver » ;
- demander une confirmation explicite indiquant que les articles et leurs
  états locaux seront également supprimés ;
- retirer immédiatement le flux de la liste des abonnements ;
- retirer immédiatement ses articles de la chronologie et fermer le panneau de
  lecture si l'article affiché appartenait à ce flux ;
- exécuter la suppression du flux et de ses articles dans une seule transaction
  afin de ne laisser aucun état partiellement supprimé ;
- afficher une erreur compréhensible si la suppression échoue.

### Décision retenue

La suppression d'un flux efface définitivement tous ses articles ainsi que les
favoris et états de lecture associés. L'implémentation supprime explicitement
les articles avant le flux dans une même transaction SQLite ; la contrainte
`ON DELETE RESTRICT` reste ainsi une protection pour les autres opérations.

## FR-002 — Marquer explicitement un article comme lu ou non lu

**État :** proposée
**Priorité :** avant Android

### Besoin

Pouvoir corriger l'état de lecture d'un article, en particulier le remettre à
« Non lu » après l'avoir ouvert.

### Comportement attendu

- proposer l'action depuis le panneau de lecture ;
- afficher clairement l'état courant et l'action inverse disponible ;
- mettre à jour immédiatement la chronologie et le panneau de lecture ;
- persister l'état après fermeture et relance de l'application ;
- conserver le marquage automatique « Lu » lors de la première ouverture.

Le marquage en masse de plusieurs articles n'est pas inclus dans cette demande.

## FR-003 — Ouvrir les liens dans le navigateur par défaut

**État :** proposée
**Priorité :** avant Android

### Besoin

Rendre les liens présents dans les articles utilisables, notamment pour ouvrir
sur le site d'origine les contenus complets réservés aux abonnés Medium ou
Substack.

### Comportement attendu

- ouvrir les liens HTTP(S) dans le navigateur système avec le plugin Tauri
  `opener` ;
- empêcher la navigation du contenu principal à l'intérieur de la WebView ;
- ignorer ou refuser explicitement les protocoles non autorisés ;
- conserver le bouton « Lire l'original » comme accès direct à l'URL de
  l'article lorsqu'elle existe ;
- afficher une erreur si le navigateur ne peut pas être lancé.

## FR-004 — Afficher le détail des erreurs de rafraîchissement

**État :** proposée
**Priorité :** avant Android

### Besoin

Remplacer le seul compteur « X flux en erreur » par des informations permettant
de comprendre et de corriger chaque échec.

### Comportement attendu

- conserver le résumé global du rafraîchissement ;
- afficher, pour chaque flux en échec, sa plateforme ou son URL, l'étape en
  erreur et le message détaillé reçu du cœur Rust ;
- distinguer au minimum les erreurs HTTP, de lecture de réponse, d'analyse du
  flux et de métadonnées ;
- permettre de fermer ou de replier le détail sans perdre les articles chargés
  avec succès ;
- ne pas masquer les articles déjà présents dans le cache.

## FR-005 — Accepter une URL de profil Medium

**État :** proposée
**Priorité :** avant Android

### Besoin

Éviter qu'une URL de profil Medium valide mais non RSS soit enregistrée sans
explication, comme `https://medium.com/@utilisateur`.

### Comportement attendu

- reconnaître les URL de profils et publications Medium courantes ;
- proposer ou appliquer leur conversion vers l'URL RSS correspondante, par
  exemple `https://medium.com/feed/@utilisateur` ;
- montrer à l'utilisateur l'URL réellement enregistrée ;
- conserver une erreur détaillée lorsque l'URL obtenue ne fournit pas un flux
  RSS ou Atom valide.

## FR-006 — Consulter les articles favoris

**État :** proposée
**Priorité :** basse

### Besoin

Disposer d'un espace permettant de retrouver uniquement les articles marqués
comme favoris.

### Comportement attendu

- afficher les favoris du plus récent au plus ancien, avec les mêmes
  informations que la chronologie principale ;
- permettre d'ouvrir et de lire un favori avec le panneau de lecture existant ;
- répercuter immédiatement l'ajout ou le retrait d'un favori dans cette liste ;
- afficher un état vide explicite lorsqu'aucun article n'est favori ;
- conserver un fonctionnement entièrement hors ligne à partir de SQLite.

### Décision à prendre

Choisir ultérieurement entre un onglet, une page dédiée ou un filtre de la
chronologie. Cette décision devra tenir compte de la future navigation Android,
mais ne bloque pas les demandes prioritaires du premier test utilisateur.
