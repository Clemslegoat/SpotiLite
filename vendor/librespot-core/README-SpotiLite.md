# librespot-core 0.8.0 (copie modifiée)

Copie de [librespot-core](https://crates.io/crates/librespot-core) 0.8.0
(licence MIT, © les contributeurs de librespot), utilisée via `[patch.crates-io]`.

Modification principale : `src/audio_key.rs`, qui reprend le principe de la PR
[librespot-org/librespot#1763](https://github.com/librespot-org/librespot/pull/1763)
(non encore publiée) :

- le code d'erreur envoyé par Spotify est conservé (`0x0001` refus définitif,
  `0x0002` refus temporaire) ;
- une demande de clé refusée temporairement ou restée sans réponse est retentée
  jusqu'à 3 fois (1 s puis 2 s d'attente) au lieu de faire échouer le titre ;
- les demandes expirées sont retirées de la table des demandes en attente ;
- délai de réponse porté de 1,5 s à 2,5 s.

Autre retouche mineure : `#[expect(deprecated)]` remplacé par `#[allow(deprecated)]`
dans `src/authentication.rs` (l'attente n'est plus remplie avec les compilateurs récents,
ce qui produisait un avertissement).

À supprimer dès qu'une version de librespot intégrant ce correctif est publiée.
