# Changelog

## Unreleased

- The panel applet starts the island, on the connection to the compositor the panel gives its
  applets. The island no longer starts from autostart; add the applet to the middle of the panel.
- Ready for a Flatpak: no helper programs needed.
  - The bars read the sound through libpulse-simple instead of `parec`.
  - KDE Connect is used over D-Bus.
  - Online Accounts open through `flatpak-spawn` inside a Flatpak.
- Hiding behind fullscreen windows is left to the compositor, as for the panel, so the island no
  longer follows windows itself.
- A full-color app icon.

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
