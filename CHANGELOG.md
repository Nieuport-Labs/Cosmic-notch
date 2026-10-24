# Changelog

## 0.2.0 – 2026-10-24

- Weather from Open-Meteo for a city you pick (off by default): next to the time and on the home
  page.
- A click outside an island opened with a click closes it.
- Switching tabs slides the new one in from its side.
- An option for a solid background instead of the theme's transparency.
- Google Calendar: events on the calendar tab and a form to add one, and the next event on the
  home page.
- Home page: more room, a greeting by the time of day, a graphics card (AMD, Intel) and a choice of
  which cards show.
- Clipboard history on a shelf of its own (off by default), watched over the panel's connection.
- Sending: devices show their type and which protocol they are reached over, LocalSend or KDE
  Connect, in one list.
- Lyrics page: the line before, the one being sung (bigger, bold) and the one after, scrolling
  smoothly; a click on a line plays from there.
- A mode without animations.
- Dragging a file from one shelf to another no longer makes the island open and close; the file
  moves instead of being copied.
- Heavier font weight for titles, numbers and the clock.
- Saves work: the system monitor sleeps when nothing shows its graphs, lyrics are looked up only on
  the media tab.
- A new app icon.
- Ready for a Flatpak: no helper programs needed.
  - The bars read the sound through libpulse-simple instead of `parec`.
  - KDE Connect is used over D-Bus.
  - Online Accounts open through `flatpak-spawn` inside a Flatpak.

## 0.1.0 – 2026-10-18

First release.

- Compact island over the top panel with the date, time and what is playing, with live audio bars.
- Expands on hover into tabs:
  - Home: CPU, network, memory, disks, battery and a mini player.
  - Media: controls, seeking and synced lyrics from LRCLIB.
  - Shelf: files, text and images, with drag in and out, several shelves, and sending over LocalSend or KDE Connect.
  - Calendar.
  - Tasks: optional sync with Google Tasks through GNOME Online Accounts.
- Follows the COSMIC theme: font, accent, light/dark, transparency, frosted glass and corner radii.
- Steps aside for fullscreen windows.
- Settings inside the island: displays, clock format, delays, compact contents and sections.
- Czech and English translations.
