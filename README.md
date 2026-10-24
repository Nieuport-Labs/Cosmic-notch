# cosmic-ext-island

English · [Česky](README.cs.md)

![The compact island](res/screenshots/compact.png)

![The home page](res/screenshots/home.png)

A Dynamic Island for the COSMIC desktop (Pop!_OS 24.04). The island sits **over the top panel**:
- at rest it shows the date and time,
- while something plays, it adds the cover, "artist – title" and bars that follow the sound,
- point at it and it opens into a home page, media controls, a shelf for files, a calendar and a
  to-do list.

It follows your COSMIC theme: interface font, accent color, light/dark mode, transparency, frosted
glass and corner radius. Changes in COSMIC Settings show up right away. It steps aside for
fullscreen videos and games. The settings have a solid background and a mode without animations.

## Parts

| Binary | What it does |
| --- | --- |
| `cosmic-ext-island-applet` | The panel applet. Keeps room in the middle of the panel as wide as the island, so other applets are not hidden under it, and starts the island. |
| `cosmic-ext-island` | The island itself. Started by the applet on the connection to the compositor that the panel gives its applets, so it may draw over the panel, also as a Flatpak. Puts one layer surface over the panel on each display. |

## Install

You need Rust 1.88 or newer (from [rustup](https://rustup.rs)) and `just`:

```bash
sudo apt install just pkg-config libxkbcommon-dev libwayland-dev
```

Install for your user, into `~/.local` (no sudo):

```bash
just install
```

For all users:

```bash
just build-release && sudo just prefix=/usr install
```

A Debian package ends up in `target/debian`:

```bash
just deb
```

Then add the *Dynamic Island* applet to the **middle** of the panel in **Settings → Desktop →
Panel → Applets**. The applet starts the island. The date and time applet can go: the island shows
both.

Uninstall with `just uninstall` (or `sudo just prefix=/usr uninstall`).

### Flatpak

`flatpak/` holds the manifest for the [COSMIC Flatpak repository](https://github.com/pop-os/cosmic-flatpak)
and the Rust sources it builds from offline (`cargo-sources.json`, made from `Cargo.lock` with
[flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo);
regenerate it whenever `Cargo.lock` changes). After pushing a release, `just flatpak-pr` writes the
folder to submit to that repository, with the manifest pointing at the pushed commit.

### Runtime dependencies

- COSMIC (cosmic-comp and cosmic-panel) on Wayland.
- `libpulse-simple` (package `libpulse0`, works with PipeWire) for the bars next to the playing
  track. Without it the bars stand still.
- GNOME Online Accounts and `gnome-control-center` for Google Tasks.
- Optionally KDE Connect to send files. LocalSend is not needed on this computer.

## Using it

- **Hover**: after a short delay (300 ms by default) the island opens; it closes when the pointer leaves.
- **Click**: opens the island and keeps it open. Another click or **Esc** closes it.
- **Middle click**: play/pause.
- **Home**: a greeting by the time of day with the next event of Google Calendar, CPU use with
  history, clock speed and temperature, the graphics card (AMD and Intel), network speed, a small
  player whose background fills as the track plays, and one card with memory and disks. Below
  that, the battery and uptime. The settings choose which cards show.
- **Media**: cover, title, seeking and controls. With lyrics switched on in the settings, the line
  being sung runs under the title and the microphone button shows the lyrics page: the line before,
  the one being sung (bigger and bold) and the one after scroll with the track. Click a line to
  play from there. Lyrics come from [LRCLIB](https://lrclib.net); the island sends it the artist, title, album
  and length of the track, so lyrics are off until you switch them on.
- **Shelf**: drag files onto the island. It opens on the shelf, swells a little and its border
  breathes in the accent color; when you let go, it swallows the file. The shelf shows photo
  previews and file type icons. Drag files back out (they are always copied), click to open them,
  or send the whole shelf to a device.
  - **Several shelves**: **+** adds one, a double click renames it, the trash icon next to the
    selected shelf deletes it (asks first when it holds files; the files on disk stay). Hold a file
    over a shelf's name to drop it there.
  - **Clipboard and text**: on the open shelf, **Ctrl+V** (or the clipboard icon) adds what is on
    the clipboard: copied files, an image (kept as PNG) or text. Text can also be dragged in, for
    example selected in a browser. Pasted text and images are kept in
    `~/.local/share/cosmic-ext-island/snippets` and removed once no shelf holds them.
  - **Clipboard history**: switch it on in the settings and the first shelf, *Clipboard*, keeps the
    last 50 things you copied (text, images, files); a click copies one back. What password
    managers copy is never kept. It needs clipboard access, which a Flatpak's sandbox doesn't
    give.
  - **Send to a device**: the send icon lists devices nearby, each with the protocol it is reached
    over. The island speaks the [LocalSend](https://localsend.org) protocol (v2) itself; the other
    device only needs the LocalSend app open. Paired KDE Connect devices show up there too.
- **Calendar**: the month, with today highlighted. With Google Calendar switched on in the
  settings, days that have events get a dot, a click on a day lists its events and **+** adds one
  (title, all day or from–to), through GNOME Online Accounts like Google Tasks.
- **Tasks**: type a task and press Enter. A double click renames it, the cross deletes it. A ticked
  task is struck through and slides away; untick it in time and it stays. Tasks are kept in
  cosmic-config state (`~/.local/state/cosmic/io.github.jiricepelka.CosmicExtIsland`).
- **Bars next to the track** show the actual sound: while something plays, the island reads the
  monitor of the default output. A recording stream named "Dynamic Island" shows
  up in the mixer only then.
- **Keyboard shortcut**: in Settings → Keyboard → Custom shortcuts, add the command:

  ```bash
  busctl --user call io.github.jiricepelka.CosmicExtIsland /io/github/jiricepelka/CosmicExtIsland io.github.jiricepelka.CosmicExtIsland Toggle
  ```

## Google Tasks

The tasks can sync with "My Tasks" in Google Tasks. GNOME Online Accounts handles signing in; the
island only asks it for an access token, so it needs no Google key of its own and never sees your
password.

1. In the island's settings, click **Open Online Accounts** (runs `gnome-control-center
   online-accounts`) and add a Google account. Keep **Tasks** switched on for it.
2. Switch on **Sync with Google Tasks**.

New, renamed, ticked and deleted tasks go to Google right away. Google's list is fetched when the
Tasks tab opens, every minute while it is open, and every 10 minutes otherwise.

## Weather

Switch on **Show the weather** in the settings, type a city and press Enter. The temperature then
shows next to the time (unless you switch that off) and the home page has the weather with today's
high and low. It comes from [Open-Meteo](https://open-meteo.com), which gets the coordinates of the
city, every 30 minutes.

## Google Calendar

With **Google Calendar events** switched on in the settings, the calendar tab shows the events of
your main calendar (a day with events has a dot; click a day to list them) and **+** adds one
(title, all day or from–to). GNOME Online Accounts signs in, as for tasks; keep **Calendar** on for
the account. Events are fetched when the tab opens, every minute while it is open and every 15
minutes otherwise.

## Settings

The gear in the open island shows the settings inside the island:
- which sections the island has (home, media, shelf, calendar, tasks); the last one can't be switched off,
- which displays show the island (same as the panel, all, the active one, selected ones),
- the clock format (empty follows the COSMIC Time applet),
- the delays before opening and closing, hiding behind fullscreen windows, a solid background
  and a mode without animations,
- the weather (a city, and whether it shows next to the time),
- what the compact island shows,
- which cards the home page has,
- the clipboard history,
- Google Tasks and Google Calendar, and lyrics.

Settings are stored with cosmic-config under `io.github.jiricepelka.CosmicExtIsland`.

## Development

```bash
just run
```

`--expand[=home|media|shelf|calendar|tasks|settings]` starts the island open on that tab, which is
handy for screenshots:

```bash
just run --expand=calendar
```


The opening animation follows [Boring Notch](https://boringnotch.com/): a spring (response 0.42 s,
damping 0.8) when opening and no overshoot (damping 1.0) when closing; the content fades and grows
from the top edge.

Tests and lints:

```bash
just check
```

The ignored tests need a live session: an MPRIS player named `org.mpris.MediaPlayer2.mock`, a
LocalSend receiver, or a Google account in GNOME Online Accounts. Run them with
`cargo test -- --ignored`.

## Icons

Icons come from the system COSMIC icon theme first. Where COSMIC has no fitting icon (shelf,
calendar, home, tasks), bundled [Lucide](https://lucide.dev) icons are used (ISC license, see
`res/icons/LICENSE-lucide`).

## License

GPL-3.0-only, see [LICENSE](LICENSE).
