# lami

Rétegzett, deklaratív rendszerkonfiguráció Arch Linuxra — csomagok, `/etc`,
systemd unitok és dotfile-ok **egy** eszközzel, **egy** configgal.

> **Állapot: korai fejlesztés.** Jelenleg csak olvasó parancsok működnek
> (`list`, `show`, `why`). Semmit nem ír a rendszeredre.

## Miért

Ha több Arch gépet tartasz hasonlóan, ma három eszközt kell összeragasztanod:
egyet a csomagokra, egyet a `/etc`-re, egyet a dotfile-okra. Mindegyiknek saját
config-nyelve, saját „mi változott" fogalma, és egyiknek sincs rendes válasza
arra, hogy **hogyan szívd vissza** a gépen élőben elvégzett beállítást.

A `lami` egyetlen modellre húzza mindezt: minden **erőforrás** (csomag, fájl,
szolgáltatás), és mindegyikre ugyanaz az életciklus érvényes — feloldás,
összehasonlítás, alkalmazás, **visszaszívás**.

## A config

Egy réteg = egy könyvtár, egy fájllal. Felülről lefelé olvasható:

```kdl
// layers/gui/layer.kdl
description "Működő Hyprland asztal"
needs "core"

packages {
    hyprland
    greetd
    firefox     // a munkahelyi SSO miatt kell, ne cseréld chromiumra
}

services {
    greetd
    power-profiles-daemon    // a caelestia-shell hard dependency-je
}

when gpu="nvidia" {
    packages { nvidia-open; nvidia-utils; egl-wayland }
}
```

**Az AUR-os csomagoknak nincs külön blokkjuk.** Ugyanabban a `packages` listában
vannak, és a lami a pacman sync adatbázisából tudja, mi jön honnan — a config
írásakor ezt nem kell fejben tartanod.

Egy gép azt mondja meg, mely rétegeket kapja és milyen paraméterekkel:

```kdl
// hosts/frodo.kdl
description "Asztali gép"

layers "core" "tools" "gui" "rice"

gpu   "intel"
ucode "intel"
class "desktop"
ddc   on            // külső monitor fényereje DDC/CI-n
```

A kapcsolók `on` / `off` alakúak. Az `off`-nak azért van értelme, holott a sor
elhagyása is kikapcsolná: **önmagát dokumentálja**. Egy hiányzó sorból nem derül
ki, hogy mérlegelted-e a dolgot, vagy csak elfelejtetted.

Ugyanaz a `gui` réteg fut Intel iGPU-n és RTX 5080-on — csak a `gpu` paraméter más.

## Eredetkövetés

Minden erőforrás megmondja, honnan jön és miért kapja ez a gép:

```
$ lami why nvidia-open
nvidia-open  (csomag)
  deklarálva:  layers/gui/layer.kdl:24
  azért kapod: a(z) 'gui' réteg szerepel sam layers listájában
  feltétel:    gpu=nvidia (a gépen: gpu = nvidia)
```

## Kipróbálás

```sh
cargo build
./target/debug/lami --config-dir examples/minimal list
./target/debug/lami --config-dir examples/minimal --host frodo show
./target/debug/lami --config-dir examples/minimal --host sam why nvidia-open
```

A config helye alapból `$XDG_CONFIG_HOME/lami`; `--config-dir` vagy
`LAMI_CONFIG_DIR` felülbírálja.

## Tervezési elvek

- **A config érthetősége az első.** A jogosultság az útból következik, a
  `.service` suffixet kitaláljuk, a rövid tartalom helyben van, a feltétel
  mondatként olvasható.
- **Arch-only, vállaltan.** pacman, AUR, systemd, mkinitcpio beépítve — nem
  absztrakció mögött.
- **Ne köss, hívj.** Nem linkeljük a libalpm-et: amikor a pacman ABI-t vált, ne
  pont az az eszköz essen ki, amivel javítanál.
- **Az `apply` sosem töröl.** Az eltávolítás külön parancs, külön megerősítéssel.

## Licenc

MIT
