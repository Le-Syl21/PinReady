# 🎯 PinReady

🇬🇧 [English](#-english) | 🇫🇷 [Français](#-français)

---

## 🇬🇧 English

Cross-platform configurator and launcher for [Visual Pinball](https://github.com/vpinball/vpinball) standalone (10.8.1).

PinReady replaces the non-existent native configuration tools for VPX standalone builds (SDL3/bgfx). It guides you through setting up a virtual pinball cabinet from scratch: screens, inputs, tilt, audio, then lets you browse and launch tables from a single interface. 🕹️

> ⚠️ **Scope — please read before installing**
>
> PinReady is designed to **support the development and adoption of VPX 10.8.1**, not to replace a stable production VPX setup. The target audience is testers and early adopters of the new 10.8.1 architecture (integrated plugins, folder-per-table layout, SDL3/bgfx backend). If you run a stable VPX 10.7.x production cabinet, PinReady is probably not for you yet.

### ✨ Features

**🧙 Configuration wizard (first run)**

- 📥 **Visual Pinball auto-install** -- Automatically download and install the correct Visual Pinball build for your platform (Linux/macOS/Windows, x64/aarch64/SBC)
- 🖥️ **Screen assignment** -- Detect displays via SDL3, auto-assign roles (Playfield, Backglass, DMD, Topper) by size, configure multi-screen positioning and cabinet physical dimensions
- 🎨 **Rendering** -- Anti-aliasing, FXAA, sharpening, reflections, texture limits, sync mode, max framerate
- 🎮 **Input mapping** -- Capture keyboard and joystick bindings for all VPX actions, auto-detect pinball controllers (Pinscape KL25Z, Pinscape Pico, DudesCab), conflict warnings
- 📐 **Tilt & nudge** -- Configure accelerometer sensitivity with simplified or advanced controls
- 🔊 **Audio routing** -- Assign playfield and backglass audio devices, configure SSF surround modes (6 modes), test speaker wiring with built-in audio sequences (music, ball sounds, knocker)
- 📁 **Tables directory** -- Select the root folder containing your tables (folder-per-table layout)
- 🌍 **Internationalization** -- 20+ languages: 🇬🇧 🇫🇷 🇩🇪 🇪🇸 🇮🇹 🇵🇹 🇳🇱 🇸🇪 🇫🇮 🇵🇱 🇨🇿 🇸🇰 🇷🇺 🇹🇷 🇸🇦 🇮🇳 🇧🇩 🇹🇭 🇻🇳 🇮🇩 🇰🇪 🇨🇳 🇹🇼 🇯🇵 🇰🇷

**🚀 Table launcher (subsequent runs)**

- 🗂️ **Table browser** -- Scan folder-per-table directories, display backglass thumbnails extracted from `.directb2s` files
- 📺 **Multi-screen layout** -- Table selector on DMD, backglass preview on BG display
- ⚡ **VPX integration** -- Launch tables with loading progress overlay, parse VPX stdout for real-time status
- 🔄 **Auto-update** -- Checks for new Visual Pinball releases on startup, one-click update from the launcher
- 🕹️ **Input navigation** -- Browse and launch tables with joystick (flippers, start) or keyboard

### 🎯 Target

- 🎰 **Visual Pinball 10.8.1** -- Uses the folder-per-table layout
- 💻 **Cross-platform** -- Linux, macOS, Windows. SDL3 only, no platform-specific APIs
- 📦 **No system dependencies** -- SDL3 and SQLite are statically linked

### 📥 Download

Grab the latest release for your platform -- no install needed, just download and run:

👉 **[Download PinReady](https://github.com/Le-Syl21/PinReady/releases/latest)** (Linux, macOS, Windows)

📹 **[Video demo — YouTube playlist (EN + FR)](https://www.youtube.com/playlist?list=PLZ838nY4NE902Am7NIOGYbEJaak5pEP71)**

### 🔨 Build from source

If you prefer to compile it yourself:

**🐧 Linux:**

```bash
sudo apt install build-essential cmake pkg-config \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev

cargo build --release
```

**🍎 macOS / 🪟 Windows:**

```bash
cargo build --release
```

SDL3 and SQLite compile from source automatically -- no manual installation needed. ✅

### 🚀 Usage

**First run** (no existing database) launches the configuration wizard.
**Subsequent runs** go directly to the table launcher. The wizard can be re-launched at any time.

```bash
# Run with debug logging
RUST_LOG=info cargo run

# Or run the release binary directly
./target/release/pinready
```

**📋 Requirements:**

- 🎰 **Visual Pinball** executable (10.8.1+) -- auto-installed or path configured in the wizard
- 📁 **Tables directory** -- folder-per-table layout as described in VPX docs
- 🌐 **Internet connection** -- required for auto-install and update checks (optional for manual install)

**📂 File locations** (auto-resolved via OS conventions — you do not need to create these folders yourself):

| What | Linux | macOS | Windows |
|---|---|---|---|
| **VPinballX.ini** (PinReady reads/writes) | `~/.local/share/VPinballX/10.8/VPinballX.ini` | `~/Library/Application Support/VPinballX/10.8/VPinballX.ini` | `%APPDATA%\VPinballX\10.8\VPinballX.ini` |
| **PinReady DB + log** | `~/.local/share/pinready/` | `~/Library/Application Support/pinready/` | `%APPDATA%\pinready\` |
| **DOF config** (extract the VPUniverse zip here) | `~/.local/share/VPinballX/10.8/directoutputconfig/` | `~/Library/Application Support/VPinballX/10.8/directoutputconfig/` | `%APPDATA%\VPinballX\10.8\directoutputconfig\` |

**🎮 Launcher controls:**

| Action | 🖱️ Mouse | ⌨️ Keyboard | 🕹️ Joystick |
|---|---|---|---|
| Previous/next table | Hover | Arrow Left/Right | Left/Right Flipper |
| Previous/next row | -- | Arrow Up/Down | Left/Right MagnaSave |
| Jump by one viewport | Wheel flick | PageUp/PageDown | -- |
| First/last table | -- | Home/End | -- |
| Scroll view (no selection change) | Mouse wheel | -- | -- |
| Launch table | Click | Enter | Start |
| Open config | -- | -- | Launch Ball |
| Quit launcher | -- | Escape | ExitGame |

**📺 Multi-screen launcher layout:**

| Screens | Playfield | Backglass | DMD | Topper |
|---|---|---|---|---|
| **1** | Table grid | -- | -- | -- |
| **2** | Table grid (fullscreen) | Backglass preview of hovered table | -- | -- |
| **3** | VPX logo cover | Backglass preview of hovered table | Table grid (fullscreen) | -- |
| **4** | VPX logo cover | Backglass preview of hovered table | Table grid (fullscreen) | VPX logo cover |

When a table is launched, all cover viewports are hidden to let VPX take over the screens.

**🕹️ Supported pinball controllers:**

PinReady auto-detects pinball controllers and applies default button mappings. The profile can be changed in the wizard.

<details>
<summary>KL25Z (KL Shield V5.1 / Brain / Rig Master) — 21 buttons</summary>

| Btn | Label | VPX Action |
|---|---|---|
| 0 | START | Start |
| 1 | EXTRA-B | ExtraBall |
| 2 | COIN1 | Credit1 |
| 3 | COIN2 | Credit2 |
| 4 | L BALL | LaunchBall |
| 5 | EXIT | ExitGame |
| 6 | QUIT | *(VP editor)* |
| 7 | L FLIPP | LeftFlipper + LeftStagedFlipper |
| 8 | R FLIPP | RightFlipper + RightStagedFlipper |
| 9 | L MAGNA | LeftMagna |
| 10 | R MAGNA | RightMagna |
| 11 | FIRE | Lockbar |
| 12 | TILT | Tilt |
| 13 | DOOR | CoinDoor |
| 14 | SERVICE EXIT | Service1 |
| 15 | SERVICE - | Service2 |
| 16 | SERVICE + | Service3 |
| 17 | ENTER | Service4 |
| 18 | N.M. | *(Night Mode)* |
| 19 | VOL- | VolumeDown |
| 20 | VOL+ | VolumeUp |

</details>

<details>
<summary>Pinscape Pico (OpenPinballDevice) — 27 buttons</summary>

| Btn | Function | VPX Action |
|---|---|---|
| 0 | Start | Start |
| 1 | Exit | ExitGame |
| 2 | Extra Ball | ExtraBall |
| 3–6 | Coin 1–4 | Credit1–4 |
| 7 | Launch Ball | LaunchBall |
| 8 | Fire | Lockbar |
| 9 | Left Flipper | LeftFlipper |
| 10 | Right Flipper | RightFlipper |
| 11 | Upper Left Flipper | LeftStagedFlipper |
| 12 | Upper Right Flipper | RightStagedFlipper |
| 13 | MagnaSave Left | LeftMagna |
| 14 | MagnaSave Right | RightMagna |
| 15 | Tilt Bob | Tilt |
| 16 | Slam Tilt | SlamTilt |
| 17 | Coin Door | CoinDoor |
| 18–21 | Service 1–4 | Service1–4 |
| 22 | Left Nudge | LeftNudge |
| 23 | Forward Nudge | CenterNudge |
| 24 | Right Nudge | RightNudge |
| 25 | Volume Up | VolumeUp |
| 26 | Volume Down | VolumeDown |

</details>

<details>
<summary>DudesCab (Arnoz) — 32 buttons</summary>

| Btn | Label | VPX Action |
|---|---|---|
| 0 | Start | Start |
| 1 | ExtraBall | ExtraBall |
| 2 | Coin1 | Credit1 |
| 3 | Coin2 | Credit2 |
| 4 | LaunchBall | LaunchBall |
| 5 | Return | ExitGame |
| 6 | Exit | *(Quit to editor)* |
| 7 | Flipper Left | LeftFlipper + LeftStagedFlipper |
| 8 | Flipper Right | RightFlipper + RightStagedFlipper |
| 9 | Magna Left | LeftMagna |
| 10 | Magna Right | RightMagna |
| 11 | Tilt | Tilt |
| 12 | Fire | Lockbar |
| 13 | Door | CoinDoor |
| 14–17 | ROM Exit/−/+/Enter | Service1–4 |
| 18 | VOL − | VolumeDown |
| 19 | VOL + | VolumeUp |
| 20–23 | DPAD | *(Hat navigation)* |
| 24 | NightMode | *(DO NOT REMAP)* |
| 25–30 | Spare 1–6 | *(User-defined)* |
| 31 | Calib | *(DO NOT REMAP)* |

</details>

### 🧩 Table preparation

Some tables need a bit of prep to behave correctly under VPX Standalone 10.8.1. Optional steps, but worth knowing:

**🔧 Patching old tables for VPX Standalone**

Some older tables use VBScript features that don't behave identically under VPX Standalone's scripting engine. The community-maintained [`jsm174/vpx-standalone-scripts`](https://github.com/jsm174/vpx-standalone-scripts) repo provides patched `.vbs` sidecar scripts for hundreds of tables.

Workflow:
1. Find the `.vbs` file matching your table name in the repo
2. Place it next to your `.vpx` (same folder, same base name)
3. VPX Standalone automatically uses the sidecar script instead of the embedded one

**⚙️ Per-table settings (custom ini)**

Global settings live in `VPinballX.ini`. To override them for a specific table, two options:

**Option A — Sidecar ini (simplest, no tool needed)**

Drop a `.ini` next to your `.vpx` with the same base name. VPX picks it up automatically:

```
MyTable/
├── MyTable.vpx
└── MyTable.ini   # overrides global VPinballX.ini for this table only
```

Only include the keys you want to override — everything else falls back to the global ini.

**Option B — Embed the ini inside the .vpx (via vpxtool)**

If you prefer keeping the settings inside the table file itself, use [vpxtool](https://github.com/francisdb/vpxtool) — grab a binary from the [latest release](https://github.com/francisdb/vpxtool/releases/latest) (Linux, macOS, Windows):

```bash
# 1. Extract the .vpx into a directory structure
vpxtool extract MyTable.vpx

# 2. Edit MyTable/MyTable.ini with your overrides

# 3. Reassemble the .vpx with the ini embedded
vpxtool assemble MyTable/
```

**🎨 Bonus — shrink table size (WebP conversion)**

VPX tables often ship with large lossless BMP/PNG images. `vpxtool` can batch-convert them to WebP:

```bash
vpxtool images webp MyTable.vpx
```

Useful when you have dozens of tables eating disk space.

---

## 🇫🇷 Français

Configurateur et lanceur multiplateforme pour [Visual Pinball](https://github.com/vpinball/vpinball) standalone (10.8.1).

PinReady remplace les outils de configuration natifs inexistants pour les builds VPX standalone (SDL3/bgfx). Il vous guide dans la mise en place d'un flipper virtuel depuis zéro : écrans, contrôles, tilt, audio, puis permet de parcourir et lancer vos tables depuis une interface unique. 🕹️

> ⚠️ **Périmètre — à lire avant d'installer**
>
> PinReady est conçu pour **accompagner le développement et l'adoption de VPX 10.8.1**, pas pour remplacer une installation VPX stable en production. Le public cible, ce sont les testeurs et les early adopters de la nouvelle architecture 10.8.1 (plugins intégrés, format dossier-par-table, backend SDL3/bgfx). Si vous avez un cabinet VPX 10.7.x stable en production, PinReady n'est probablement pas encore pour vous.

### ✨ Fonctionnalités

**🧙 Assistant de configuration (premier lancement)**

- 📥 **Installation automatique de Visual Pinball** -- Télécharge et installe automatiquement le bon build Visual Pinball pour votre plateforme (Linux/macOS/Windows, x64/aarch64/SBC)
- 🖥️ **Affectation des écrans** -- Détection des écrans via SDL3, affectation automatique des rôles (Playfield, Backglass, DMD, Topper) par taille, configuration du positionnement multi-écran et des dimensions physiques du cabinet
- 🎨 **Rendu** -- Anti-aliasing, FXAA, netteté, reflets, limites de texture, mode sync, framerate max
- 🎮 **Mapping des contrôles** -- Capture des touches clavier et boutons joystick pour toutes les actions VPX, détection automatique des contrôleurs pinball (Pinscape KL25Z, Pinscape Pico, DudesCab), avertissements de conflits
- 📐 **Tilt & nudge** -- Configuration de la sensibilité de l'accéléromètre en mode simplifié ou avancé
- 🔊 **Routage audio** -- Affectation des périphériques audio playfield et backglass, configuration des modes surround SSF (6 modes), test du câblage des enceintes avec séquences audio intégrées (musique, bruits de bille, knocker)
- 📁 **Répertoire des tables** -- Sélection du dossier racine contenant vos tables (format dossier-par-table)
- 🌍 **Internationalisation** -- 20+ langues : 🇬🇧 🇫🇷 🇩🇪 🇪🇸 🇮🇹 🇵🇹 🇳🇱 🇸🇪 🇫🇮 🇵🇱 🇨🇿 🇸🇰 🇷🇺 🇹🇷 🇸🇦 🇮🇳 🇧🇩 🇹🇭 🇻🇳 🇮🇩 🇰🇪 🇨🇳 🇹🇼 🇯🇵 🇰🇷

**🚀 Lanceur de tables (lancements suivants)**

- 🗂️ **Navigateur de tables** -- Scan des répertoires dossier-par-table, affichage des miniatures backglass extraites des fichiers `.directb2s`
- 📺 **Affichage multi-écran** -- Sélecteur de table sur le DMD, aperçu du backglass sur l'écran BG
- ⚡ **Intégration VPX** -- Lancement des tables avec overlay de progression, lecture du stdout VPX pour le statut en temps réel
- 🔄 **Mise à jour automatique** -- Vérifie les nouvelles releases Visual Pinball au démarrage, mise à jour en un clic depuis le lanceur
- 🕹️ **Navigation aux contrôles** -- Parcourir et lancer les tables au joystick (flippers, start) ou au clavier

### 🎯 Cible

- 🎰 **Visual Pinball 10.8.1** -- Utilise le format dossier-par-table
- 💻 **Multiplateforme** -- Linux, macOS, Windows. SDL3 uniquement, aucune API spécifique à une plateforme
- 📦 **Aucune dépendance système** -- SDL3 et SQLite sont liés statiquement

### 📥 Téléchargement

Téléchargez la dernière version pour votre plateforme -- pas d'installation, il suffit de lancer :

👉 **[Télécharger PinReady](https://github.com/Le-Syl21/PinReady/releases/latest)** (Linux, macOS, Windows)

📹 **[Démo vidéo — playlist YouTube (FR + EN)](https://www.youtube.com/playlist?list=PLZ838nY4NE902Am7NIOGYbEJaak5pEP71)**

### 🔨 Compilation depuis les sources

Si vous préférez compiler vous-même :

**🐧 Linux :**

```bash
sudo apt install build-essential cmake pkg-config \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev

cargo build --release
```

**🍎 macOS / 🪟 Windows :**

```bash
cargo build --release
```

SDL3 et SQLite se compilent depuis les sources automatiquement -- aucune installation manuelle nécessaire. ✅

### 🚀 Utilisation

**Premier lancement** (pas de base de données existante) : lance l'assistant de configuration.
**Lancements suivants** : accès direct au lanceur de tables. L'assistant peut être relancé à tout moment.

```bash
# Lancer avec les logs de debug
RUST_LOG=info cargo run

# Ou lancer directement le binaire release
./target/release/pinready
```

**📋 Prérequis :**

- 🎰 **Visual Pinball** exécutable (10.8.1+) -- installé automatiquement ou chemin configuré dans l'assistant
- 📁 **Répertoire de tables** -- format dossier-par-table tel que décrit dans la doc VPX
- 🌐 **Connexion internet** -- nécessaire pour l'installation automatique et la vérification des mises à jour (optionnel pour l'installation manuelle)

**📂 Emplacements des fichiers** (résolus automatiquement via les conventions OS — vous n'avez pas à créer ces dossiers vous-même) :

| Quoi | Linux | macOS | Windows |
|---|---|---|---|
| **VPinballX.ini** (PinReady lit/écrit) | `~/.local/share/VPinballX/10.8/VPinballX.ini` | `~/Library/Application Support/VPinballX/10.8/VPinballX.ini` | `%APPDATA%\VPinballX\10.8\VPinballX.ini` |
| **BDD + log PinReady** | `~/.local/share/pinready/` | `~/Library/Application Support/pinready/` | `%APPDATA%\pinready\` |
| **Config DOF** (décompresser le zip VPUniverse ici) | `~/.local/share/VPinballX/10.8/directoutputconfig/` | `~/Library/Application Support/VPinballX/10.8/directoutputconfig/` | `%APPDATA%\VPinballX\10.8\directoutputconfig\` |

**🎮 Contrôles du lanceur :**

| Action | 🖱️ Souris | ⌨️ Clavier | 🕹️ Joystick |
|---|---|---|---|
| Table précédente/suivante | Survol | Flèche Gauche/Droite | Flipper Gauche/Droit |
| Ligne précédente/suivante | -- | Flèche Haut/Bas | MagnaSave Gauche/Droit |
| Saut d'un viewport | Flick molette | PageUp/PageDown | -- |
| Première/dernière table | -- | Home/End | -- |
| Scroll visuel (sans changer la sélection) | Molette | -- | -- |
| Lancer une table | Clic | Entrée | Start |
| Ouvrir la config | -- | -- | Launch Ball |
| Quitter le lanceur | -- | Échap | ExitGame |

**📺 Disposition multi-écran du lanceur :**

| Écrans | Playfield | Backglass | DMD | Topper |
|---|---|---|---|---|
| **1** | Grille tables | -- | -- | -- |
| **2** | Grille tables (plein écran) | Aperçu backglass de la table survolée | -- | -- |
| **3** | Logo VPX (cover) | Aperçu backglass de la table survolée | Grille tables (plein écran) | -- |
| **4** | Logo VPX (cover) | Aperçu backglass de la table survolée | Grille tables (plein écran) | Logo VPX (cover) |

Au lancement d'une table, tous les viewports de couverture sont masqués pour laisser VPX prendre le contrôle des écrans.

**🕹️ Contrôleurs pinball supportés :**

PinReady détecte automatiquement les contrôleurs pinball et applique le mapping par défaut. Le profil est modifiable dans l'assistant.

<details>
<summary>KL25Z (KL Shield V5.1 / Brain / Rig Master) — 21 boutons</summary>

| Btn | Sérigraphie | Action VPX |
|---|---|---|
| 0 | START | Start |
| 1 | EXTRA-B | ExtraBall |
| 2 | COIN1 | Credit1 |
| 3 | COIN2 | Credit2 |
| 4 | L BALL | LaunchBall |
| 5 | EXIT | ExitGame |
| 6 | QUIT | *(éditeur VP)* |
| 7 | L FLIPP | LeftFlipper + LeftStagedFlipper |
| 8 | R FLIPP | RightFlipper + RightStagedFlipper |
| 9 | L MAGNA | LeftMagna |
| 10 | R MAGNA | RightMagna |
| 11 | FIRE | Lockbar |
| 12 | TILT | Tilt |
| 13 | DOOR | CoinDoor |
| 14 | SERVICE EXIT | Service1 |
| 15 | SERVICE - | Service2 |
| 16 | SERVICE + | Service3 |
| 17 | ENTER | Service4 |
| 18 | N.M. | *(Night Mode)* |
| 19 | VOL- | VolumeDown |
| 20 | VOL+ | VolumeUp |

</details>

<details>
<summary>Pinscape Pico (OpenPinballDevice) — 27 boutons</summary>

| Btn | Fonction | Action VPX |
|---|---|---|
| 0 | Start | Start |
| 1 | Exit | ExitGame |
| 2 | Extra Ball | ExtraBall |
| 3–6 | Coin 1–4 | Credit1–4 |
| 7 | Launch Ball | LaunchBall |
| 8 | Fire | Lockbar |
| 9 | Flipper Gauche | LeftFlipper |
| 10 | Flipper Droit | RightFlipper |
| 11 | Upper Flipper Gauche | LeftStagedFlipper |
| 12 | Upper Flipper Droit | RightStagedFlipper |
| 13 | MagnaSave Gauche | LeftMagna |
| 14 | MagnaSave Droit | RightMagna |
| 15 | Tilt Bob | Tilt |
| 16 | Slam Tilt | SlamTilt |
| 17 | Porte monnayeur | CoinDoor |
| 18–21 | Service 1–4 | Service1–4 |
| 22 | Nudge Gauche | LeftNudge |
| 23 | Nudge Centre | CenterNudge |
| 24 | Nudge Droit | RightNudge |
| 25 | Volume + | VolumeUp |
| 26 | Volume − | VolumeDown |

</details>

<details>
<summary>DudesCab (Arnoz) — 32 boutons</summary>

| Btn | Label | Action VPX |
|---|---|---|
| 0 | Start | Start |
| 1 | ExtraBall | ExtraBall |
| 2 | Coin1 | Credit1 |
| 3 | Coin2 | Credit2 |
| 4 | LaunchBall | LaunchBall |
| 5 | Return | ExitGame |
| 6 | Exit | *(Quit éditeur)* |
| 7 | Flipper Left | LeftFlipper + LeftStagedFlipper |
| 8 | Flipper Right | RightFlipper + RightStagedFlipper |
| 9 | Magna Left | LeftMagna |
| 10 | Magna Right | RightMagna |
| 11 | Tilt | Tilt |
| 12 | Fire | Lockbar |
| 13 | Door | CoinDoor |
| 14–17 | ROM Exit/−/+/Enter | Service1–4 |
| 18 | VOL − | VolumeDown |
| 19 | VOL + | VolumeUp |
| 20–23 | DPAD | *(Navigation hat)* |
| 24 | NightMode | *(NE PAS REMAPPER)* |
| 25–30 | Spare 1–6 | *(Libre)* |
| 31 | Calib | *(NE PAS REMAPPER)* |

</details>

### 🧩 Préparation des tables

Certaines tables demandent un peu de préparation pour bien fonctionner sous VPX Standalone 10.8.1. Étapes optionnelles, mais bon à savoir :

**🔧 Patcher les anciennes tables pour VPX Standalone**

Certaines tables anciennes utilisent des particularités VBScript qui ne se comportent pas à l'identique sous le moteur de script de VPX Standalone. Le dépôt communautaire [`jsm174/vpx-standalone-scripts`](https://github.com/jsm174/vpx-standalone-scripts) fournit des scripts `.vbs` patchés pour des centaines de tables, à utiliser en sidecar.

Procédure :
1. Trouvez le fichier `.vbs` correspondant au nom de votre table dans le dépôt
2. Placez-le à côté de votre `.vpx` (même dossier, même nom de base)
3. VPX Standalone utilise automatiquement le script sidecar à la place du script embarqué

**⚙️ Réglages par table (ini personnalisé)**

Les réglages globaux sont dans `VPinballX.ini`. Pour les surcharger pour une table donnée, deux options :

**Option A — Ini sidecar (le plus simple, sans outil)**

Déposez un `.ini` à côté de votre `.vpx` avec le même nom de base. VPX le prend automatiquement :

```
MaTable/
├── MaTable.vpx
└── MaTable.ini   # surcharge le VPinballX.ini global pour cette table uniquement
```

N'incluez que les clés à surcharger — tout le reste retombe sur l'ini global.

**Option B — Embarquer l'ini dans le .vpx (via vpxtool)**

Si vous préférez garder les réglages dans le fichier de table lui-même, utilisez [vpxtool](https://github.com/francisdb/vpxtool) — récupérez un binaire sur la [dernière release](https://github.com/francisdb/vpxtool/releases/latest) (Linux, macOS, Windows) :

```bash
# 1. Extraire le .vpx sous forme de répertoire
vpxtool extract MaTable.vpx

# 2. Éditer MaTable/MaTable.ini avec vos surcharges

# 3. Réassembler le .vpx avec l'ini embarqué
vpxtool assemble MaTable/
```

**🎨 Bonus — réduire la taille des tables (conversion WebP)**

Les tables VPX embarquent souvent des images BMP/PNG sans perte, volumineuses. `vpxtool` peut les convertir en WebP en une passe :

```bash
vpxtool images webp MaTable.vpx
```

Utile quand vous avez des dizaines de tables qui mangent de l'espace disque.

---

## 🏗️ Architecture

```
src/
  main.rs       Entry point, first-run detection, eframe launch
  app/          Main App struct, page routing, wizard & launcher UI
  screens.rs    SDL3 display enumeration + role assignment
  inputs.rs     Input mapping with SDL3 event loop on dedicated thread
  tilt.rs       Tilt/nudge sensitivity configuration
  audio.rs      Audio device detection + routing + test sequences
  assets.rs     Backglass extraction from directb2s files
  config.rs     VPinballX.ini read/write (format-preserving)
  db.rs         SQLite catalog
  updater.rs    Visual Pinball release check, download, install
```

## 🧰 Stack

| Layer | Crate | Role |
|---|---|---|
| 🖼️ UI | `eframe` + `egui` | Immediate mode GUI |
| 🖥️ Display/Input | `sdl3-sys` (build-from-source-static) | Screen enumeration, input capture |
| ⚙️ Config | `ini-preserve` | Read/write VPinballX.ini |
| 🗄️ Database | `rusqlite` (bundled) | Local table catalog |
| 🖼️ Images | `image` + `directb2s` | Backglass thumbnail extraction |
| 🔊 Audio | `symphonia` | OGG/Vorbis decode for SDL3 playback |
| 🌐 HTTP | `ureq` | GitHub API + release download |
| 📦 Archive | `zip` + `flate2` + `tar` | Release extraction |
| 🌍 i18n | `rust-i18n` + `noto-fonts-dl` | 20+ languages with font support |

## 🔧 Visual Pinball fork management

The `vpinball-fork.sh` script manages a personal fork of [vpinball/vpinball](https://github.com/vpinball/vpinball) for building Visual Pinball. It keeps CI workflows set to manual dispatch so builds only run when you decide.

Releases created by this script are automatically detected by PinReady clients, which can download and install the correct build for their platform. 🎉

### Prerequisites

- [gh CLI](https://cli.github.com) installed and authenticated (`gh auth login`)
- `jq` installed (`sudo apt install jq`)
- A fork of `vpinball/vpinball` on your GitHub account

### Workflow

```bash
# 1. Sync fork with upstream + patch CI + trigger builds
./vpinball-fork.sh sync

# 2. Monitor build progress
./vpinball-fork.sh status

# 3. Test the build manually on your pincab

# 4. When validated, create a GitHub Release (clients will auto-detect it)
./vpinball-fork.sh release
```

### Commands

| Command | Action |
|---|---|
| `sync` | Force-reset fork to upstream HEAD, patch workflows to `workflow_dispatch`, trigger `vpinball` + `vpinball-sbc` builds |
| `release` | Wait for both builds to succeed, run `prerelease` workflow to create a GitHub Release, upload SBC artifacts |
| `status` | Show recent workflow runs and latest release info |

## 📄 License

GPL-3.0-or-later
