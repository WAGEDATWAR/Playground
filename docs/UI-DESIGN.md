# UI design language

The look of the game's own screens (menus, windows, the HUD). The code is `crates/pg-app/src/theme.rs`; this page
says why. A change of look is a change to that one file. The widget model (`pg-ui-model`) never names a colour:
it says what a thing is (a button, the main action, a box with a line between its neighbours) and the shell
decides how it looks.

## The idea: a town at dusk

The pixel-art map is the star, so the interface stays quiet and a little warm around it. Nothing is pure black
or pure white, so the map is always the brightest thing on screen. Surfaces are deep blue-slate; the one warm
colour, lantern amber, is kept for the few things that matter.

## Palette

| Role | Token | Colour | Used for |
|---|---|---|---|
| Screen | `BG` | `#14171F` | behind everything |
| Card | `SURFACE` | `#1C212C` | panels, the HUD bar, the main-menu card |
| Window | `RAISED` | `#232936` | floating windows and popups |
| Control | `CONTROL`, `_HOVER`, `_ACTIVE` | `#2C3344`, `#374056`, `#424C66` | resting, hovered and pressed buttons and fields |
| Hairline | `HAIRLINE` | `#343C4E` | quiet outlines, dividers |
| Edge | `EDGE` | `#5B667E` | outlines that should be seen (a column's frame, a hovered control) |
| Text | `TEXT`, `TEXT_MUTED` | `#E7E9EF`, `#9CA5B8` | body, secondary and notes |
| **Lantern amber** | `ACCENT` (+ `_HOVER`, `_PRESSED`, `ON_ACCENT`, `ACCENT_QUIET`) | `#F2B84B` | **act here / you are here:** the focus ring, the one main button, section titles (turned down to `#D8B06A`) |
| **Teal** | `INFO`, `INFO_STRONG` | `#3F8696`, `#6CB8C8` | **information:** selections, bars, sliders |
| Status | `WARN` (amber), `BAD` | `#F2B84B`, `#E5766B` | warnings and errors |

Two accents, two fixed jobs: amber is about action and place, teal is about data. A screen should rarely show more
than one amber button.

## Shape and type

- Corners: controls 6, cards and columns 8, windows 10 (points). Borders are hairlines (1 pt); a column that frames a
  group of settings uses the stronger `EDGE`.
- Type: title 30, section titles 17 (bold, quiet amber), body and controls 16 to 17, notes 13 (muted).
- Space: items 10 by 9, button padding 14 by 6, window margin 14, a minimum control height of 30 to 34.
- Floating things have a soft drop shadow so they lift off the map without a heavy frame.

## Rules for screens

1. **One main action per screen** gets `Widget::Primary` (Continue, Create world, Resume, Load). Everything else is
   a normal button. Destructive actions are never primary.
2. **Sections** are groups with a quiet amber title and a line between neighbours (`Border::between`); a whole
   settings column sits in one `Border::solid(2, Strong)` frame. Boxes inside a box are avoided.
3. **Buttons that finish a screen** (Create, Back, Close) go in a pinned footer button bar, side by side.
4. **Over the map** (menus on the backdrop, windows in game) use a card or window surface, never bare text.
5. Never hard-code a colour in a screen or widget. Add a token to `theme.rs` first.

## Still to do in this direction (milestone 1.8b)

Journal, console and inspector windows (tabs and a calmer layout), the HUD strip, a second New world pass, the Mods
items in `docs/TODO.md`, motion (hover and press easing, windows fading in), and a custom pixel-friendly UI font
(licence permitting).
