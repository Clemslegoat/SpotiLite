# egui_software_backend pour SpotiLite

Copie de [egui_software_backend](https://github.com/DGriffin91/egui_software_backend) 0.0.3
(MIT OU Apache-2.0, fichiers `LICENSE-*`), le rastériseur egui sur processeur, portée à egui 0.36 :

- `egui::ahash::HashMap` → `std::collections::HashMap` (egui 0.36 ne réexporte plus ahash) ;
- les `TexturesDelta` d'egui 0.36 regroupent plusieurs `ImageDelta` par texture : ils sont
  appliqués un par un (`set_texture`) ;
- ajout de `apply_textures`, pour appliquer les textures sans dessiner (fenêtre réduite) ;
- l'intégration winit d'origine, les exemples, tests et options rayon/log ne sont pas repris :
  SpotiLite a sa propre boucle de fenêtre (`src/window.rs`).
