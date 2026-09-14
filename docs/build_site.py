#!/usr/bin/env python3
"""Generate the GitHub Pages site (English at the root, French under fr/).

Run from anywhere: python3 docs/build_site.py
Pages are plain HTML so search engines index them without JavaScript.
Every statement on the site must be backed by the repository (README, src/,
locales/, CI workflow, release assets): update this file when the app changes.
"""
import html
import json
import re
from pathlib import Path

DOCS = Path(__file__).resolve().parent
ROOT = DOCS.parent
SITE = "https://le-syl21.github.io/PinReady/"
# Google Search Console ownership check (the token belongs to the owner's Google account).
GOOGLE_VERIFICATION = "TqbXre6qrm9jaoj6tFwRRiI2vuQilAZLm6kUJA-etmo"
REPO = "https://github.com/Le-Syl21/PinReady"
RELEASES = REPO + "/releases/latest"
DL = REPO + "/releases/latest/download/"
TREE = REPO + "/tree/main/"
DISCORD = "https://discord.gg/T37DYHmt2j"
VIDEO = "https://www.youtube.com/playlist?list=PLZ838nY4NE902Am7NIOGYbEJaak5pEP71"
VPX_FORK = "https://github.com/Le-Syl21/vpinball"
VPX_UPSTREAM = "https://github.com/vpinball/vpinball"
HEADTRACKING = "https://github.com/Le-Syl21/headtracking"
CONFIGTOOL = "https://configtool.vpuniverse.com/app/home"
VPS_DB = "https://github.com/VirtualPinballSpreadsheet/vps-db"
MEDIA_DB = "https://github.com/superhac/vpinmediadb"
VBS_FORK = "https://github.com/Le-Syl21/vpx-standalone-scripts"
VBS_UPSTREAM = "https://github.com/jsm174/vpx-standalone-scripts"
VPXTOOL = "https://github.com/francisdb/vpxtool"

PAGES = ["index", "download", "wizard", "launcher", "faq"]

# Release asset names come from the CI matrix (`pinready-<label>.<ext>`) and
# do not carry the version, so `releases/latest/download/<asset>` stays valid.
ASSETS = [
    ("linux-x86_64", "tar.gz", {"en": ("Linux", "PC with an Intel or AMD 64-bit processor (x86_64)"),
                                "fr": ("Linux", "PC à processeur Intel ou AMD 64 bits (x86_64)")}),
    ("linux-aarch64", "tar.gz", {"en": ("Linux", "ARM 64-bit processor (aarch64)"),
                                 "fr": ("Linux", "Processeur ARM 64 bits (aarch64)")}),
    ("macos-aarch64", "tar.gz", {"en": ("macOS", "Apple silicon Mac (M-series chip)"),
                                 "fr": ("macOS", "Mac à puce Apple silicon (série M)")}),
    ("macos-x86_64", "tar.gz", {"en": ("macOS", "Intel Mac"),
                                "fr": ("macOS", "Mac à processeur Intel")}),
    ("windows-x86_64", "zip", {"en": ("Windows", "64-bit Windows PC (x86_64)"),
                               "fr": ("Windows", "PC Windows 64 bits (x86_64)")}),
]

UI = {
    "en": {
        "nav": {"index": "Home", "download": "Download", "wizard": "Setup wizard",
                "launcher": "Launcher", "faq": "FAQ"},
        "other": ("fr", "Version française", "FR"),
        "footer_src": "Source code and issues on GitHub", "footer_chat": "Discord", "footer_video": "Video demo",
        "footer_note": "PinReady is free software under the GNU GPL v3 or later. "
                       "Visual Pinball is a separate project by its own authors.",
        "system": "System", "file": "Download",
    },
    "fr": {
        "nav": {"index": "Accueil", "download": "Télécharger", "wizard": "Assistant",
                "launcher": "Lanceur", "faq": "FAQ"},
        "other": ("en", "English version", "GB"),
        "footer_src": "Code source et tickets sur GitHub", "footer_chat": "Discord", "footer_video": "Démo vidéo",
        "footer_note": "PinReady est un logiciel libre sous licence GNU GPL v3 ou ultérieure. "
                       "Visual Pinball est un projet distinct, développé par ses propres auteurs.",
        "system": "Système", "file": "Téléchargement",
    },
}


def version():
    """Current version, read from Cargo.toml (used in structured data only)."""
    m = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(encoding="utf-8"), re.M)
    return m.group(1) if m else None


def href(page, lang, from_lang):
    """Relative link from a page in `from_lang` to `page` in `lang`."""
    if lang == from_lang:
        base = ""
    else:
        base = "../" if from_lang == "fr" else "fr/"
    return (base + ("" if page == "index" else page + ".html")) or "./"


def url(page, lang):
    return SITE + ("fr/" if lang == "fr" else "") + ("" if page == "index" else page + ".html")


def downloads(lang):
    u = UI[lang]
    rows = []
    for label, ext, names in ASSETS:
        name = f"pinready-{label}.{ext}"
        system, detail = names[lang]
        rows.append(f'<tr><td><strong>{system}</strong><br><span class="muted">{detail}</span></td>'
                    f'<td><a class="btn" href="{DL}{name}">{name}</a></td></tr>')
    return (f'<div class="table"><table class="dl"><thead><tr><th>{u["system"]}</th><th>{u["file"]}</th></tr></thead>'
            f'<tbody>{"".join(rows)}</tbody></table></div>')


# Default button layouts applied by each controller profile, mirrored from
# `pinscape_button_defaults` in src/inputs.rs (SDL button numbers start at 0).
# Buttons the profile leaves unmapped are not listed.
PROFILES = [
    ("KL25Z (KL Shield V5.1 / Brain / Rig Master)", [
        (0, "START", "Start"), (1, "EXTRA-B", "ExtraBall"), (2, "COIN1", "Credit1"), (3, "COIN2", "Credit2"),
        (4, "L BALL", "LaunchBall"), (5, "EXIT", "ExitGame"), (7, "L FLIPP", "LeftFlipper + LeftStagedFlipper"),
        (8, "R FLIPP", "RightFlipper + RightStagedFlipper"), (9, "L MAGNA", "LeftMagna"), (10, "R MAGNA", "RightMagna"),
        (11, "FIRE", "Lockbar"), (13, "DOOR", "CoinDoor"), (14, "SERVICE EXIT", "Service1"),
        (15, "SERVICE -", "Service2"), (16, "SERVICE +", "Service3"), (17, "ENTER", "Service4"),
        (19, "VOL-", "VolumeDown"), (20, "VOL+", "VolumeUp")]),
    ("Pinscape Pico (OpenPinballDevice)", [
        (0, "Start", "Start"), (1, "Exit", "ExitGame"), (2, "Extra Ball", "ExtraBall"), (3, "Coin 1", "Credit1"),
        (4, "Coin 2", "Credit2"), (5, "Coin 3", "Credit3"), (6, "Coin 4", "Credit4"), (7, "Launch Ball", "LaunchBall"),
        (8, "Fire", "Lockbar"), (9, "Left Flipper", "LeftFlipper"), (10, "Right Flipper", "RightFlipper"),
        (11, "Upper Left Flipper", "LeftStagedFlipper"), (12, "Upper Right Flipper", "RightStagedFlipper"),
        (13, "MagnaSave Left", "LeftMagna"), (14, "MagnaSave Right", "RightMagna"), (16, "Slam Tilt", "SlamTilt"),
        (17, "Coin Door", "CoinDoor"), (18, "Service Cancel", "Service1"), (19, "Service Down", "Service2"),
        (20, "Service Up", "Service3"), (21, "Service Enter", "Service4"), (22, "Left Nudge", "LeftNudge"),
        (23, "Forward Nudge", "CenterNudge"), (24, "Right Nudge", "RightNudge"), (25, "Volume Up", "VolumeUp"),
        (26, "Volume Down", "VolumeDown")]),
    ("DudesCab (Arnoz)", [
        (0, "Start", "Start"), (1, "ExtraBall", "ExtraBall"), (2, "Coin1", "Credit1"), (3, "Coin2", "Credit2"),
        (4, "LaunchBall", "LaunchBall"), (5, "Return", "ExitGame"), (7, "Flipper Left", "LeftFlipper + LeftStagedFlipper"),
        (8, "Flipper Right", "RightFlipper + RightStagedFlipper"), (9, "Magna Left", "LeftMagna"),
        (10, "Magna Right", "RightMagna"), (12, "Fire", "Lockbar"), (13, "Door", "CoinDoor"),
        (14, "ROM Exit", "Service1"), (15, "ROM -", "Service2"), (16, "ROM +", "Service3"),
        (17, "ROM Enter", "Service4"), (18, "VOL -", "VolumeDown"), (19, "VOL +", "VolumeUp")]),
    ("CSD PinOne", [
        (0, "VPX 1", "RightFlipper"), (1, "VPX 2", "RightMagna"), (2, "VPX 3", "LeftFlipper"), (3, "VPX 4", "LeftMagna"),
        (4, "VPX 5", "ExtraBall"), (5, "VPX 6", "Start"), (6, "VPX 7", "Credit1"), (7, "VPX 8", "ExitGame"),
        (8, "VPX 9", "Lockbar"), (15, "VPX 16", "VolumeUp"), (16, "VPX 17", "VolumeDown"), (17, "VPX 18", "CoinDoor"),
        (18, "VPX 19", "Service1"), (19, "VPX 20", "Service2"), (20, "VPX 21", "Service3"), (21, "VPX 22", "Service4"),
        (23, "VPX 24", "LaunchBall")]),
]


def profile_tables(lang):
    head = (("Button", "Label on the board", "VPX action") if lang == "en"
            else ("Bouton", "Nom sur la carte", "Action VPX"))
    out = []
    for name, rows in PROFILES:
        body = "".join(f"<tr><td>{b}</td><td>{html.escape(l)}</td><td><code>{html.escape(a)}</code></td></tr>"
                       for b, l, a in rows)
        out.append(f'<details><summary>{html.escape(name)}</summary><div class="table"><table>'
                   f'<thead><tr><th>{head[0]}</th><th>{head[1]}</th><th>{head[2]}</th></tr></thead>'
                   f'<tbody>{body}</tbody></table></div></details>')
    return "\n".join(out)


def files_table(lang):
    rows = [
        ("VPinballX.ini", "~/.local/share/VPinballX/10.8/VPinballX.ini",
         "~/Library/Application Support/VPinballX/10.8/VPinballX.ini", r"%APPDATA%\VPinballX\10.8\VPinballX.ini"),
        ("PinReady", "~/.local/share/pinready/", "~/Library/Application Support/pinready/", "%APPDATA%\\pinready\\"),
        ("DOF", "~/.local/share/VPinballX/10.8/directoutputconfig/",
         "~/Library/Application Support/VPinballX/10.8/directoutputconfig/",
         "%APPDATA%\\VPinballX\\10.8\\directoutputconfig\\"),
    ]
    labels = {"en": ["Visual Pinball settings (read and written by PinReady)", "PinReady database and log",
                     "DOF configuration (extract the VPUniverse zip here)"],
              "fr": ["Réglages de Visual Pinball (lus et écrits par PinReady)", "Base de données et journal de PinReady",
                     "Configuration DOF (décompressez ici le zip de VPUniverse)"]}[lang]
    head = ("What", "Linux", "macOS", "Windows") if lang == "en" else ("Quoi", "Linux", "macOS", "Windows")
    body = "".join(f"<tr><td>{labels[i]}</td><td><code>{html.escape(a)}</code></td><td><code>{html.escape(b)}</code></td>"
                   f"<td><code>{html.escape(c)}</code></td></tr>" for i, (_, a, b, c) in enumerate(rows))
    return (f'<div class="table"><table class="paths"><thead><tr>{"".join(f"<th>{h}</th>" for h in head)}</tr></thead>'
            f"<tbody>{body}</tbody></table></div>")


# ---------------------------------------------------------------- FAQ data
# (question, answer HTML) pairs; also emitted as FAQPage structured data.

def faq_items(lang):
    p = lambda name: href(name, lang, lang)  # noqa: E731
    if lang == "en":
        return [
            ("Is PinReady for my cabinet?",
             f"<p>PinReady is built to support the development and adoption of Visual Pinball X 10.8.1. It targets "
             f"testers and early adopters of the new 10.8.1 architecture: integrated plugins, one folder per table, "
             f"SDL3/bgfx engine. If your cabinet runs a stable VPX 10.7.x setup that you rely on, PinReady is probably "
             f"not for you yet.</p>"),
            ("Do I need a real pinball cabinet?",
             f"<p>No. The wizard starts from one screen: pick <strong>1 screen</strong> and the <strong>Desktop</strong> "
             f"display mode for a normal computer, and play with the keyboard or a gamepad. Two to four screens, "
             f"pinball controller boards, accelerometers and surround sound are all optional.</p>"),
            ("Does PinReady install the official Visual Pinball?",
             f'<p>No. The automatic install downloads a recent build of the Visual Pinball X fork tested with PinReady, '
             f'from <a href="{VPX_FORK}">Le-Syl21/vpinball</a>, not the official VPX build. If you prefer your own '
             f'install, choose <strong>I already have Visual Pinball</strong> on the first page and point PinReady to '
             f'the <code>VPinballX_BGFX</code> program.</p>'),
            ("Which pinball controller boards are recognized?",
             f'<p>Pinscape KL25Z (KL Shield V5.1, Brain, Rig Master), Pinscape Pico, DudesCab and CSD PinOne. When one is '
             f'detected, its factory button layout fills the joystick column, and VPX handles the plunger and the '
             f'accelerometer by itself. Any other joystick or gamepad works too: map its buttons one by one. See the '
             f'<a href="{p("wizard")}#inputs">Inputs page of the wizard</a>.</p>'),
            ("An older table does not work under VPX standalone. What can I do?",
             f'<p>Some older tables use VBScript features that only work on Windows. On the <strong>Tables</strong> '
             f'page, turn on the automatic VBS patches: PinReady then places a patched script next to the table when '
             f'<a href="{VBS_FORK}">Le-Syl21/vpx-standalone-scripts</a> has one. It is off by default, because a patch '
             f'can occasionally make a specific table worse. You can also do it by hand: take the <code>.vbs</code> '
             f'matching your table from <a href="{VBS_UPSTREAM}">jsm174/vpx-standalone-scripts</a> and put it next to '
             f'the <code>.vpx</code>, with the same base name.</p>'),
            ("How do I use different settings for one table?",
             f'<p>Put a <code>.ini</code> file next to the <code>.vpx</code>, with the same base name, containing only '
             f'the settings you want to change; everything else comes from the global <code>VPinballX.ini</code>. '
             f'To keep those settings inside the table file itself, <a href="{VPXTOOL}">vpxtool</a> can extract the '
             f'table (<code>vpxtool extract MyTable.vpx</code>) and put it back together with the ini inside '
             f'(<code>vpxtool assemble MyTable/</code>).</p>'),
            ("My tables take a lot of disk space.",
             f'<p>Tables often ship large lossless images. <a href="{VPXTOOL}">vpxtool</a> can convert them to WebP: '
             f'<code>vpxtool images webp MyTable.vpx</code>.</p>'),
            ("My toys, lights and solenoids do nothing in game.",
             f'<p>Visual Pinball ships with every plugin turned off, DOF included. Generate your DOF configuration with '
             f'the VPUniverse ConfigTool, extract it into the <code>directoutputconfig</code> folder, then turn DOF on '
             f'from the <a href="{p("wizard")}#accessories">Accessories page</a>.</p>'),
            ("The table tilts for nothing, or never tilts.",
             f'<p>Check the <strong>accelerometer range</strong> on the <a href="{p("wizard")}#tilt">Tilt / Nudge page</a>. '
             f'It must match the range your board really uses: PinReady reads it from the board when the board '
             f'publishes it, and warns you when the selected value does not match. A wrong range makes every shake '
             f'look too strong or too weak, and no other setting can make up for it. The page also tells you when the '
             f'chosen tilt angle is out of the sensor\'s reach.</p>'),
            ("The sound comes from the wrong speakers.",
             f'<p>Use the tests on the <a href="{p("wizard")}#audio">Audio page</a>. If the rolling ball goes the wrong '
             f'way from front to back, change the output mode; if it goes the wrong way from left to right, check your '
             f'left/right wiring. Jack colours vary between sound cards.</p>'),
            ("Linux: my output board is not found on the Accessories page.",
             f'<p>Raw USB HID access on Linux usually needs a udev rule. The Accessories page shows the rules, can copy '
             f'them, or install them for you after asking for your password.</p>'),
            ("Linux: do I need to run the wizard again after switching between Wayland and X11?",
             f'<p>No. A screen is named differently depending on whether Visual Pinball runs under Wayland or X11, so '
             f'PinReady remembers each screen by its position, size and EDID identity (the ID the monitor reports). '
             f'When the launcher starts, it picks the driver VPX will use and updates the screen names in '
             f'<code>VPinballX.ini</code> to match.</p>'),
            ("How do I start the configuration again?",
             f'<p>Open the wizard from the <strong>Configuration</strong> button of the launcher, or run '
             f'<code>pinready --config</code>. To start from scratch, use <strong>Reset the configuration</strong> in '
             f'the wizard: it forgets every answer and keeps your old <code>VPinballX.ini</code> as '
             f'<code>VPinballX.ini.reset-backup</code>. Your tables are left alone.</p>'),
            ("Visual Pinball crashed. How do I report it?",
             f'<p>When VPX exits abnormally, PinReady shows a report with the command to reproduce the crash, system '
             f'information, the logs and the location of the crash dump. Click <strong>Copy</strong> or '
             f'<strong>Save…</strong> and share it on <a href="{DISCORD}">Discord</a> or in the '
             f'<a href="{REPO}/issues">GitHub issues</a>.</p>'),
            ("A table shows “no image” in the launcher.",
             f'<p>Put a picture at <code>medias/launcher.png</code> in the table folder (.webp, .jpg and .jpeg work too), '
             f'or a <code>.directb2s</code> backglass file. If pictures look out of date after you reorganized your '
             f'tables, click <strong>Rebuild</strong> in the launcher.</p>'),
            ("Where are my settings stored?",
             f'<p>See <a href="{p("download")}#files">file locations</a>. <code>pinready --print-paths</code> prints '
             f'the paths PinReady actually uses.</p>'),
        ]
    return [
        ("PinReady est-il fait pour mon flipper ?",
         f"<p>PinReady est conçu pour accompagner le développement et l'adoption de Visual Pinball X 10.8.1. Il "
         f"s'adresse aux testeurs et aux premiers utilisateurs de la nouvelle architecture 10.8.1 : plugins intégrés, "
         f"un dossier par table, moteur SDL3/bgfx. Si votre flipper tourne sur une installation VPX 10.7.x stable dont "
         f"vous dépendez, PinReady n'est probablement pas encore pour vous.</p>"),
        ("Faut-il un vrai meuble de flipper ?",
         f"<p>Non. L'assistant part d'un seul écran : choisissez <strong>1 écran</strong> et le mode d'affichage "
         f"<strong>Bureau</strong> pour un ordinateur classique, et jouez au clavier ou à la manette. De deux à quatre "
         f"écrans, les cartes de contrôleur de flipper, l'accéléromètre et le son surround sont tous facultatifs.</p>"),
        ("PinReady installe-t-il le Visual Pinball officiel ?",
         f'<p>Non. L\'installation automatique télécharge une version récente du fork de Visual Pinball X validé avec '
         f'PinReady, depuis <a href="{VPX_FORK}">Le-Syl21/vpinball</a>, et non la version officielle de VPX. Si vous '
         f'préférez votre propre installation, choisissez <strong>J\'ai déjà Visual Pinball</strong> sur la première '
         f'page et indiquez le programme <code>VPinballX_BGFX</code>.</p>'),
        ("Quelles cartes de contrôleur de flipper sont reconnues ?",
         f'<p>Pinscape KL25Z (KL Shield V5.1, Brain, Rig Master), Pinscape Pico, DudesCab et CSD PinOne. Quand l\'une '
         f'd\'elles est détectée, sa disposition de boutons d\'usine remplit la colonne Joystick, et VPX gère lui-même '
         f'le lanceur de bille (plunger) et l\'accéléromètre. Tout autre joystick ou manette fonctionne aussi : '
         f'assignez ses boutons un par un. Voir la <a href="{p("wizard")}#inputs">page Inputs de l\'assistant</a>.</p>'),
        ("Une ancienne table ne fonctionne pas sous VPX standalone. Que faire ?",
         f'<p>Certaines anciennes tables utilisent des fonctions VBScript qui n\'existent que sous Windows. Sur la page '
         f'<strong>Tables</strong>, activez les patches VBS automatiques : PinReady place alors un script corrigé à côté '
         f'de la table quand <a href="{VBS_FORK}">Le-Syl21/vpx-standalone-scripts</a> en propose un. L\'option est '
         f'désactivée par défaut, car un patch peut parfois dégrader une table précise. Vous pouvez aussi le faire à la '
         f'main : prenez le <code>.vbs</code> de votre table dans <a href="{VBS_UPSTREAM}">jsm174/vpx-standalone-scripts</a> '
         f'et posez-le à côté du <code>.vpx</code>, avec le même nom de base.</p>'),
        ("Comment régler une table différemment des autres ?",
         f'<p>Posez un fichier <code>.ini</code> à côté du <code>.vpx</code>, avec le même nom de base, qui ne contient '
         f'que les réglages à changer ; tout le reste vient du <code>VPinballX.ini</code> global. Pour garder ces '
         f'réglages dans le fichier de la table, <a href="{VPXTOOL}">vpxtool</a> peut extraire la table '
         f'(<code>vpxtool extract MaTable.vpx</code>) puis la réassembler avec l\'ini à l\'intérieur '
         f'(<code>vpxtool assemble MaTable/</code>).</p>'),
        ("Mes tables prennent beaucoup de place.",
         f'<p>Les tables contiennent souvent de grosses images sans perte. <a href="{VPXTOOL}">vpxtool</a> sait les '
         f'convertir en WebP : <code>vpxtool images webp MaTable.vpx</code>.</p>'),
        ("Mes jouets, lumières et solénoïdes ne réagissent pas en jeu.",
         f'<p>Visual Pinball est livré avec tous ses plugins désactivés, DOF compris. Générez votre configuration DOF '
         f'avec le ConfigTool de VPUniverse, décompressez-la dans le dossier <code>directoutputconfig</code>, puis '
         f'activez DOF depuis la <a href="{p("wizard")}#accessories">page Accessoires</a>.</p>'),
        ("La table tilte pour rien, ou ne tilte jamais.",
         f'<p>Vérifiez la <strong>plage de l\'accéléromètre</strong> sur la <a href="{p("wizard")}#tilt">page Tilt / '
         f'Nudge</a>. Elle doit correspondre à la plage réellement utilisée par votre carte : PinReady la lit sur la '
         f'carte quand celle-ci la publie, et vous prévient si la valeur choisie ne correspond pas. Une mauvaise plage '
         f'fait paraître chaque secousse trop forte ou trop faible, et aucun autre réglage ne peut compenser. La page '
         f'indique aussi quand l\'angle de tilt choisi est hors de portée du capteur.</p>'),
        ("Le son sort des mauvaises enceintes.",
         f'<p>Servez-vous des tests de la <a href="{p("wizard")}#audio">page Audio</a>. Si la bille qui roule va dans le '
         f'mauvais sens d\'avant en arrière, changez le mode de sortie ; si elle va dans le mauvais sens de gauche à '
         f'droite, vérifiez le câblage gauche/droite. Les couleurs des prises varient d\'une carte son à l\'autre.</p>'),
        ("Linux : ma carte de sorties n'est pas trouvée sur la page Accessoires.",
         f'<p>Sous Linux, l\'accès USB HID direct demande en général une règle udev. La page Accessoires affiche ces '
         f'règles, peut les copier, ou les installer pour vous après avoir demandé votre mot de passe.</p>'),
        ("Linux : faut-il relancer l'assistant après être passé de Wayland à X11 ?",
         f'<p>Non. Un même écran porte un nom différent selon que Visual Pinball tourne sous Wayland ou sous X11 ; '
         f'PinReady retient donc chaque écran par sa position, sa taille et son identité EDID (l\'identifiant que '
         f'l\'écran transmet). Au démarrage du lanceur, il choisit le pilote que VPX utilisera et met à jour les noms '
         f'd\'écran dans <code>VPinballX.ini</code> en conséquence.</p>'),
        ("Comment refaire la configuration ?",
         f'<p>Ouvrez l\'assistant avec le bouton <strong>Config</strong> du lanceur, ou lancez '
         f'<code>pinready --config</code>. Pour repartir de zéro, utilisez <strong>Réinitialiser la configuration</strong> '
         f'dans l\'assistant : il oublie toutes les réponses et garde votre ancien <code>VPinballX.ini</code> sous le nom '
         f'<code>VPinballX.ini.reset-backup</code>. Vos tables ne sont pas touchées.</p>'),
        ("Visual Pinball a planté. Comment le signaler ?",
         f'<p>Quand VPX se ferme anormalement, PinReady affiche un rapport avec la commande pour reproduire le plantage, '
         f'les informations système, les journaux et l\'emplacement du fichier de plantage. Cliquez sur '
         f'<strong>Copier</strong> ou <strong>Enregistrer…</strong> et partagez-le sur <a href="{DISCORD}">Discord</a> '
         f'ou dans les <a href="{REPO}/issues">tickets GitHub</a>.</p>'),
        ("Une table affiche « pas d'image » dans le lanceur.",
         f'<p>Déposez une image <code>medias/launcher.png</code> dans le dossier de la table (.webp, .jpg et .jpeg '
         f'fonctionnent aussi), ou un fichier de fronton <code>.directb2s</code>. Si les images semblent périmées après '
         f'une réorganisation de vos tables, cliquez sur <strong>Reconstruire</strong> dans le lanceur.</p>'),
        ("Où sont enregistrés mes réglages ?",
         f'<p>Voir les <a href="{p("download")}#files">emplacements des fichiers</a>. <code>pinready --print-paths</code> '
         f'affiche les chemins réellement utilisés par PinReady.</p>'),
    ]


# ---------------------------------------------------------------- content

def content(page, lang):
    p = lambda name: href(name, lang, lang)  # noqa: E731
    en = lang == "en"

    if page == "index":
        if en:
            return ("PinReady – VPX 10.8.1 standalone configurator and table launcher for Linux, macOS, Windows",
                    "Free, open-source setup wizard and table launcher for Visual Pinball X 10.8.1 standalone: "
                    "cabinet screens, controls, tilt and nudge, audio routing, tables. Linux, macOS and Windows.",
                    f"""
<section class="hero">
<p class="eyebrow">Visual Pinball X 10.8.1 standalone</p>
<h1>Set up your virtual pinball, then play</h1>
<p class="lead">PinReady is a free, open-source setup wizard and table launcher for Visual Pinball X (VPX) 10.8.1
standalone. It configures screens, controls, tilt and nudge, and sound in plain language, then lets you browse and
start your tables from one menu. From a single-screen computer to a full pincab (a pinball cabinet with several
screens), on Linux, macOS and Windows.</p>
<p class="actions"><a class="btn big" href="{p('download')}">Download PinReady</a>
<a class="btn big ghost" href="{VIDEO}">Watch the video demo</a></p>
</section>

<div class="note"><strong>Scope, please read before installing.</strong> PinReady is designed to support the
development and adoption of VPX 10.8.1, not to replace a stable production setup. It targets testers and early
adopters of the new 10.8.1 architecture: integrated plugins, one folder per table, SDL3/bgfx engine. If you run a
stable VPX 10.7.x cabinet, PinReady is probably not for you yet.</div>

<h2>Why PinReady</h2>
<p>The standalone builds of Visual Pinball X have no configuration tool of their own: screens, buttons and speakers
are lines in a text file, <code>VPinballX.ini</code>. PinReady asks the questions instead, detects your hardware and
writes that file for you, keeping the comments it contains. Once the wizard is done, PinReady opens straight on your
table collection.</p>

<div class="cards">
<div class="card"><h3>Setup wizard</h3><p>Eight pages: screens, rendering, inputs, accessories, tilt and nudge,
audio, tables, system. Every option comes with a short explanation.</p><a class="more" href="{p('wizard')}">The wizard →</a></div>
<div class="card"><h3>Table launcher</h3><p>A grid of your tables with their backglass, search, joystick navigation,
and one-click start with a loading progress bar.</p><a class="more" href="{p('launcher')}">The launcher →</a></div>
<div class="card"><h3>Download</h3><p>One program file for Linux, macOS or Windows. Nothing to install, no
dependencies.</p><a class="more" href="{p('download')}">Download and start →</a></div>
<div class="card"><h3>Questions</h3><p>Old tables, per-table settings, DOF, tilt, speakers, Wayland and
X11.</p><a class="more" href="{p('faq')}">FAQ →</a></div>
</div>

<h2>What PinReady takes care of</h2>
<ul class="features">
<li><strong>Visual Pinball itself</strong>: downloads and installs a VPX build for your system, or uses the one you
already have, and offers updates when a new build comes out.</li>
<li><strong>Screens</strong>: detects every display, suggests which one is the playfield, the backglass (the upright
screen at the back), the DMD (the score display) and the topper, and places the game windows.</li>
<li><strong>Controls</strong>: keyboard and joystick for every VPX action, with ready-made layouts for Pinscape KL25Z,
Pinscape Pico, DudesCab and PinOne boards, and a warning when two actions share a key.</li>
<li><strong>Tilt and nudge</strong>: accelerometer sensitivity, with a live view while you shake the cabinet.</li>
<li><strong>Sound</strong>: one sound card for music and voices, one for the mechanical sounds of the playfield, six
speaker layouts including SSF (surround sound feedback), and test sounds to check the wiring.</li>
<li><strong>Tables</strong>: your tables folder, an optional import that gives each table its own folder, optional
script patches for older tables, and backglass pictures downloaded for the launcher.</li>
<li><strong>Accessories</strong>: a step-by-step DOF guide (Direct Output Framework: toys, lights, solenoids) and a
one-click head tracking install.</li>
<li><strong>System</strong>: start at boot, menu shortcuts, and <code>.vpx</code> files opening with Visual Pinball.</li>
</ul>

<h2>Runs on</h2>
<ul>
<li><strong>Linux</strong> (x86_64 and ARM 64-bit), <strong>macOS</strong> (Apple silicon and Intel, macOS 11 or
later) and <strong>Windows</strong> (64-bit).</li>
<li>No system dependencies: SDL3 and SQLite are built into the program.</li>
<li>Interface available in 26 languages, including English and French.</li>
</ul>

<h2>Free software, built in the open</h2>
<p>PinReady is written in Rust with egui and SDL3, and released under the GNU GPL v3 or later. The source code, the
releases and the issue tracker are on <a href="{REPO}">GitHub</a>. Questions, bug reports, beta testing, or just a
chat: join the <a href="{DISCORD}">Discord</a>.</p>
""")
        return ("PinReady – configurer et lancer Visual Pinball X 10.8.1 standalone sous Linux, macOS, Windows",
                "Assistant de configuration et lanceur de tables libre et gratuit pour Visual Pinball X 10.8.1 "
                "standalone : écrans du flipper virtuel, commandes, tilt et secousses, son, tables. Linux, macOS, Windows.",
                f"""
<section class="hero">
<p class="eyebrow">Visual Pinball X 10.8.1 standalone</p>
<h1>Configurez votre flipper virtuel, puis jouez</h1>
<p class="lead">PinReady est un assistant de configuration et un lanceur de tables libre et gratuit pour Visual
Pinball X (VPX) 10.8.1 standalone. Il règle les écrans, les commandes, le tilt et les secousses, et le son avec des
mots simples, puis vous permet de parcourir et de lancer vos tables depuis un seul menu. D'un ordinateur à un seul
écran jusqu'au pincab complet (un meuble de flipper à plusieurs écrans), sous Linux, macOS et Windows.</p>
<p class="actions"><a class="btn big" href="{p('download')}">Télécharger PinReady</a>
<a class="btn big ghost" href="{VIDEO}">Voir la démo vidéo</a></p>
</section>

<div class="note"><strong>Périmètre, à lire avant d'installer.</strong> PinReady est conçu pour accompagner le
développement et l'adoption de VPX 10.8.1, pas pour remplacer une installation stable en production. Il s'adresse aux
testeurs et aux premiers utilisateurs de la nouvelle architecture 10.8.1 : plugins intégrés, un dossier par table,
moteur SDL3/bgfx. Si votre flipper tourne sur une installation VPX 10.7.x stable, PinReady n'est probablement pas
encore pour vous.</div>

<h2>Pourquoi PinReady</h2>
<p>Les versions standalone de Visual Pinball X n'ont pas d'outil de configuration : écrans, boutons et enceintes
sont des lignes dans un fichier texte, <code>VPinballX.ini</code>. PinReady pose les questions à votre place, détecte
votre matériel et écrit ce fichier pour vous, en conservant les commentaires qu'il contient. Une fois l'assistant
terminé, PinReady s'ouvre directement sur votre collection de tables.</p>

<div class="cards">
<div class="card"><h3>Assistant de configuration</h3><p>Huit pages : écrans, rendu, commandes, accessoires, tilt et
secousses, son, tables, système. Chaque option est accompagnée d'une courte explication.</p><a class="more" href="{p('wizard')}">L'assistant →</a></div>
<div class="card"><h3>Lanceur de tables</h3><p>Une grille de vos tables avec leur fronton, la recherche, la navigation
au joystick et le lancement en un clic avec barre de progression.</p><a class="more" href="{p('launcher')}">Le lanceur →</a></div>
<div class="card"><h3>Téléchargement</h3><p>Un seul fichier programme pour Linux, macOS ou Windows. Rien à installer,
aucune dépendance.</p><a class="more" href="{p('download')}">Télécharger et démarrer →</a></div>
<div class="card"><h3>Questions</h3><p>Anciennes tables, réglages par table, DOF, tilt, enceintes, Wayland et
X11.</p><a class="more" href="{p('faq')}">FAQ →</a></div>
</div>

<h2>Ce dont PinReady s'occupe</h2>
<ul class="features">
<li><strong>Visual Pinball lui-même</strong> : télécharge et installe une version de VPX adaptée à votre système, ou
utilise celle que vous avez déjà, et propose les mises à jour quand une nouvelle version sort.</li>
<li><strong>Écrans</strong> : détecte chaque écran, propose lequel sert de plateau de jeu (playfield), de fronton
(backglass, l'écran vertical du fond), d'afficheur de score (DMD) et de topper, et place les fenêtres du jeu.</li>
<li><strong>Commandes</strong> : clavier et joystick pour chaque action de VPX, avec des dispositions toutes prêtes
pour les cartes Pinscape KL25Z, Pinscape Pico, DudesCab et PinOne, et un avertissement quand deux actions partagent
une touche.</li>
<li><strong>Tilt et secousses</strong> : sensibilité de l'accéléromètre, avec une vue en direct pendant que vous
secouez le meuble.</li>
<li><strong>Son</strong> : une carte son pour la musique et les voix, une pour les bruits mécaniques du plateau, six
dispositions d'enceintes dont le SSF (retour sonore surround), et des sons de test pour vérifier le câblage.</li>
<li><strong>Tables</strong> : votre dossier de tables, un import facultatif qui donne à chaque table son propre
dossier, des patches de script facultatifs pour les anciennes tables, et des images de fronton téléchargées pour le
lanceur.</li>
<li><strong>Accessoires</strong> : un guide pas à pas pour DOF (Direct Output Framework : jouets, lumières,
solénoïdes) et l'installation du head tracking en un clic.</li>
<li><strong>Système</strong> : lancement au démarrage, raccourcis dans le menu, et ouverture des fichiers
<code>.vpx</code> avec Visual Pinball.</li>
</ul>

<h2>Fonctionne sous</h2>
<ul>
<li><strong>Linux</strong> (x86_64 et ARM 64 bits), <strong>macOS</strong> (Apple silicon et Intel, macOS 11 ou
plus récent) et <strong>Windows</strong> (64 bits).</li>
<li>Aucune dépendance système : SDL3 et SQLite sont intégrés au programme.</li>
<li>Interface disponible en 26 langues, dont le français et l'anglais.</li>
</ul>

<h2>Un logiciel libre, développé ouvertement</h2>
<p>PinReady est écrit en Rust avec egui et SDL3, et publié sous licence GNU GPL v3 ou ultérieure. Le code source, les
versions publiées et les tickets sont sur <a href="{REPO}">GitHub</a>. Questions, bugs, tests des bêtas ou simple
discussion : rejoignez le <a href="{DISCORD}">Discord</a>.</p>
""")

    if page == "download":
        if en:
            return ("Download PinReady for Linux, macOS and Windows – Visual Pinball X 10.8.1 launcher",
                    "Download PinReady, the free Visual Pinball X 10.8.1 standalone configurator and launcher, for "
                    "Linux (x86_64, ARM64), macOS (Apple silicon, Intel) and Windows. No install, no dependencies.",
                    f"""
<h1>Download PinReady</h1>
<p class="lead">PinReady is a single program file: download the one for your system, extract it and start it.
There is nothing to install. These links always point to the latest release.</p>
{downloads("en")}
<p>Release notes and older versions: <a href="{REPO}/releases">all releases on GitHub</a>.</p>

<h2>Start it</h2>
<ul>
<li><strong>Windows</strong>: extract the zip, then double-click <code>pinready.exe</code>.</li>
<li><strong>Linux</strong>: extract the archive and start the program from its folder:
<pre><code>tar xzf pinready-linux-x86_64.tar.gz
./pinready</code></pre></li>
<li><strong>macOS</strong>: extract the archive and start the program from the Terminal:
<pre><code>tar xzf pinready-macos-aarch64.tar.gz
./pinready</code></pre></li>
</ul>
<p>The Windows program is digitally signed, and the macOS programs are signed and notarized with Apple. The macOS
builds target macOS 11 or later.</p>
<p>The first start opens the <a href="{p('wizard')}">setup wizard</a>; the following ones open the
<a href="{p('launcher')}">table launcher</a>.</p>

<h2>What you need</h2>
<ul>
<li><strong>Visual Pinball X 10.8.1 or later.</strong> PinReady can install it for you, or use a copy you installed
yourself.</li>
<li><strong>A tables folder</strong> using the 10.8.1 layout, one folder per table. PinReady can help you get there:
see <a href="{p('wizard')}#tables">the Tables page of the wizard</a>.</li>
<li><strong>An Internet connection</strong> for the automatic Visual Pinball install and the update checks. It is
optional if you install Visual Pinball yourself.</li>
</ul>
<p>There are no libraries to install: SDL3 and SQLite are built into PinReady.</p>

<h2 id="vpx">Installing Visual Pinball</h2>
<p>The first page of the wizard offers two choices:</p>
<ul>
<li><strong>Install automatically (recommended)</strong> downloads and installs a recent build of the Visual Pinball
X fork tested with PinReady, from <a href="{VPX_FORK}">Le-Syl21/vpinball</a>. This is not the official VPX build. It
picks the build for your system, including builds for Raspberry Pi and RK3588 boards on ARM Linux. You choose the
installation folder.</li>
<li><strong>I already have Visual Pinball</strong> uses a copy you installed yourself: point PinReady to the
<code>VPinballX_BGFX</code> program.</li>
</ul>
<p>The automatic install is validated on Ubuntu 24.04 LTS, under X11, on a 3-screen pincab. The official Visual
Pinball project lives at <a href="{VPX_UPSTREAM}">vpinball/vpinball</a>.</p>

<h2>Updates</h2>
<p>At startup PinReady checks whether a new build of Visual Pinball or a new version of PinReady is out. When one
is, an update button appears in the launcher: one click downloads it and installs it.</p>

<h2 id="files">Where files go</h2>
<p>The folders are found automatically; you do not need to create them.</p>
{files_table("en")}

<h2>Build from source</h2>
<p>PinReady is written in Rust. On Linux (Debian or Ubuntu), install the build tools first:</p>
<pre><code>sudo apt install build-essential cmake pkg-config \\
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \\
  libxkbcommon-dev libssl-dev

cargo build --release</code></pre>
<p>On macOS and Windows, <code>cargo build --release</code> is enough. SDL3 and SQLite are compiled from source
automatically. The program lands in <code>target/release/</code>.</p>
""")
        return ("Télécharger PinReady pour Linux, macOS et Windows – lanceur Visual Pinball X 10.8.1",
                "Téléchargez PinReady, le configurateur et lanceur libre de Visual Pinball X 10.8.1 standalone, pour "
                "Linux (x86_64, ARM64), macOS (Apple silicon, Intel) et Windows. Sans installation, sans dépendance.",
                f"""
<h1>Télécharger PinReady</h1>
<p class="lead">PinReady tient en un seul fichier programme : téléchargez celui de votre système, décompressez-le et
lancez-le. Il n'y a rien à installer. Ces liens mènent toujours à la dernière version.</p>
{downloads("fr")}
<p>Notes de version et anciennes versions : <a href="{REPO}/releases">toutes les versions sur GitHub</a>.</p>

<h2>Le lancer</h2>
<ul>
<li><strong>Windows</strong> : décompressez le zip, puis double-cliquez sur <code>pinready.exe</code>.</li>
<li><strong>Linux</strong> : décompressez l'archive et lancez le programme depuis son dossier :
<pre><code>tar xzf pinready-linux-x86_64.tar.gz
./pinready</code></pre></li>
<li><strong>macOS</strong> : décompressez l'archive et lancez le programme depuis le Terminal :
<pre><code>tar xzf pinready-macos-aarch64.tar.gz
./pinready</code></pre></li>
</ul>
<p>Le programme Windows est signé numériquement, et les programmes macOS sont signés et notariés auprès d'Apple.
Les versions macOS visent macOS 11 ou plus récent.</p>
<p>Le premier lancement ouvre l'<a href="{p('wizard')}">assistant de configuration</a> ; les suivants ouvrent le
<a href="{p('launcher')}">lanceur de tables</a>.</p>

<h2>Ce qu'il faut</h2>
<ul>
<li><strong>Visual Pinball X 10.8.1 ou plus récent.</strong> PinReady peut l'installer pour vous, ou utiliser une
copie que vous avez installée vous-même.</li>
<li><strong>Un dossier de tables</strong> au format 10.8.1, un dossier par table. PinReady peut vous aider à y
arriver : voir <a href="{p('wizard')}#tables">la page Tables de l'assistant</a>.</li>
<li><strong>Une connexion Internet</strong> pour l'installation automatique de Visual Pinball et la recherche de
mises à jour. Elle est facultative si vous installez Visual Pinball vous-même.</li>
</ul>
<p>Aucune bibliothèque à installer : SDL3 et SQLite sont intégrés à PinReady.</p>

<h2 id="vpx">Installer Visual Pinball</h2>
<p>La première page de l'assistant propose deux choix :</p>
<ul>
<li><strong>Installer automatiquement (recommandé)</strong> télécharge et installe une version récente du fork de
Visual Pinball X validé avec PinReady, depuis <a href="{VPX_FORK}">Le-Syl21/vpinball</a>. Ce n'est pas la version
officielle de VPX. La version adaptée à votre système est choisie automatiquement, y compris pour les cartes
Raspberry Pi et RK3588 sous Linux ARM. Vous choisissez le dossier d'installation.</li>
<li><strong>J'ai déjà Visual Pinball</strong> utilise une copie installée par vos soins : indiquez à PinReady le
programme <code>VPinballX_BGFX</code>.</li>
</ul>
<p>L'installation automatique est validée sous Ubuntu 24.04 LTS, en X11, sur un pincab à 3 écrans. Le projet
officiel Visual Pinball se trouve sur <a href="{VPX_UPSTREAM}">vpinball/vpinball</a>.</p>

<h2>Mises à jour</h2>
<p>Au démarrage, PinReady vérifie si une nouvelle version de Visual Pinball ou de PinReady est sortie. Si c'est le
cas, un bouton de mise à jour apparaît dans le lanceur : un clic la télécharge et l'installe.</p>

<h2 id="files">Emplacement des fichiers</h2>
<p>Les dossiers sont trouvés automatiquement ; inutile de les créer vous-même.</p>
{files_table("fr")}

<h2>Compiler depuis les sources</h2>
<p>PinReady est écrit en Rust. Sous Linux (Debian ou Ubuntu), installez d'abord les outils de compilation :</p>
<pre><code>sudo apt install build-essential cmake pkg-config \\
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \\
  libxkbcommon-dev libssl-dev

cargo build --release</code></pre>
<p>Sous macOS et Windows, <code>cargo build --release</code> suffit. SDL3 et SQLite sont compilés depuis les sources
automatiquement. Le programme se trouve ensuite dans <code>target/release/</code>.</p>
""")

    if page == "wizard":
        toc_en = [("screens", "Screens"), ("rendering", "Rendering"), ("inputs", "Inputs"),
                  ("accessories", "Accessories"), ("tilt", "Tilt / Nudge"), ("audio", "Audio"),
                  ("tables", "Tables"), ("system", "System")]
        toc_fr = [("screens", "Écrans"), ("rendering", "Rendu"), ("inputs", "Inputs"),
                  ("accessories", "Accessoires"), ("tilt", "Tilt / Nudge"), ("audio", "Audio"),
                  ("tables", "Tables"), ("system", "Système")]
        toc = "".join(f'<li><a href="#{a}">{n}</a></li>' for a, n in (toc_en if en else toc_fr))
        if en:
            modes = [("2 ch — Front stereo", "Plain stereo, both speakers in front of the cabinet."),
                     ("2 ch — Rear stereo (lockbar)", "Plain stereo, both speakers near the lockbar, close to the player."),
                     ("5.1 — Rear at lockbar", "Adds front-to-back positioning; rear channels at the lockbar. Needs a 6-channel sound card."),
                     ("5.1 — Front at lockbar", "Same, with the front channels at the lockbar. Pick the one matching your wiring."),
                     ("SSF — Side &amp; rear at lockbar (Legacy)", "Surround Sound Feedback with the older mixing."),
                     ("SSF — Side &amp; rear at lockbar (New)", "Surround Sound Feedback with the newer mixing, recommended.")]
            wiring = [("Green (Front)", "FL / FR", "Backglass speakers (music) or 2.1 system"),
                      ("Black (Rear)", "BL / BR", "Playfield top exciters (backglass side)"),
                      ("Grey (Side)", "SL / SR", "Playfield bottom exciters (lockbar / player side)"),
                      ("Orange (Center/Sub)", "FC / LFE", "Subwoofer or bass shaker (optional)")]
        else:
            modes = [("2 canaux — Stéréo avant", "Stéréo simple, les deux enceintes à l'avant du meuble."),
                     ("2 canaux — Stéréo arrière (barre avant)", "Stéréo simple, les deux enceintes près de la barre avant, côté joueur."),
                     ("5.1 — Arrière à la barre avant", "Ajoute la position avant/arrière ; canaux arrière à la barre avant. Demande une carte son 6 canaux."),
                     ("5.1 — Avant à la barre avant", "Idem, avec les canaux avant à la barre avant. Prenez celui qui correspond à votre câblage."),
                     ("SSF — Latéral &amp; arrière (Historique)", "Surround Sound Feedback avec l'ancien mixage."),
                     ("SSF — Latéral &amp; arrière (Nouveau)", "Surround Sound Feedback avec le mixage récent, recommandé.")]
            wiring = [("Vert (Front)", "FL / FR", "Enceintes du fronton (musique) ou système 2.1"),
                      ("Noir (Rear)", "BL / BR", "Transducteurs du haut du plateau (côté fronton)"),
                      ("Gris (Side)", "SL / SR", "Transducteurs du bas du plateau (côté barre avant, joueur)"),
                      ("Orange (Center/Sub)", "FC / LFE", "Caisson de basses ou bass shaker (facultatif)")]
        modes_rows = "".join(f"<tr><td>{m}</td><td>{d}</td></tr>" for m, d in modes)
        wiring_rows = "".join(f"<tr><td>{a}</td><td>{b}</td><td>{c}</td></tr>" for a, b, c in wiring)
        audiomap = (f'<a href="{TREE}audiomapping_linux">Linux</a>, <a href="{TREE}audiomapping_windows">Windows</a>, '
                    f'<a href="{TREE}audiomapping_mac">macOS</a>')
        if en:
            return ("VPX standalone configuration wizard: screens, controls, tilt, audio – PinReady",
                    "How PinReady configures Visual Pinball X 10.8.1 standalone step by step: pincab screens, "
                    "keyboard and Pinscape/DudesCab/PinOne controls, tilt and nudge, SSF audio, DOF, tables folder.",
                    f"""
<h1>The setup wizard</h1>
<p class="lead">The first time you start PinReady, a wizard walks you through eight pages. Each option has a small
info marker: hover it, or click it on a cabinet, for a plain-language explanation.</p>
<p>Your answers are written into <code>VPinballX.ini</code>, the settings file of Visual Pinball, and the comments
in that file are kept. If the file does not exist yet, PinReady first has Visual Pinball create its complete default
file, then adds your choices to it. You can reopen the wizard at any time from the <strong>Configuration</strong>
button of the launcher or with <code>pinready --config</code>: every field shows your current settings. Most settings
can also be changed while playing, from the Visual Pinball menu on <kbd>F12</kbd>.</p>
<nav class="toc" aria-label="Wizard pages"><ol>{toc}</ol></nav>
<p><strong>Reset the configuration</strong> throws away every answer, sets the global <code>VPinballX.ini</code>
aside as <code>VPinballX.ini.reset-backup</code> and starts again at page one. Your tables are left alone.</p>

<h2 id="screens">1. Screens</h2>
<p>The first page shows the detected system (for example <em>Ubuntu 24.04 LTS · X11 · GNOME</em>) so that bug reports
always carry their context, lets you pick the interface language, and <a href="{p('download')}#vpx">installs Visual
Pinball</a> if needed.</p>
<ul>
<li><strong>Number of screens</strong>: 1 for a desktop computer, 3 or 4 for a full pincab (playfield, backglass,
DMD and an optional topper).</li>
<li><strong>Roles</strong>: each display is listed with its name, resolution, refresh rate (Hz) and physical size.
PinReady suggests a role from the screen sizes: the largest becomes the playfield, then the backglass, the DMD and the
topper. Change any role from its list, or set a screen to <em>Unused</em>.</li>
<li><strong>Display mode</strong>: <em>Desktop</em> (windowed, for a normal computer), <em>Cabinet</em> (the playfield
drawn for a screen lying flat, backglass and DMD on their own screens) or <em>Full Single Screen</em> (playfield and
backglass on one tall screen). It switches to Cabinet once you have 2 screens or more, and you can still change it.</li>
<li><strong>External DMD device</strong> (ZeDMD, PinDMD…): tick it if you have a physical score display; Visual Pinball
detects it when a table starts.</li>
<li><strong>Disable touch screen</strong>: turns off the touch overlay of VPX, useful when the playfield is not a touch
screen or when you control the cabinet remotely.</li>
<li><strong>Cabinet dimensions</strong>: lockbar width and height, playfield slope, your height and where you stand.
Drag the handles on the drawing or type the values; they give the 3D view the right perspective.</li>
</ul>

<h2 id="rendering">2. Rendering</h2>
<p>Graphics settings for the playfield. The advice on the page: start with the defaults, then adjust to your
machine.</p>
<ul>
<li><strong>Synchronization</strong>: <em>No sync</em>, <em>Vertical sync</em>, <em>Adaptive sync</em> or <em>Frame
pacing</em>, the Visual Pinball default. Each choice explains its trade-off between smoothness and input delay.</li>
<li><strong>FPS limit</strong>: set to the refresh rate of your playfield screen, so no frame is drawn faster than the
screen can show it.</li>
<li><strong>Image quality</strong>: supersampling, MSAA, post-processing anti-aliasing (FXAA and others), sharpening,
reflections and maximum texture size.</li>
<li><strong>Show FPS</strong> (on by default) displays a frame counter in game, handy to check your settings right
after setup.</li>
<li><strong>Round ball</strong> (on by default) keeps the ball from looking slightly egg-shaped when it moves fast.
Purely cosmetic.</li>
</ul>

<h2 id="inputs">3. Inputs</h2>
<p>Every Visual Pinball action has two columns, <strong>Keyboard</strong> and <strong>Joystick</strong>, and both
work side by side. Click <strong>Map</strong> and press a key or a button: a key fills the keyboard column, a joystick
button the joystick column. <strong>Auto-map all inputs</strong> walks through every action in one pass. Essential
actions (flippers, MagnaSave, start, credit, launch ball…) are shown first; advanced ones are one click away. PinReady
warns you when a key is already used by another action.</p>
<p><strong>Pinball controller boards.</strong> When a Pinscape KL25Z, Pinscape Pico, DudesCab or PinOne board is
detected, PinReady picks the matching profile and fills the joystick column with the board's factory button layout;
Visual Pinball then handles the plunger and the accelerometer nudge by itself. Choose <em>None</em> to map every button
yourself. A detected gamepad can also be used for the flippers, plunger and nudge.</p>
{profile_tables("en")}
<p class="muted">Button numbers are the ones SDL reports, counted from 0. Buttons a profile leaves unmapped are not
listed.</p>

<h2 id="accessories">4. Accessories</h2>
<h3>DOF effects (lights and toys)</h3>
<p>VPX 10.8.1 standalone includes DOF (Direct Output Framework), which drives the solenoids, lights, shaker motor,
knocker or beacon of a cabinet. It only needs a configuration describing your hardware. The page walks you through
it:</p>
<ol>
<li>Create a free account on the <a href="{CONFIGTOOL}">VPUniverse ConfigTool</a>.</li>
<li>Create a table matching your cabinet, add its output system (Pinscape, LedWiz, PinOne, DudesCab, PacLed,
TeensyStripController…) and attach your devices to the outputs.</li>
<li>Click <em>Generate</em>, download the zip and extract it into the target folder: the <strong>Open folder in file manager</strong>
button takes you there.</li>
<li>Turn DOF on: Visual Pinball ships with every plugin disabled, so nothing fires in game until you do.</li>
</ol>
<p>PinReady can also look for your output board over USB and tell you which controller type to pick in the
ConfigTool. On Linux it shows, copies or installs the udev rules that give access to the board. An output tester that
pulses each output in turn, with a built-in safety stop, currently runs on a simulator only.</p>
<h3>Head tracking</h3>
<p>For cabinets with a Kinect or a camera on the backglass or topper: the perspective of the table follows your head.
One click downloads the <a href="{HEADTRACKING}">head tracking</a> plugin and its demo, enables the plugin, switches
Visual Pinball to the <em>Window</em> view mode it needs, and opens the demo so you can install the camera drivers and
check that you are tracked. Not available on Intel Macs.</p>

<h2 id="tilt">5. Tilt / Nudge</h2>
<p>Settings for the accelerometer that detects when you shake (nudge) the cabinet.</p>
<ul>
<li><strong>Nudge sensor type</strong>: <em>Game Controller</em> for a gamepad stick, <em>Intent Sensor</em> for
cabinets with an accelerometer board such as Pinscape or DudesCab (it ignores small vibrations and turns a real shove
into a nudge), <em>Cabinet Sensor</em> for a fast, clean sensor whose readings are applied as measured.</li>
<li><strong>Strength</strong>: how hard a shake pushes the ball. In practice, the one setting worth tuning.</li>
<li><strong>Dead zone</strong>: tiny readings below this level are ignored, so vibrations and sensor noise do not move
the ball.</li>
<li><strong>Accelerometer range</strong>: a fact about your board, not a preference. PinReady reads it from the board
when the board publishes it, and warns you if the selected value does not match.</li>
<li><strong>TILT threshold</strong>: how hard you can nudge before the game calls a tilt.</li>
</ul>
<p>Shake the cabinet and watch the live view: a dot shows each shake against rings for the dead zone, the nudge
threshold and the tilt; crossing the red ring means TILT. PinReady also keeps the measured peak, tells you when the
tilt angle is out of the sensor's reach, and asks you to check which way the board is mounted, since a board turned
the wrong way swaps or mirrors the nudge directions.</p>

<h2 id="audio">6. Audio</h2>
<p>Visual Pinball plays sound on two devices, and PinReady lists your sound cards for each:</p>
<ul>
<li><strong>Backglass</strong>: music, voices and general game sounds.</li>
<li><strong>Playfield</strong>: the mechanical sounds (flippers, bumpers, rolling ball). For surround or SSF, this must
be the multi-channel output.</li>
</ul>
<p><strong>Playfield output mode.</strong> PinReady preselects a mode from the number of channels it detects, and
shows a diagram of the speaker layout with the wiring it requires.</p>
<div class="table"><table><thead><tr><th>Mode</th><th>For</th></tr></thead><tbody>{modes_rows}</tbody></table></div>
<p>Wiring for a 7.1 sound card, the recommended SSF setup (jack colours can vary between cards):</p>
<div class="table"><table><thead><tr><th>Jack</th><th>Channels</th><th>Cabinet connection</th></tr></thead><tbody>{wiring_rows}</tbody></table></div>
<p><strong>Volumes and tests.</strong> Set the music and playfield volumes, then check your wiring: test 1 loops music
on the backglass with a left/right balance; test 2 plays a sound on each playfield speaker in turn, and sweeps a
rolling ball from the top to the bottom and from left to right. If the ball moves the wrong way from front to back,
change the output mode; from left to right, check the wiring. A sound card with only 3 jacks (green, pink, blue) can
often be switched to 5.1 or 7.1: see the guides for {audiomap}.</p>

<h2 id="tables">7. Tables</h2>
<p>Choose the folder that holds your tables, one subfolder per table: the layout used by Visual Pinball 10.8.1.</p>
<pre><code>Tables/
└── Table Name/
    ├── Table Name.vpx            table file (required)
    ├── Table Name.directb2s      backglass (optional)
    ├── Table Name.ini            per-table settings (optional)
    ├── Table Name.vbs            script patch (optional)
    ├── medias/launcher.png       launcher picture (optional)
    ├── pinmame/roms/&lt;rom&gt;.zip    ROM (required for ROM tables)
    ├── altsound/&lt;rom&gt;/          alt-sound pack (optional)
    ├── pupvideos/&lt;rom&gt;/         PinUP Player videos (optional)
    ├── serum/&lt;rom&gt;/&lt;rom&gt;.crz    Serum colorization (optional)
    └── vni/&lt;rom&gt;/&lt;rom&gt;.vni      VNI colorization, with .pal (optional)</code></pre>
<p>DMD colorization in Serum (<code>.crz</code>) and VNI (<code>.vni</code> + <code>.pal</code>) formats is supported;
proprietary formats such as <code>.pac</code> are not.</p>
<ul>
<li><strong>Bring your collection into the new layout.</strong> Point PinReady at one folder (a table collection, an
old VPX install, even a whole drive). It indexes everything inside, then gives each table its own folder with the ROM,
backglass, PUP pack, alt-sound, colorization and music that belong to it. If your tables already have one folder each,
they stay where they are and missing files are added. Run a <em>dry run</em> first to preview, then choose to copy
(safe) or move the files.</li>
<li><strong>Visual Pinball Standalone compatibility</strong> (off by default): PinReady recognizes the script built
into each table and, when <a href="{VBS_FORK}">Le-Syl21/vpx-standalone-scripts</a> has a patch for it, places the
patched <code>.vbs</code> next to the <code>.vpx</code>. A <code>.vbs</code> of your own is kept as
<code>&lt;table&gt;.pre_standalone.vbs</code>.</li>
<li><strong>Catalog enrichment</strong> (on by default): matches each table against the
<a href="{VPS_DB}">Virtual Pinball Spreadsheet</a> and downloads a backglass picture and an audio jingle from
<a href="{MEDIA_DB}">VPinMediaDB</a> into the table's <code>medias</code> folder. The first sync downloads about 7 MB of
catalog data, plus a few MB per matched table. Turn it off on an offline cabinet.</li>
</ul>

<h2 id="system">8. System</h2>
<p>Optional conveniences, best suited to a dedicated cabinet:</p>
<ul>
<li><strong>Launch PinReady automatically at startup.</strong></li>
<li><strong>Menu shortcuts and <code>.vpx</code> file association</strong>: adds PinReady and Visual Pinball to the
application menu (GNOME, KDE or XFCE on Linux, Launchpad on macOS, Start Menu on Windows) and opens <code>.vpx</code>
files with Visual Pinball. Untick it and finish the wizard again to remove it.</li>
<li><strong>Desktop shortcut</strong>, and on GNOME, <strong>pin to the dock</strong>.</li>
<li><strong>Self-hosted mirror</strong>: fetch the script patches and the media database from your own server instead
of GitHub.</li>
</ul>
""")
        return ("Configurer un flipper virtuel VPX standalone : écrans, commandes, tilt, son – PinReady",
                "Comment PinReady configure Visual Pinball X 10.8.1 standalone pas à pas : écrans du pincab, clavier et "
                "cartes Pinscape/DudesCab/PinOne, tilt et secousses, son SSF, DOF, dossier des tables.",
                f"""
<h1>L'assistant de configuration</h1>
<p class="lead">Au premier lancement de PinReady, un assistant vous guide sur huit pages. Chaque option a un petit
repère d'information : survolez-le, ou cliquez dessus sur un flipper, pour lire une explication en mots simples.</p>
<p>Vos réponses sont écrites dans <code>VPinballX.ini</code>, le fichier de réglages de Visual Pinball, en conservant
les commentaires de ce fichier. S'il n'existe pas encore, PinReady fait d'abord créer par Visual Pinball son fichier
complet par défaut, puis y ajoute vos choix. Vous pouvez rouvrir l'assistant à tout moment avec le bouton
<strong>Config</strong> du lanceur ou avec <code>pinready --config</code> : chaque champ affiche vos réglages actuels.
La plupart des réglages se changent aussi en jeu, depuis le menu de Visual Pinball sur la touche <kbd>F12</kbd>.</p>
<nav class="toc" aria-label="Pages de l'assistant"><ol>{toc}</ol></nav>
<p><strong>Réinitialiser la configuration</strong> efface toutes les réponses, met de côté le
<code>VPinballX.ini</code> global sous le nom <code>VPinballX.ini.reset-backup</code> et repart de la première page.
Vos tables ne sont pas touchées.</p>

<h2 id="screens">1. Écrans</h2>
<p>La première page affiche le système détecté (par exemple <em>Ubuntu 24.04 LTS · X11 · GNOME</em>) pour que les
signalements de bugs portent leur contexte, permet de choisir la langue de l'interface, et
<a href="{p('download')}#vpx">installe Visual Pinball</a> si besoin.</p>
<ul>
<li><strong>Nombre d'écrans</strong> : 1 pour un ordinateur de bureau, 3 ou 4 pour un pincab complet (plateau,
fronton, DMD et un topper facultatif).</li>
<li><strong>Rôles</strong> : chaque écran est présenté avec son nom, sa résolution, sa fréquence (Hz) et sa taille
physique. PinReady propose un rôle d'après la taille des écrans : le plus grand devient le plateau, puis viennent le
fronton, le DMD et le topper. Changez n'importe quel rôle dans sa liste, ou marquez un écran comme inutilisé.</li>
<li><strong>Mode d'affichage</strong> : <em>Bureau</em> (en fenêtre, pour un ordinateur classique), <em>Cabinet</em>
(le plateau dessiné pour un écran posé à plat, fronton et DMD sur leurs propres écrans) ou <em>Plein écran unique</em>
(plateau et fronton réunis sur un seul écran vertical). Le mode passe à Cabinet dès 2 écrans, et vous pouvez toujours
le changer.</li>
<li><strong>DMD externe</strong> (ZeDMD, PinDMD…) : cochez si vous avez un afficheur de score physique ; Visual
Pinball le détecte au lancement d'une table.</li>
<li><strong>Désactiver l'écran tactile</strong> : coupe la surcouche tactile de VPX, utile quand le plateau n'est pas
tactile ou quand vous pilotez le flipper à distance.</li>
<li><strong>Dimensions du meuble</strong> : largeur et hauteur de la barre avant (lockbar), inclinaison du plateau,
votre taille et votre position. Déplacez les poignées sur le schéma ou tapez les valeurs ; elles donnent à la vue 3D
la bonne perspective.</li>
</ul>

<h2 id="rendering">2. Rendu</h2>
<p>Réglages graphiques du plateau. Le conseil de la page : partez des valeurs par défaut, puis ajustez selon votre
machine.</p>
<ul>
<li><strong>Synchronisation</strong> : <em>Aucune synchro</em>, <em>Synchro verticale</em>, <em>Synchro
adaptative</em> ou <em>Cadencement (Frame Pacing)</em>, le choix par défaut de Visual Pinball. Chaque choix explique
son compromis entre fluidité et délai de réaction.</li>
<li><strong>Limite d'images par seconde</strong> : calée sur la fréquence de l'écran du plateau, pour ne jamais
dessiner plus d'images que l'écran ne peut en afficher.</li>
<li><strong>Qualité d'image</strong> : suréchantillonnage, MSAA, anticrénelage en post-traitement (FXAA et autres),
netteté, reflets et taille maximale des textures.</li>
<li><strong>Afficher les FPS</strong> (activé par défaut) montre un compteur d'images par seconde en jeu, pratique pour
vérifier vos réglages juste après la configuration.</li>
<li><strong>Bille ronde</strong> (activé par défaut) évite que la bille paraisse légèrement ovale quand elle va vite.
Purement esthétique.</li>
</ul>

<h2 id="inputs">3. Inputs (commandes)</h2>
<p>Chaque action de Visual Pinball a deux colonnes, <strong>Clavier</strong> et <strong>Joystick</strong>, qui
fonctionnent côte à côte. Cliquez sur <strong>Mapper</strong> et appuyez sur une touche ou un bouton : une touche
remplit la colonne Clavier, un bouton de joystick la colonne Joystick. <strong>Auto-assigner toutes les entrées</strong>
passe toutes les actions en revue d'un coup. Les actions essentielles (batteurs, MagnaSave, départ, crédit, lancement
de bille…) viennent en premier ; les actions avancées sont à un clic. PinReady vous prévient quand une touche est déjà
utilisée par une autre action.</p>
<p><strong>Cartes de contrôleur de flipper.</strong> Quand une carte Pinscape KL25Z, Pinscape Pico, DudesCab ou
PinOne est détectée, PinReady choisit le profil correspondant et remplit la colonne Joystick avec la disposition de
boutons d'usine de la carte ; Visual Pinball gère alors lui-même le lanceur de bille (plunger) et les secousses lues
par l'accéléromètre. Choisissez <em>Aucune</em> pour assigner chaque bouton vous-même. Une manette détectée peut aussi
servir pour les batteurs, le lanceur et les secousses.</p>
{profile_tables("fr")}
<p class="muted">Les numéros de bouton sont ceux que rapporte SDL, comptés à partir de 0. Les boutons laissés libres
par un profil ne sont pas listés.</p>

<h2 id="accessories">4. Accessoires</h2>
<h3>Effets DOF (lumières et jouets)</h3>
<p>VPX 10.8.1 standalone intègre DOF (Direct Output Framework), qui pilote les solénoïdes, lumières, moteur de
vibration (shaker), knocker ou gyrophare d'un meuble. Il lui faut seulement une configuration qui décrit votre
matériel. La page vous guide :</p>
<ol>
<li>Créez un compte gratuit sur le <a href="{CONFIGTOOL}">ConfigTool de VPUniverse</a>.</li>
<li>Créez une table correspondant à votre meuble, ajoutez son système de sorties (Pinscape, LedWiz, PinOne, DudesCab,
PacLed, TeensyStripController…) et associez vos périphériques aux sorties.</li>
<li>Cliquez sur <em>Generate</em>, téléchargez le zip et décompressez-le dans le dossier cible : le bouton
<strong>Ouvrir le dossier dans l'explorateur</strong> vous y emmène.</li>
<li>Activez DOF : Visual Pinball est livré avec tous ses plugins désactivés, rien ne se déclenche en jeu tant que vous
ne l'avez pas fait.</li>
</ol>
<p>PinReady peut aussi chercher votre carte de sorties en USB et vous dire quel type de contrôleur choisir dans le
ConfigTool. Sous Linux, il affiche, copie ou installe les règles udev qui donnent accès à la carte. Un testeur de
sorties, qui active chaque sortie à tour de rôle avec un arrêt de sécurité intégré, ne fonctionne pour l'instant
qu'avec un simulateur.</p>
<h3>Head tracking (suivi de la tête)</h3>
<p>Pour les meubles équipés d'une Kinect ou d'une caméra sur le fronton ou le topper : la perspective de la table suit
votre tête. Un clic télécharge le plugin <a href="{HEADTRACKING}">head tracking</a> et sa démo, active le plugin, passe
Visual Pinball dans le mode d'affichage <em>Window</em> dont il a besoin, et ouvre la démo pour installer les pilotes
de la caméra et vérifier que vous êtes bien suivi. Non disponible sur les Mac Intel.</p>

<h2 id="tilt">5. Tilt / Nudge (secousses)</h2>
<p>Réglages de l'accéléromètre qui détecte quand vous secouez le meuble (le « nudge »).</p>
<ul>
<li><strong>Type de capteur</strong> : <em>Manette de jeu</em> pour le stick d'une manette, <em>Capteur
d'intention</em> pour les meubles équipés d'une carte à accéléromètre comme Pinscape ou DudesCab (il ignore les petites
vibrations et transforme une vraie poussée en secousse), <em>Capteur de cabinet</em> pour un capteur rapide et propre
dont les mesures sont appliquées telles quelles.</li>
<li><strong>Intensité</strong> : la force avec laquelle une secousse pousse la bille. En pratique, le seul réglage qui
mérite d'être ajusté.</li>
<li><strong>Zone morte</strong> : les toutes petites mesures sous ce seuil sont ignorées, pour que les vibrations et le
bruit du capteur ne fassent pas bouger la bille.</li>
<li><strong>Plage de l'accéléromètre</strong> : une caractéristique de votre carte, pas une préférence. PinReady la lit
sur la carte quand celle-ci la publie, et vous prévient si la valeur choisie ne correspond pas.</li>
<li><strong>Seuil de TILT</strong> : jusqu'où vous pouvez secouer avant que le jeu ne déclare un tilt.</li>
</ul>
<p>Secouez le meuble et regardez la vue en direct : un point montre chaque secousse face à des anneaux pour la zone
morte, le seuil de secousse et le tilt ; franchir l'anneau rouge, c'est TILT. PinReady garde aussi le pic mesuré, vous
signale quand l'angle de tilt est hors de portée du capteur, et vous demande de vérifier le sens de montage de la
carte, car une carte tournée dans le mauvais sens inverse ou échange les directions des secousses.</p>

<h2 id="audio">6. Audio</h2>
<p>Visual Pinball joue le son sur deux périphériques, et PinReady liste vos cartes son pour chacun :</p>
<ul>
<li><strong>Backglass (fronton)</strong> : musique, voix et sons généraux du jeu.</li>
<li><strong>Playfield (plateau)</strong> : les bruits mécaniques (batteurs, bumpers, bille qui roule). Pour le surround
ou le SSF, ce doit être la sortie multicanal.</li>
</ul>
<p><strong>Mode de sortie du plateau.</strong> PinReady présélectionne un mode selon le nombre de canaux détectés, et
affiche un schéma de la disposition des enceintes avec le câblage nécessaire.</p>
<div class="table"><table><thead><tr><th>Mode</th><th>Pour</th></tr></thead><tbody>{modes_rows}</tbody></table></div>
<p>Câblage pour une carte son 7.1, la configuration SSF recommandée (les couleurs des prises peuvent varier selon la
carte) :</p>
<div class="table"><table><thead><tr><th>Prise</th><th>Canaux</th><th>Branchement dans le meuble</th></tr></thead><tbody>{wiring_rows}</tbody></table></div>
<p><strong>Volumes et tests.</strong> Réglez le volume de la musique et celui du plateau, puis vérifiez le câblage :
le test 1 joue une musique en boucle sur le fronton avec une balance gauche/droite ; le test 2 joue un son sur chaque
enceinte du plateau à tour de rôle, puis fait rouler une bille du haut vers le bas et de gauche à droite. Si la bille
va dans le mauvais sens d'avant en arrière, changez le mode de sortie ; de gauche à droite, vérifiez le câblage. Une
carte son à 3 prises seulement (verte, rose, bleue) peut souvent passer en 5.1 ou 7.1 : voir les guides pour
{audiomap} (en anglais et en français).</p>

<h2 id="tables">7. Tables</h2>
<p>Choisissez le dossier qui contient vos tables, un sous-dossier par table : le format utilisé par Visual Pinball
10.8.1.</p>
<pre><code>Tables/
└── Nom de la table/
    ├── Nom de la table.vpx          fichier de la table (obligatoire)
    ├── Nom de la table.directb2s    fronton (facultatif)
    ├── Nom de la table.ini          réglages propres à la table (facultatif)
    ├── Nom de la table.vbs          patch de script (facultatif)
    ├── medias/launcher.png          image pour le lanceur (facultatif)
    ├── pinmame/roms/&lt;rom&gt;.zip       ROM (obligatoire pour les tables à ROM)
    ├── altsound/&lt;rom&gt;/             pack alt-sound (facultatif)
    ├── pupvideos/&lt;rom&gt;/            vidéos PinUP Player (facultatif)
    ├── serum/&lt;rom&gt;/&lt;rom&gt;.crz       colorisation Serum (facultatif)
    └── vni/&lt;rom&gt;/&lt;rom&gt;.vni         colorisation VNI, avec .pal (facultatif)</code></pre>
<p>La colorisation du DMD aux formats Serum (<code>.crz</code>) et VNI (<code>.vni</code> + <code>.pal</code>) est
prise en charge ; les formats propriétaires comme <code>.pac</code> ne le sont pas.</p>
<ul>
<li><strong>Ranger votre collection au nouveau format.</strong> Indiquez à PinReady un seul dossier (une collection de
tables, une ancienne installation VPX, voire un disque entier). Il indexe tout ce qu'il contient, puis donne à chaque
table son propre dossier avec la ROM, le fronton, le PUP pack, l'alt-sound, la colorisation et la musique qui lui
reviennent. Si vos tables ont déjà chacune leur dossier, elles restent en place et les fichiers manquants sont ajoutés.
Lancez d'abord un <em>aperçu</em>, puis choisissez de copier (sans risque) ou de déplacer les fichiers.</li>
<li><strong>Compatibilité Visual Pinball Standalone</strong> (désactivée par défaut) : PinReady reconnaît le script
intégré à chaque table et, quand <a href="{VBS_FORK}">Le-Syl21/vpx-standalone-scripts</a> propose un patch, place le
<code>.vbs</code> corrigé à côté du <code>.vpx</code>. Un <code>.vbs</code> personnel est conservé sous le nom
<code>&lt;table&gt;.pre_standalone.vbs</code>.</li>
<li><strong>Enrichissement du catalogue</strong> (activé par défaut) : identifie chaque table dans le
<a href="{VPS_DB}">Virtual Pinball Spreadsheet</a> et télécharge une image de fronton et un jingle audio depuis
<a href="{MEDIA_DB}">VPinMediaDB</a> dans le dossier <code>medias</code> de la table. La première synchronisation
télécharge environ 7 Mo de catalogue, plus quelques Mo par table identifiée. Désactivez-le sur un flipper hors
ligne.</li>
</ul>

<h2 id="system">8. Système</h2>
<p>Des options de confort facultatives, surtout utiles sur un flipper dédié :</p>
<ul>
<li><strong>Lancer PinReady automatiquement au démarrage.</strong></li>
<li><strong>Raccourcis dans le menu et association des fichiers <code>.vpx</code></strong> : ajoute PinReady et
Visual Pinball au menu des applications (GNOME, KDE ou XFCE sous Linux, Launchpad sous macOS, menu Démarrer sous
Windows) et ouvre les fichiers <code>.vpx</code> avec Visual Pinball. Décochez et terminez à nouveau l'assistant pour
tout retirer.</li>
<li><strong>Raccourci sur le bureau</strong> et, sous GNOME, <strong>épinglage dans le dock</strong>.</li>
<li><strong>Miroir auto-hébergé</strong> : récupérer les patches de script et la base de médias depuis votre propre
serveur plutôt que depuis GitHub.</li>
</ul>
""")

    if page == "launcher":
        if en:
            return ("Visual Pinball X 10.8.1 table launcher and browser for pincabs – PinReady",
                    "PinReady's table launcher for Visual Pinball X 10.8.1 standalone: backglass grid, search, "
                    "joystick and flipper-button navigation, multi-screen cabinet layout, loading progress, crash reports.",
                    f"""
<h1>The table launcher</h1>
<p class="lead">Once the wizard is done, PinReady opens straight on your tables: pick one, start it, and come back to
the list when Visual Pinball closes.</p>

<h2>Browse your tables</h2>
<ul>
<li>A grid of every table folder found in your tables directory, each with its backglass picture: taken from the
table's <code>.directb2s</code> file, from the artwork downloaded by the catalog enrichment, or from your own
<code>medias/launcher.png</code>.</li>
<li><strong>Search</strong>: just start typing, the list filters as you go.</li>
<li>An <strong>update available</strong> badge on tables with a newer version listed in the Virtual Pinball
Spreadsheet.</li>
<li><strong>Rebuild</strong> clears the cached pictures and script patch state, then scans the tables folder again,
for when the list no longer matches your disk.</li>
<li>The toolbar also has the <strong>Configuration</strong> button to reopen the wizard, a light/dark theme switch, a
rotation button and an About box.</li>
</ul>

<h2>Controls</h2>
<p>The joystick buttons follow the bindings you set in the wizard, so the cabinet buttons work in the launcher
too.</p>
<div class="table"><table><thead><tr><th>Action</th><th>Keyboard</th><th>Cabinet buttons</th><th>Mouse</th></tr></thead><tbody>
<tr><td>Previous / next table</td><td>Left / Right arrow, Left / Right Shift</td><td>Left / Right flipper</td><td>Hover</td></tr>
<tr><td>Row above / below</td><td>Up / Down arrow, Left / Right Ctrl</td><td>Left / Right MagnaSave</td><td>Wheel (cabinet mode)</td></tr>
<tr><td>Start the table</td><td>Enter</td><td>Start or Launch Ball</td><td>Click</td></tr>
<tr><td>Clear the search, then quit</td><td>Escape</td><td>Exit Game</td><td>–</td></tr>
</tbody></table></div>

<h2>On a multi-screen cabinet</h2>
<p>In the Cabinet display mode, the launcher opens full screen on the playfield screen, turned to match a screen lying
flat, with a mouse pointer drawn by PinReady. The other screens are used too:</p>
<ul>
<li><strong>Backglass</strong>: the backglass picture of the selected table.</li>
<li><strong>DMD and topper</strong>: a cover with the Visual Pinball logo.</li>
</ul>
<p>When a table starts, these covers step aside so Visual Pinball can take over every screen.</p>

<h2>Starting a table</h2>
<p>A loading overlay shows the progress reported by Visual Pinball while the table loads. On Linux, PinReady picks
the display driver Visual Pinball runs under (Wayland or X11) according to your desktop session, and keeps the screen
names in <code>VPinballX.ini</code> in step, so each window opens on the right screen.</p>

<h2>When Visual Pinball crashes</h2>
<p>If Visual Pinball exits abnormally, PinReady shows a report instead of silently returning to the list: the command
to reproduce the problem, the working folder, system information, the loading log and the last 100 lines of the game
log, and the exact location of the crash dump on your system (systemd-coredump on Linux,
<code>~/Library/Logs/DiagnosticReports/</code> on macOS, <code>%LOCALAPPDATA%\\CrashDumps\\</code> on Windows).
<strong>Copy</strong> puts it on the clipboard, ready for a bug report; <strong>Save…</strong> writes it to a
file.</p>

<h2>Updates</h2>
<p>PinReady checks for new versions of Visual Pinball and of itself at startup; update buttons appear in the launcher
and install in one click.</p>

<h2 id="cli">Command-line options</h2>
<div class="table"><table><thead><tr><th>Option</th><th>What it does</th></tr></thead><tbody>
<tr><td><code>--config</code>, <code>-c</code></td><td>Open the setup wizard.</td></tr>
<tr><td><code>--reset-wizard</code></td><td>Mark the wizard as not completed and exit: the next start goes back to the wizard.</td></tr>
<tr><td><code>--merge-dry-run SCAN_ROOT [OUTPUT] [--strategy MODE]</code></td><td>Show what the collection import would place, without touching any file.</td></tr>
<tr><td><code>--merge SCAN_ROOT [OUTPUT] [--strategy MODE] [--yes]</code></td><td>Run the import. <code>MODE</code> is <code>copy</code> (default), <code>move</code> or <code>symlink</code>; omit <code>OUTPUT</code> when the collection already has one folder per table.</td></tr>
<tr><td><code>--print-paths</code></td><td>Print the database, log, ini, tables and Visual Pinball paths.</td></tr>
<tr><td><code>--list-tables</code></td><td>Print one line per table folder found.</td></tr>
<tr><td><code>--version</code>, <code>--help</code></td><td>Show the version and licence, or the help.</td></tr>
</tbody></table></div>
""")
        return ("Lanceur et navigateur de tables Visual Pinball X 10.8.1 pour pincab – PinReady",
                "Le lanceur de tables de PinReady pour Visual Pinball X 10.8.1 standalone : grille de frontons, "
                "recherche, navigation au joystick et aux boutons du flipper, multi-écran, progression, rapports de plantage.",
                f"""
<h1>Le lanceur de tables</h1>
<p class="lead">Une fois l'assistant terminé, PinReady s'ouvre directement sur vos tables : choisissez-en une,
lancez-la, et retrouvez la liste quand Visual Pinball se ferme.</p>

<h2>Parcourir vos tables</h2>
<ul>
<li>Une grille de tous les dossiers de tables trouvés dans votre répertoire, chacun avec l'image de son fronton :
tirée du fichier <code>.directb2s</code> de la table, des images téléchargées par l'enrichissement du catalogue, ou de
votre propre <code>medias/launcher.png</code>.</li>
<li><strong>Recherche</strong> : tapez simplement au clavier, la liste se filtre au fur et à mesure.</li>
<li>Un badge <strong>mise à jour disponible</strong> sur les tables dont une version plus récente figure dans le
Virtual Pinball Spreadsheet.</li>
<li><strong>Reconstruire</strong> vide le cache des images et l'état des patches de script, puis analyse de nouveau le
dossier des tables, quand la liste ne correspond plus à votre disque.</li>
<li>La barre d'outils propose aussi le bouton <strong>Config</strong> pour rouvrir l'assistant, le choix du thème clair
ou sombre, un bouton de rotation et une fenêtre « À propos ».</li>
</ul>

<h2>Commandes</h2>
<p>Les boutons du joystick suivent les assignations faites dans l'assistant : les boutons du flipper fonctionnent donc
aussi dans le lanceur.</p>
<div class="table"><table><thead><tr><th>Action</th><th>Clavier</th><th>Boutons du flipper</th><th>Souris</th></tr></thead><tbody>
<tr><td>Table précédente / suivante</td><td>Flèche gauche / droite, Maj gauche / droite</td><td>Batteur gauche / droit</td><td>Survol</td></tr>
<tr><td>Ligne au-dessus / en dessous</td><td>Flèche haut / bas, Ctrl gauche / droit</td><td>MagnaSave gauche / droit</td><td>Molette (mode Cabinet)</td></tr>
<tr><td>Lancer la table</td><td>Entrée</td><td>Start ou Launch Ball</td><td>Clic</td></tr>
<tr><td>Effacer la recherche, puis quitter</td><td>Échap</td><td>Exit Game</td><td>–</td></tr>
</tbody></table></div>

<h2>Sur un flipper à plusieurs écrans</h2>
<p>En mode d'affichage Cabinet, le lanceur s'ouvre en plein écran sur l'écran du plateau, tourné pour un écran posé à
plat, avec un pointeur de souris dessiné par PinReady. Les autres écrans servent aussi :</p>
<ul>
<li><strong>Fronton</strong> : l'image du fronton de la table sélectionnée.</li>
<li><strong>DMD et topper</strong> : un cache avec le logo de Visual Pinball.</li>
</ul>
<p>Au lancement d'une table, ces caches s'effacent pour laisser Visual Pinball prendre tous les écrans.</p>

<h2>Lancer une table</h2>
<p>Pendant le chargement, un écran affiche la progression transmise par Visual Pinball. Sous Linux, PinReady choisit
le pilote d'affichage de Visual Pinball (Wayland ou X11) selon votre session de bureau, et tient à jour les noms
d'écran dans <code>VPinballX.ini</code>, pour que chaque fenêtre s'ouvre sur le bon écran.</p>

<h2>Quand Visual Pinball plante</h2>
<p>Si Visual Pinball se ferme anormalement, PinReady affiche un rapport au lieu de revenir en silence à la liste : la
commande pour reproduire le problème, le dossier de travail, les informations système, le journal de chargement et les
100 dernières lignes du journal de jeu, et l'emplacement exact du fichier de plantage sur votre système
(systemd-coredump sous Linux, <code>~/Library/Logs/DiagnosticReports/</code> sous macOS,
<code>%LOCALAPPDATA%\\CrashDumps\\</code> sous Windows). <strong>Copier</strong> le place dans le presse-papiers, prêt
pour un signalement ; <strong>Enregistrer…</strong> l'écrit dans un fichier.</p>

<h2>Mises à jour</h2>
<p>Au démarrage, PinReady cherche les nouvelles versions de Visual Pinball et de lui-même ; des boutons de mise à jour
apparaissent dans le lanceur et installent en un clic.</p>

<h2 id="cli">Options en ligne de commande</h2>
<div class="table"><table><thead><tr><th>Option</th><th>Effet</th></tr></thead><tbody>
<tr><td><code>--config</code>, <code>-c</code></td><td>Ouvre l'assistant de configuration.</td></tr>
<tr><td><code>--reset-wizard</code></td><td>Marque l'assistant comme non terminé et quitte : le prochain lancement revient à l'assistant.</td></tr>
<tr><td><code>--merge-dry-run SCAN_ROOT [OUTPUT] [--strategy MODE]</code></td><td>Montre ce que l'import de la collection placerait, sans toucher aucun fichier.</td></tr>
<tr><td><code>--merge SCAN_ROOT [OUTPUT] [--strategy MODE] [--yes]</code></td><td>Effectue l'import. <code>MODE</code> vaut <code>copy</code> (par défaut), <code>move</code> ou <code>symlink</code> ; omettez <code>OUTPUT</code> si la collection a déjà un dossier par table.</td></tr>
<tr><td><code>--print-paths</code></td><td>Affiche les chemins de la base de données, du journal, de l'ini, des tables et de Visual Pinball.</td></tr>
<tr><td><code>--list-tables</code></td><td>Affiche une ligne par dossier de table trouvé.</td></tr>
<tr><td><code>--version</code>, <code>--help</code></td><td>Affiche la version et la licence, ou l'aide.</td></tr>
</tbody></table></div>
""")

    if page == "faq":
        items = faq_items(lang)
        body = "\n".join(f'<details class="faq"><summary><h2>{html.escape(q)}</h2></summary>{a}</details>'
                         for q, a in items)
        if en:
            return ("PinReady FAQ: pincab setup on Linux, VPX 10.8.1 standalone troubleshooting",
                    "Answers about PinReady and Visual Pinball X 10.8.1 standalone: who it is for, old tables and VBS "
                    "patches, per-table ini, DOF, tilt and nudge, speakers, Wayland and X11, crash reports.",
                    f"""
<h1>Questions and answers</h1>
<p class="lead">Common questions about PinReady and Visual Pinball X 10.8.1 standalone. Something missing? Ask on
<a href="{DISCORD}">Discord</a>.</p>
{body}
""")
        return ("FAQ PinReady : installer un pincab sous Linux, dépannage de VPX 10.8.1 standalone",
                "Réponses sur PinReady et Visual Pinball X 10.8.1 standalone : pour qui, anciennes tables et patches VBS, "
                "ini par table, DOF, tilt et secousses, enceintes, Wayland et X11, rapports de plantage.",
                f"""
<h1>Questions et réponses</h1>
<p class="lead">Les questions fréquentes sur PinReady et Visual Pinball X 10.8.1 standalone. Il en manque une ?
Posez-la sur <a href="{DISCORD}">Discord</a>.</p>
{body}
""")
    raise KeyError(page)


# ---------------------------------------------------------------- layout

def structured_data(page, lang, title, description):
    app = {
        "@type": "SoftwareApplication",
        "@id": SITE + "#app",
        "name": "PinReady",
        "description": ("Cross-platform configurator and launcher for Visual Pinball X 10.8.1 standalone."
                        if lang == "en" else
                        "Configurateur et lanceur multiplateforme pour Visual Pinball X 10.8.1 standalone."),
        "applicationCategory": "GameApplication",
        "operatingSystem": "Linux, macOS, Windows",
        "url": SITE,
        "downloadUrl": RELEASES,
        "license": "https://www.gnu.org/licenses/gpl-3.0.html",
        "codeRepository": REPO,
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": "EUR"},
    }
    v = version()
    if v:
        app["softwareVersion"] = v
    graph = [app, {"@type": "WebPage", "name": title, "description": description, "url": url(page, lang),
                   "inLanguage": lang, "about": {"@id": SITE + "#app"}}]
    if page == "faq":
        graph.append({"@type": "FAQPage", "url": url(page, lang), "inLanguage": lang, "mainEntity": [
            {"@type": "Question", "name": q,
             "acceptedAnswer": {"@type": "Answer", "text": html.unescape(re.sub(r"<[^>]+>", "", a)).strip()}}
            for q, a in faq_items(lang)]})
    return {"@context": "https://schema.org", "@graph": graph}


def render(page, lang):
    title, description, body = content(page, lang)
    u = UI[lang]
    up = "../" if lang == "fr" else ""
    other, other_label, other_code = u["other"]
    nav = "".join(
        f'<a href="{href(n, lang, lang)}"{" aria-current=\"page\"" if n == page else ""}>{u["nav"][n]}</a>'
        for n in PAGES)
    ld = json.dumps(structured_data(page, lang, title, description), ensure_ascii=False).replace("</", "<\\/")
    verification = ('<meta name="google-site-verification" content="' + GOOGLE_VERIFICATION + '">\n'
                    if page == "index" else "")
    return f"""<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(description)}">
{verification}<link rel="canonical" href="{url(page, lang)}">
<link rel="alternate" hreflang="en" href="{url(page, 'en')}">
<link rel="alternate" hreflang="fr" href="{url(page, 'fr')}">
<link rel="alternate" hreflang="x-default" href="{url(page, 'en')}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="PinReady">
<meta property="og:title" content="{html.escape(title)}">
<meta property="og:description" content="{html.escape(description)}">
<meta property="og:url" content="{url(page, lang)}">
<meta property="og:image" content="{SITE}img/og.jpg">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:locale" content="{'fr_FR' if lang == 'fr' else 'en_GB'}">
<meta property="og:locale:alternate" content="{'en_GB' if lang == 'fr' else 'fr_FR'}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{html.escape(title)}">
<meta name="twitter:description" content="{html.escape(description)}">
<meta name="twitter:image" content="{SITE}img/og.jpg">
<meta name="color-scheme" content="light dark">
<link rel="icon" type="image/svg+xml" href="{up}img/favicon.svg">
<link rel="stylesheet" href="{up}style.css">
<script type="application/ld+json">{ld}</script>
</head>
<body>
<header class="site"><div class="wrap">
<a class="brand" href="{href('index', lang, lang)}"><img src="{up}img/favicon.svg" alt="" width="24" height="24"><span>Pin<b>Ready</b></span></a>
<nav class="main">{nav}</nav>
<a class="lang" href="{href(page, other, lang)}" hreflang="{other}" lang="{other}"><img src="{up}img/{other_code.lower()}.svg" alt="" width="21" height="14">{other_label}</a>
</div></header>
<main><div class="wrap">
{body.strip()}
</div></main>
<footer class="site"><div class="wrap">
<a href="{REPO}">{u["footer_src"]}</a>
<a href="{DISCORD}">{u["footer_chat"]}</a>
<a href="{VIDEO}">{u["footer_video"]}</a>
<span>{u["footer_note"]}</span>
</div></footer>
</body>
</html>
"""


def main():
    (DOCS / "fr").mkdir(exist_ok=True)
    for lang in ("en", "fr"):
        out = DOCS / ("fr" if lang == "fr" else "")
        for page in PAGES:
            (out / ("index.html" if page == "index" else page + ".html")).write_text(render(page, lang), encoding="utf-8")
    urls = []
    for page in PAGES:
        alts = "".join(f'<xhtml:link rel="alternate" hreflang="{l}" href="{url(page, l)}"/>' for l in ("en", "fr"))
        alts += f'<xhtml:link rel="alternate" hreflang="x-default" href="{url(page, "en")}"/>'
        for lang in ("en", "fr"):
            urls.append(f"<url><loc>{url(page, lang)}</loc>{alts}</url>")
    (DOCS / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">\n'
        + "\n".join(urls) + "\n</urlset>\n", encoding="utf-8")
    (DOCS / ".nojekyll").write_text("", encoding="utf-8")


if __name__ == "__main__":
    main()
