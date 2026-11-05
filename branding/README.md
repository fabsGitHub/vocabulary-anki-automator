# Polywort brand assets

The source of the app icon of both apps (Polywort and Polywort Learn) and of
the website's mark. Everything else is generated from these files.

| File            | What it is                                                                                   |
| --------------- | -------------------------------------------------------------------------------------------- |
| `icon.svg`      | The app icon on Apple's 1024 grid: a selected word with a text cursor on a card, over a deck |
| `icon-1024.png` | `icon.svg` rendered at 1024 × 1024 (RGBA), the input for `tauri icon`                        |
| `glyph.svg`     | One-colour mark (`currentColor`) for the website header and other single-colour uses         |
| `favicon.svg`   | Full-colour icon simplified for 16 and 32 px (browser tabs)                                  |
| `gen_icon.py`   | Writes `icon.svg`; its settings are at the top of the file                                   |

Palette: body `#FF9A5C` → `#E2306B`, selection `#AE98FF` → `#9478FA`, ink
`#3A1650` → `#2C0F40`, card text lines `#D9D0F0`.

## Changing the icon

```bash
python3 branding/gen_icon.py branding/icon.svg            # optional: key=value overrides
rsvg-convert -w 1024 -h 1024 branding/icon.svg -o branding/icon-1024.png
npx tauri icon branding/icon-1024.png -o src-tauri/icons
npx tauri icon branding/icon-1024.png -o apps/learn/src-tauri/icons
```

`tauri icon` also writes `android/`, `ios/` and `64x64.png`; neither app has a
mobile build, so delete those again and keep the file set the folders already
have. The website copies its icons from `site/src/assets`, see `site/README.md`.
