# SpotiLite

Client Spotify **Premium** natif pour Windows, pensé pour consommer le moins possible :
un seul exécutable, **objectif 50 à 100 Mo de RAM**, et une consommation de données maîtrisée.

![Aperçu de SpotiLite](docs/apercu.png)

SpotiLite n'est pas une version web de Spotify : l'interface est dessinée directement par
l'application (Rust + [egui](https://github.com/emilk/egui), rendu OpenGL), sans navigateur
embarqué ni Electron. Le son est décodé localement grâce à
[librespot](https://github.com/librespot-org/librespot), l'implémentation libre du protocole
de lecture Spotify.

## Points forts

| | |
|---|---|
| **Léger** | Un seul `.exe` (~12 Mo) sans DLL à installer. Pas de moteur web : l'interface ne se redessine que lorsqu'il se passe quelque chose (au plus une fois par seconde pendant la lecture), donc le processeur reste au repos. |
| **Économe en données** | Qualité **Éco 96 kbit/s par défaut** (≈ 43 Mo/heure), cache audio sur disque (un titre réécouté ne coûte rien), réponses de l'API compressées (gzip), bibliothèque mise en cache et resynchronisée seulement quand elle change, pochettes minuscules (64 px) et désactivables. |
| **Sobre** | Thème **noir AMOLED** (pixels éteints sur les écrans OLED) avec le **blanc comme seul accent** : listes typographiques, gris neutres, aucune image décorative, barre de titre Windows sombre. |
| **Votre propre quota** | Bibliothèque et recherche passent par **votre application Spotify** (créée en 2 minutes sur le tableau de bord développeur, guide intégré) : fini les erreurs « limite de requêtes » du jeton partagé. |
| **Intégré à Windows** | Touches multimédia du clavier, panneau média de Windows (volume, écran de verrouillage), icône, pas de fenêtre console. |

## Fonctionnalités

- Titres likés, albums sauvegardés, playlists (y compris celles des autres), pages album et artiste.
- Recherche (titres, artistes, albums, playlists).
- Lecture continue sans blanc (gapless), aléatoire, répétition (tout / un titre), file d'attente
  (« Ajouter à la file » au clic droit), normalisation du volume.
- J'aime / je n'aime plus, liens vers l'album et l'artiste, copie du lien d'un titre.
- Filtre instantané dans une liste (sans aucune requête réseau).
- Listes virtualisées : une playlist de 10 000 titres s'affiche aussi vite qu'une de 20.
- Mode hors ligne partiel : ce qui a déjà été chargé reste consultable depuis le cache.
- Indicateurs en bas de la barre latérale : **RAM utilisée** (la même valeur que le Gestionnaire
  des tâches) et **données reçues** pendant la session.

## Économie de données en détail

| Poste | Ce que fait SpotiLite |
|---|---|
| Audio | 96 kbit/s par défaut (réglable : 160 ou 320 kbit/s). Cache audio de 1 Go par défaut (0 à 4 Go). La piste suivante n'est préchargée que 30 secondes avant la fin de la piste en cours (pour enchaîner sans blanc). |
| Bibliothèque | Une playlist n'est retéléchargée que si son `snapshot_id` a changé. Les titres likés sont vérifiés avec **une seule requête** quand rien n'a changé. Les albums ne sont jamais retéléchargés. |
| API | Compression gzip, paramètre `market=from_token` (supprime les longues listes de pays), réponses réduites immédiatement aux seuls champs affichés. |
| Images | Aucune image dans les listes. Une vignette de 64 px (≈ 3 Ko) pour le titre en cours, gardée sur disque. Réglage pour ne plus télécharger aucune image. |

## Mémoire

L'objectif est de rester entre 50 et 100 Mo. Ce qui aide :

- pas de moteur web : l'interface est tessellée par egui et rendue en OpenGL, sans tampon
  d'anticrénelage, de profondeur ni de pochoir ;
- l'audio en cours de téléchargement est écrit dans des fichiers temporaires, pas gardé en mémoire ;
- les pochettes sont réduites à 128 px maximum et au plus 48 sont gardées ;
- seules les 12 dernières pages visitées restent en mémoire (les autres reviennent du cache disque) ;
- lorsque la fenêtre est réduite, SpotiLite rend à Windows la mémoire qu'il n'utilise pas
  (désactivable dans les réglages).

La valeur exacte dépend surtout du **pilote graphique** (le pilote OpenGL est chargé dans le
processus). La consommation réelle est affichée en permanence en bas à gauche : vous pouvez la
vérifier d'un coup d'œil.

## Installation

1. Téléchargez `SpotiLite-windows-x64.zip` depuis les *Releases* du dépôt (ou l'artefact
   **SpotiLite-windows-x64** de la dernière exécution de l'action *Build*).
2. Décompressez et lancez `SpotiLite.exe`. Rien d'autre à installer.

Si Windows SmartScreen affiche un avertissement (exécutable non signé) : *Informations
complémentaires* → *Exécuter quand même*.

### Première connexion (2 étapes)

**1. Votre compte.** Cliquez sur **Se connecter avec Spotify** : la page de connexion officielle
de Spotify s'ouvre dans votre navigateur. Aucun mot de passe ne transite par SpotiLite ; seul un
jeton de session est conservé pour les lancements suivants. **Un compte Premium est
obligatoire** (Spotify n'autorise pas la lecture par des clients tiers pour les comptes gratuits).

**2. Votre application Spotify.** SpotiLite affiche ensuite un mini-guide, à suivre une seule fois :

![Écran de configuration](docs/configuration.png)

1. Ouvrez le [tableau de bord Spotify](https://developer.spotify.com/dashboard) et cliquez sur
   **Create app** (gratuit ; Spotify exige un compte Premium, ce que vous avez déjà).
2. Nom et description : au choix, par exemple « SpotiLite ».
3. Dans **Redirect URIs**, ajoutez exactement `http://127.0.0.1:8898/login` (bouton *Copier* dans
   l'app) puis **Add**.
4. Cochez **Web API**, acceptez les conditions, **Save**.
5. Dans **Settings**, copiez le **Client ID** et le **Client Secret** (*View client secret*),
   collez-les dans SpotiLite puis cliquez sur **Connecter** et acceptez dans le navigateur.

Si vous écoutez avec un autre compte Spotify que celui du tableau de bord, ajoutez-le dans
**User Management**. Le Client Secret est facultatif (sans lui, SpotiLite utilise PKCE).

Pourquoi ? Le jeton de la session d'écoute est partagé par tous les lecteurs basés sur
librespot et Spotify le limite très vite (erreur 429). Votre application a son propre quota.
Les applications en mode « développement » ne peuvent lire que les playlists dont vous êtes
propriétaire ou collaborateur, et ne donnent plus les titres populaires d'un artiste : pour ces
deux cas, SpotiLite passe automatiquement par le protocole de lecture de Spotify.

**Sécurité** : le Client Secret et le jeton d'accès sont chiffrés par Windows (DPAPI) : seule
votre session Windows, sur ce PC, peut les lire. Ils ne sont envoyés qu'à Spotify.
*Réglages → Application Spotify* permet de modifier les identifiants, de changer le port de
l'URI de redirection ou d'oublier l'application.

## Raccourcis clavier

| Touche | Action |
|---|---|
| `Espace` | Lecture / pause |
| `Ctrl` + `→` / `←` | Titre suivant / précédent |
| `Ctrl` + `↑` / `↓` | Volume |
| `Ctrl` + `F` | Rechercher |
| `Ctrl` + `L` | J'aime le titre en cours |
| `Alt` + `←` ou bouton « précédent » de la souris | Retour |
| `↑` / `↓` puis `Entrée` | Parcourir une liste et lancer un titre |
| Double-clic sur un titre | Lecture |
| Clic droit sur un titre | Ajouter à la file, aller à l'album / à l'artiste, j'aime, copier le lien |
| Touches multimédia | Lecture, pause, suivant, précédent |

## Fichiers

- Réglages, jeton de session et identifiants de l'application (chiffrés) : `%APPDATA%\SpotiLite`
- Caches (audio, bibliothèque, pochettes) et journal `spotilite.log` : `%LOCALAPPDATA%\SpotiLite`
- **Mode portable** : créez un dossier `spotilite-data` à côté de `SpotiLite.exe` ; tout y sera
  enregistré.
- *Réglages → Se déconnecter* supprime le jeton de session et les caches de bibliothèque
  (les identifiants de l'application restent, pour se reconnecter en un clic).

## Limites connues

- Pas de podcasts, de paroles, ni de mode « Spotify Connect » (piloter SpotiLite depuis le
  téléphone) : ces fonctions ont été laissées de côté pour rester léger.
- Qualité maximale 320 kbit/s (pas de lossless).
- **Titres refusés par Spotify (code `0x0001`)** : Spotify décide titre par titre s'il accorde la
  clé de déchiffrement « classique » utilisée par les lecteurs libres. Les titres soumis à
  certaines licences ne sont plus déchiffrables que par les applications officielles, via le DRM
  de Spotify (PlayPlay). Aucun lecteur tiers ne peut les lire légitimement, et SpotiLite ne
  contourne pas ce DRM (ce serait illégal et exposerait votre compte à un bannissement).
  SpotiLite saute ces titres sans interrompre la lecture, les affiche en grisé et ne les retente
  plus pendant 14 jours (aucune donnée gaspillée). Un double-clic permet de réessayer. Si Spotify
  refuse 10 titres d'affilée, la lecture s'arrête : le refus concerne alors sans doute tout le
  compte ([librespot#1649](https://github.com/librespot-org/librespot/issues/1649)).
- **Refus temporaires (`0x0002`) ou absence de réponse** : SpotiLite embarque le correctif
  proposé en amont ([librespot#1763](https://github.com/librespot-org/librespot/pull/1763)) et
  redemande la clé jusqu'à 3 fois, puis se reconnecte une fois avant d'abandonner.
- SpotiLite est un client **non officiel**, sans lien avec Spotify AB. Il repose sur librespot,
  comme de nombreux lecteurs libres ; Spotify peut modifier son protocole à tout moment.

## Compiler

Prérequis : [Rust](https://rustup.rs) stable ≥ 1.95 et, sous Windows, les *Build Tools* de
Visual Studio (le SDK Windows fournit `rc.exe` pour l'icône).

```powershell
cargo build --release
# => target\release\spotilite.exe
```

Compilation croisée depuis Linux : `rustup target add x86_64-pc-windows-gnu`, installez
`mingw-w64`, puis `cargo build --release --target x86_64-pc-windows-gnu`.

Tests : `cargo test`. En build de debug, `SPOTILITE_DEMO=1` affiche l'interface avec des
données fictives (utile pour travailler sur l'interface sans compte) ; ajoutez
`SPOTILITE_DEMO_SETUP=1` pour l'écran de configuration.

### Architecture

```
src/
├── main.rs          fenêtre, options de rendu, icône dessinée par le code
├── ui/              interface egui (thème, vues, widgets et icônes vectorielles)
├── backend/         thread réseau/audio (Tokio, 2 threads)
│   ├── mod.rs       session librespot, lecteur, commandes, cache de bibliothèque
│   ├── auth.rs      OAuth 2.0 dans le navigateur (Client Secret ou PKCE)
│   ├── webapi.rs    client minimal de l'API Web (gzip, pagination, reprise sur 429)
│   ├── vault.rs     secrets chiffrés avec DPAPI sous Windows
│   └── store.rs     cache disque (JSON et vignettes)
vendor/librespot-core/   librespot-core 0.8.0 + nouvelles tentatives sur les clés audio
├── queue.rs         file de lecture locale (aléatoire, répétition, ajouts)
├── media.rs         touches multimédia et panneau média de Windows
└── sys.rs           mesure et libération de la mémoire
```

L'interface envoie des commandes au backend et reçoit des événements par des canaux ; le
backend réveille l'interface seulement quand il a quelque chose à afficher.

## Licence

MIT. SpotiLite n'est ni affilié à Spotify ni approuvé par Spotify. « Spotify » est une marque
de Spotify AB.
