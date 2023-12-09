# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Special color for unavailable items
- Changelog with all the relevant user-facing changes to the project
- `info` command line subcommand to show platform specific information

### Changed

- Improve error messages generated by the command line
- Build with crossterm terminal backend by default
- Move UNIX IPC socket from the user's cache path to the user's runtime directory
- Improve messages relating to errors in the configuration file

### Fixed

- Crash when internal commands can't be handled
- Documentation for the behavior of the `Ctrl+S` keybinding
- Multiple instances interfering with each other's MPRIS implementation
- An unlikely crash when the UNIX IPC socket is removed before `ncspot` is closed
- Guaranteed crash while quiting `ncspot` when using MPRIS
- MPRIS volume not being updated when given numbers smaller than 0 or larger than 1
- Allow previous track via MPRIS if first track in queue is playing

## [0.13.4] - 2023-07-24

### Added

- `save current` and `add current` commands for saving/adding the currently playing item
- Visual indicator for local tracks
- Error when deleting local tracks, as this is currently not possible


### Changed

- Improve release profile to generate higher quality executables
- Ignore a leading 'The' when sorting artists
- Improve context menu loading time

### Fixed

- Crash when deleting local tracks from a playlist

## [0.13.3] - 2023-06-11

### Added

- Instructions for installation with `cargo`

### Removed

- Instructions for installation with Snap as the package has been removed

### Fixed

- Disappearing selection in lists when removing the last element
- Playback notification not being shown on some systems
- IPC socket being overriden when opening multiple instances
- IPC socket persisting after closing `ncspot`
- Crash when submitting a command with a multibyte unicode character as the prefix
- Tabs switching when command line is active and `Left`/`Right` is held down

## [0.13.2] - 2023-05-04

### Added

- `ncurses_backend` feature flag to compile with `ncurses` support

### Fixed

- Crash when running with MPRIS support while D-Bus isn't available
- Nerdfont icons not rendering correctly

## [0.13.1] - 2023-04-05

### Added

- Vim-like page scrolling commands like `Ctrl+d`/`Ctrl+u` (not set to keybinding by default)
- Double click emulation to play items

### Changed

- Split up the documentation to make it easier to find relevant information

### Fixed

- Documentation about library tabs to include the new browse tab
- Incorrect D-Bus paths for the MPRIS implementation

## [0.13.0] - 2023-03-09

### Added

- Example for how to use the UNIX domain socket IPC interface
- `HighlightInactive` option to the theme config
- `reconnect` command to reconnect to Spotify if network changes broke the current session
- Support for basic password managers
- Man page support
- Shell completion support

### Changed

- Improve the way duration for items is displayed
- Remove `pulseaudio` dependency from the Debian package

### Fixed

- `Ctrl+z` shortcut not working to put `ncspot` in the background
- Cover not being sent to the notification if not compiled with the `cover` feature

## [0.12.0] - 2022-12-29

### Added

- Context menu action to save/remove the album of the selected track to the library
- Flatpak option to installation instructions
- IPC socket on UNIX platforms to control `ncspot` remotely

### Fixed

- Crash information being dumped in the terminal. It is now automatically written to a file instead.
- Bug causing command prompt to appear after 'Connecting to Spotify..' startup message after quiting
  `ncspot`

## [0.11.2] - 2022-10-15

### Added

- More context menu options to items to more closely match what's already offered for tracks
- POSIX signal handling to quit gracefully when not closed by the user (eg. system shutdown)

### Changed

- Move playback controls out of their own submenu in the context menu

### Fixed

- Crash when trying to play an empty, shuffled queue

## [0.11.1] - 2022-09-17

### Changed

- Mouse scrolling now moves the view, not the focussed element
- Setting the track position now only works with a click, not by dragging the progress bar

### Fixed

- Back button going back twice when clicked once with the mouse
- Back button clickable area not corresponding to the text on the button
- Bug while dragging the track progress bar
- Flickering when using the Termion TUI backend
- Albums with more than 50 songs not showing all the songs when viewed in the library
- Bug that could cause items to not load until the screen is filled on bigger screens

[Unreleased]: https://github.com/hrkfdn/ncspot/compare/v0.13.4...HEAD
[0.13.4]: https://github.com/hrkfdn/ncspot/compare/v0.13.3...v0.13.4
[0.13.3]: https://github.com/hrkfdn/ncspot/compare/v0.13.2...v0.13.3
[0.13.2]: https://github.com/hrkfdn/ncspot/compare/v0.13.1...v0.13.2
[0.13.1]: https://github.com/hrkfdn/ncspot/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/hrkfdn/ncspot/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/hrkfdn/ncspot/compare/v0.11.2...v0.12.0
[0.11.2]: https://github.com/hrkfdn/ncspot/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/hrkfdn/ncspot/compare/v0.11.0...v0.11.1