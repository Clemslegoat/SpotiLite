# SpotiLite

Client Spotify **Premium** natif pour Windows, pensé pour consommer le moins possible :
un seul exécutable, **objectif 50 à 100 Mo de RAM**, et une consommation de données maîtrisée.

![Aperçu de SpotiLite](docs/apercu.png)

SpotiLite n'est pas une version web de Spotify : l'interface est dessinée directement par
l'application (Rust + [egui](https://github.com/emilk/egui), rendu OpenGL), sans navigateur
embarqué ni Electron. Le son est décodé localement grâce à
[librespot](https://github.com/librespot-org/librespot), l'implémentation libre du protocole
de lecture Spotify, ou, au choix, par le **moteur officiel** de Spotify (voir plus bas) quand
Spotify refuse ses clés à librespot.

## Points forts

| | |
|---|---|
| **Léger** | Un seul `.exe` (~12 Mo) sans DLL à installer. Pas de moteur web : l'interface ne se redessine que lorsqu'il se passe quelque chose (au plus une fois par seconde pendant la lecture), donc le processeur reste au repos. |
| **Économe en données** | Qualité **Éco 96 kbit/s par défaut** (≈ 43 Mo/heure), cache audio sur disque (un titre réécouté ne coûte rien), réponses de l'API compressées (gzip), bibliothèque mise en cache et resynchronisée seulement quand elle change, pochettes minuscules (64 px) et désactivables. |
| **Sobre** | Thème **noir AMOLED** (pixels éteints sur les écrans OLED) avec le **blanc comme seul accent** : listes typographiques, gris neutres, aucune image décorative, barre de titre Windows sombre. |
| **Votre propre quota** | Bibliothèque et recherche passent par **votre application Spotify** (créée en 2 minutes sur le tableau de bord développeur, guide intégré) : fini les erreurs « limite de requêtes » du jeton partagé. |
| **Deux moteurs de lecture** | **SpotiLite** (librespot, le plus léger) ou **Officiel Spotify** : le lecteur de Spotify lui-même, invisible, déchiffré par le DRM du moteur Edge (Widevine ou PlayReady), comme dans un navigateur. Il lit les titres que Spotify refuse à librespot, contre plus de mémoire. |
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
| Audio | 96 kbit/s par défaut (réglable : 160 ou 320 kbit/s). Cache audio de 1 Go par défaut (0 à 4 Go). La piste suivante n'est préchargée que 30 secondes avant la fin de la piste en cours (pour enchaîner sans blanc). Avec le moteur officiel, c'est Spotify qui fixe le débit. |
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
vérifier d'un coup d'œil. Le moteur officiel, optionnel, ajoute ses propres processus WebView2
(≈ 100 à 150 Mo), comptés à part.

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
4. Cochez **Web API** et **Web Playback SDK**, acceptez les conditions, **Save**.
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

## Moteur de lecture

*Réglages → Lecture → Moteur de lecture* :

| | SpotiLite (librespot) | Officiel Spotify (WebView2) |
|---|---|---|
| Mémoire | ≈ 50 Mo | ≈ 100 à 150 Mo **en plus** (processus Microsoft Edge WebView2 ; 134 Mo mesurés sous Windows) |
| Qualité | 96, 160 ou 320 kbit/s, au choix | choisie par Spotify (AAC, en général 128 à 256 kbit/s) |
| Cache audio | oui | non |
| Titres refusés par Spotify (`0x0001`) | sautés | **lus** |

Le moteur officiel est le [Web Playback SDK](https://developer.spotify.com/documentation/web-playback-sdk)
de Spotify, chargé dans une page invisible de WebView2, le moteur Edge intégré à Windows 10 et
11. Spotify y déchiffre l'audio avec le **DRM du moteur Edge** (Widevine, ou PlayReady, le DRM de
Windows), exactement comme dans un navigateur : SpotiLite ne contourne aucune protection, il pilote ce lecteur (lecture, pause,
position, volume) et garde son interface, sa file d'attente et ses raccourcis. Le titre à lire est
envoyé par l'API Web de votre application (`PUT /me/player/play`, une requête par titre).

Pour l'utiliser :

- votre application Spotify doit avoir **Web Playback SDK** coché (tableau de bord → *Settings* →
  *Edit*) ;
- si votre application a été autorisée avec une version précédente de SpotiLite, une nouvelle
  autorisation est demandée une fois (les droits de lecture `streaming` et
  `user-modify-playback-state` s'ajoutent) : le navigateur s'ouvre quand vous choisissez ce
  moteur, ou cliquez sur *Réglages → Application Spotify → Autoriser la lecture* ;
- Microsoft Edge WebView2 doit être présent (c'est le cas sur Windows 11 et sur Windows 10 à
  jour ; sinon : [installation](https://go.microsoft.com/fwlink/p/?LinkId=2124703)).

L'état du moteur (démarrage, prêt, DRM utilisé) et sa mémoire sont affichés dans *Réglages →
Lecture* ; la RAM en bas à gauche devient « RAM SpotiLite + moteur ». Le moteur démarre à la
première lecture et s'arrête quand vous revenez au moteur SpotiLite. Pour le diagnostic,
`SPOTILITE_WEBVIEW_DEBUG=1` affiche sa fenêtre et ouvre les outils de développement.

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
  téléphone) : ces fonctions ont été laissées de côté pour rester léger. Avec le moteur
  officiel, SpotiLite apparaît toutefois comme appareil « SpotiLite » dans vos applications
  Spotify pendant la lecture.
- Qualité maximale 320 kbit/s (pas de lossless).
- **Titres refusés par Spotify (code `0x0001`)** : depuis 2025, Spotify refuse par moments la
  clé de déchiffrement « classique » utilisée par les lecteurs libres basés sur librespot
  ([librespot#1649](https://github.com/librespot-org/librespot/issues/1649)). La cause exacte
  n'est pas publique : les développeurs de go-librespot pensent à une décision titre par titre
  liée aux licences (les applications officielles passant par un DRM). Avec le moteur SpotiLite,
  le titre est réessayé en 160 puis 320 kbit/s (la qualité qui fonctionne est gardée) ; un titre
  refusé dans toutes les qualités est sauté, grisé et non retenté pendant 14 jours (*Réglages →
  Lecture → Réessayer ces titres*). Si 10 titres d'affilée sont refusés, la lecture s'arrête.
  **Solution : le moteur officiel** (ci-dessus), qui passe par le DRM d'Edge et n'est pas concerné.
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
`SPOTILITE_DEMO_SETUP=1` pour l'écran de configuration, `SPOTILITE_DEMO_OFFICIAL=1` pour l'état du
moteur officiel. `cargo test -- --ignored --nocapture` lance sous Windows un test réel du moteur
officiel (WebView2 + lecteur de Spotify, réseau nécessaire).

### Architecture

```
src/
├── main.rs          fenêtre, options de rendu, icône dessinée par le code
├── ui/              interface egui (thème, vues, widgets et icônes vectorielles)
├── backend/         thread réseau/audio (Tokio, 2 threads)
│   ├── mod.rs       session librespot, lecteur, commandes, cache de bibliothèque
│   ├── auth.rs      OAuth 2.0 dans le navigateur (Client Secret ou PKCE)
│   ├── official/    moteur officiel : WebView2 invisible + Web Playback SDK
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
