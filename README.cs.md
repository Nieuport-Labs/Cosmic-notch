# cosmic-ext-island

[English](README.md) · Česky

![Kompaktní ostrov](res/screenshots/compact.png)

![Domovská stránka](res/screenshots/home.png)

Dynamic Island pro desktop COSMIC (Pop!_OS 24.04). Ostrov sedí **přes horní panel**:
- v klidu ukazuje datum a čas,
- při přehrávání obal, „interpret – skladba“ a pruhy,
- po najetí myší se rozbalí na domovskou stránku, média, přihrádku souborů, kalendář a úkoly.

Vzhled se řídí motivem COSMIC: font rozhraní, akcentní barva, světlý/tmavý režim, průhlednost,
ledové sklo (frosted) a zaoblení. Změny v Nastavení COSMIC se projeví okamžitě.

## Části

| Binárka | Co dělá |
| --- | --- |
| `cosmic-ext-island-applet` | Applet do středu panelu. Drží v panelu místo široké jako ostrov, aby se pod něj nedostaly jiné applety, a spouští ostrov. |
| `cosmic-ext-island` | Samotný ostrov. Spouští ho applet na spojení s kompozitorem, které panel appletům dává, takže smí kreslit přes panel, i jako Flatpak. Na každém monitoru vytvoří jednu layer-surface nad panelem. |

## Sestavení a spuštění z repozitáře

```bash
sudo apt install just
```

```bash
just run
```

Volby pro vývoj a snímky obrazovky:

- `--expand[=home|media|shelf|calendar|tasks]` spustí ostrov rovnou rozbalený.
- `--expand=settings` otevře rovnou nastavení.

```bash
just run --expand=calendar
```


Animace otevírání napodobuje [Boring Notch](https://boringnotch.com/): pružina (response 0,42 s,
tlumení 0,8) při otevírání, bez překmitu (tlumení 1,0) při zavírání, obsah se prolne a zvětší od horního okraje.

## Instalace

```bash
just install
```

Nainstaluje se do `~/.local` (bez sudo).

Systémová instalace (pro všechny uživatele): `just build-release && sudo just prefix=/usr install`.
Balík `.deb`: `just deb`, výsledek je v `target/debian`.

Pak v **Nastavení → Plocha → Panel → Applety** přidej applet *Dynamický ostrov* do **středu**
panelu. Applet ostrov spustí. Applet s datem a časem můžeš odebrat, ostrov ukazuje datum a čas sám.

### Flatpak

Složka `flatpak/` obsahuje manifest pro [repozitář COSMIC Flatpak](https://github.com/pop-os/cosmic-flatpak)
a Rust zdroje, ze kterých se staví bez internetu (`cargo-sources.json`, vzniká z `Cargo.lock`
nástrojem [flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo);
při každé změně `Cargo.lock` ho vygeneruj znovu). Po pushi vydání připraví `just flatpak-pr`
složku pro pull request do toho repozitáře, s manifestem ukazujícím na pushnutý commit.

### Závislosti za běhu

- COSMIC (cosmic-comp a cosmic-panel), Wayland.
- `libpulse-simple` (balík `libpulse0`, funguje i s PipeWire) pro pruhy u skladby. Bez něj pruhy stojí.
- Pro Google Tasks GNOME Online Accounts a `gnome-control-center`.
- Volitelně KDE Connect pro posílání souborů. LocalSend na tomto počítači není potřeba.

Odinstalace: `just uninstall`.

## Ovládání

- **Najetí myší**: po prodlevě (výchozí 300 ms) se ostrov rozbalí. Po odjetí se sbalí.
- **Klik**: rozbalí a zafixuje ostrov. Další klik nebo **Esc** ho zavře.
- **Prostřední tlačítko**: přehrát/pozastavit.
- **Přetažení souborů**: když držíš soubor nad ostrovem, ostrov se rozbalí na přihrádku, celý se
  lehce nafoukne a jeho okraj dýchá barvou akcentu. Po puštění soubor „spolkne“: krátce se stáhne,
  pružinou se vrátí a okraj dobliká. Přihrádka ukazuje náhledy fotek a ikony typů a složek. Soubor z ní jde
  vytáhnout jinam (vždy se kopíruje, pod kurzorem je vidět jeho náhled) nebo poslat přes LocalSend
  či KDE Connect, pokud jsou nainstalované.
- **Více přihrádek**: nad soubory jsou názvy přihrádek. **+** přidá novou, dvojklik na název ji
  přejmenuje, ikona koše vedle vybrané přihrádky ji smaže (neprázdnou až po potvrzení; soubory na
  disku zůstanou). Když soubor při tažení podržíš nad názvem přihrádky, přepne se na ni a soubor
  skončí v ní. Vyprázdnění a odeslání jsou ikony v horní liště.
- **Historie schránky**: po zapnutí v nastavení drží první přihrádka *Schránka* posledních 50
  kopií (text, obrázky, soubory); kliknutím se kopie vrátí do schránky. Co kopírují správci hesel,
  se neukládá. Potřebuje přístup ke schránce, který sandbox Flatpaku nedává.
- **Schránka a text**: v otevřené přihrádce **Ctrl+V** (nebo ikona schránky v liště) vloží, co je ve
  schránce: zkopírované soubory, obrázek (uloží se jako PNG) nebo text. Text jde do přihrádky i
  přetáhnout, třeba označený v prohlížeči. Z textu je kartička s náhledem; vytažením do editoru se
  vloží jako text, do složky jako soubor `.txt`, kliknutím se zkopíruje zpět do schránky. Kliknutí
  na soubor ho otevře. Vložené texty a obrázky se drží v `~/.local/share/cosmic-ext-island/snippets`
  a smažou se, jakmile nejsou v žádné přihrádce.
- **Domů**: pozdrav podle denní doby s další událostí z Google Kalendáře, vytížení procesoru
  s historií, frekvencí a teplotou, grafika (AMD a Intel), rychlost sítě (stahování nahoru,
  odesílání dolů), malý přehrávač a jedna karta s pamětí a disky. Pod tím je baterie a doba od
  zapnutí. Které karty se ukážou, se volí v nastavení. Údaje se čtou z `/proc` a `/sys` jednou za sekundu. Síťové disky se vynechávají, protože
  dotaz na volné místo by mohl viset.
- **Úkoly**: napiš úkol a stiskni Enter. Dvojklik na úkol ho přejmenuje, křížek ho smaže. Po
  odškrtnutí políčko poskočí, text přeškrtne čára, úkol odjede doprava a seznam se zavře; když ho
  stihneš odškrtnout zpět, zůstane. Úkoly se
  ukládají ve stavu cosmic-config (`~/.local/state/cosmic/io.github.jiricepelka.CosmicExtIsland`).
- **Odeslání do zařízení**: ikona odeslání otevře stránku se zařízeními v síti. Ostrov sám mluví
  protokolem [LocalSend](https://localsend.org) (v2), na tomto počítači LocalSend být nemusí; na
  druhém zařízení stačí otevřená aplikace LocalSend. Kliknutím na zařízení se pošle celá přihrádka
  (složky i s obsahem), u zařízení je vidět stav: čeká na přijetí, odesláno x z y, hotovo. Spárovaná
  zařízení KDE Connect se ukážou tamtéž; u každého je vidět, přes jaký protokol se pošle.
- **Texty písní**: v záložce Média běží pod názvem aktuální řádek textu; tlačítko s mikrofonem
  ukáže stránku s texty: řádek před, aktuální (větší a tučný) a řádek po plynule sjíždějí podle
  skladby. Kliknutí na řádek přehraje od něj. Texty pocházejí z [LRCLIB](https://lrclib.net) – ostrov
  tam pošle interpreta, název, album a délku skladby, proto jsou ve výchozím stavu vypnuté
  a zapínají se v nastavení. Nalezené texty se
  ukládají do `~/.cache/cosmic-ext-island/lyrics`.
- **Pruhy u skladby** ukazují skutečný zvuk: ostrov při přehrávání čte monitor výchozího výstupu.
  V mixéru se proto objeví nahrávací proud „Dynamic Island“, jen když něco hraje.
- **Klávesová zkratka**: v Nastavení → Klávesnice → Vlastní zkratky přidej příkaz:

  ```bash
  busctl --user call io.github.jiricepelka.CosmicExtIsland /io/github/jiricepelka/CosmicExtIsland io.github.jiricepelka.CosmicExtIsland Toggle
  ```

## Google Tasks

Úkoly se umí synchronizovat se seznamem „Moje úkoly“ v Google Tasks. Přihlášení obstarává GNOME
Online Accounts, ostrov si od něj jen bere přístupový token, takže nepotřebuje vlastní Google klíč
a heslo nikdy nevidí.

1. V nastavení ostrova klikni na **Otevřít Online účty** (spustí `gnome-control-center
   online-accounts`) a přidej Google účet. Nech u něj zapnuté **Úkoly**.
2. Zapni **Synchronizovat s Google Tasks**.

Nové, přejmenované, odškrtnuté a smazané úkoly jdou do Googlu hned. Seznam z Googlu se stahuje
při otevření záložky Úkoly, každou minutu, dokud je otevřená, a jinak jednou za 10 minut. Úkoly vytvořené bez připojení se nahrají při další
synchronizaci.

## Počasí

V nastavení zapni **Ukazovat počasí**, napiš město a stiskni Enter. Teplota se pak ukáže vedle času
(jde vypnout) a domovská stránka má počasí s dnešním maximem a minimem. Údaje jsou z
[Open-Meteo](https://open-meteo.com), které každých 30 minut dostane souřadnice města.

## Google Kalendář

Po zapnutí **Události z Google Kalendáře** v nastavení se v záložce Kalendář ukážou události
hlavního kalendáře (den s událostí má tečku, kliknutím na den se vypíšou) a tlačítko **+** přidá
novou (název, celý den nebo od–do). Přihlášení obstarává GNOME Online Accounts stejně jako u
úkolů; u účtu nech zapnutý **Kalendář**. Události se stahují při otevření záložky, každou minutu,
dokud je otevřená, a jinak jednou za 15 minut.

## Nastavení

Ozubené kolo v rozbaleném ostrově přepne na stránku nastavení přímo v ostrově:
- které části ostrov má (domů, média, přihrádka, kalendář, úkoly); poslední zapnutá vypnout nejde,
- na kterých monitorech se ostrov zobrazí (stejně jako panel, všechny, aktivní, vybrané),
- formát hodin (prázdné = podle appletu Čas v COSMICu),
- prodlevy rozbalení a sbalení, skrytí za oknem přes celou obrazovku, plné pozadí a režim bez
  animací,
- počasí (město a jestli se ukazuje i vedle času),
- co ukazovat v kompaktním stavu,
- které karty má domovská stránka,
- historii schránky,
- Google Tasks a Google Kalendář, texty písní.

Nastavení se ukládá přes cosmic-config pod `io.github.jiricepelka.CosmicExtIsland`.

## Ikony

Ikony pochází primárně ze systémového motivu ikon COSMIC. Kde COSMIC odpovídající ikonu nemá
(přihrádka, kalendář), používají se přibalené ikony [Lucide](https://lucide.dev) (licence ISC,
viz `res/icons/LICENSE-lucide`).

## Testy

```bash
just check
```

Integrační test ovládání přehrávače potřebuje MPRIS přehrávač `org.mpris.MediaPlayer2.mock`
a spouští se s `cargo test -- --ignored`.

## Licence

GPL-3.0-only
