# Social assets

Rebuildable cards for README, docs Open Graph, and LinkedIn/X. Story matches the README “What you get” five-step strip (Create → Day-2 → Access → Guard → Audit).

| File | Size | Use |
|------|------|-----|
| `zorvia-share-card.png` | 1200×630 | README hero (light), docs site `themeConfig.image`, Twitter/OG |
| `zorvia-share-card-dark.png` | 1200×630 | README hero for dark themes (`<picture>` `prefers-color-scheme: dark`) |
| `zorvia-social-card.jpg` | 1600×900 | LinkedIn / X posts |
| `zorvia-share-card.html` · `zorvia-share-card-dark.html` · `zorvia-social-card.html` | sources | Edit HTML, then rebuild |

## Palette

apple.com blue and white: white → `#f5f5f7` background with a faint blue wash, ink `#1d1d1f`, secondary `#6e6e73`,
hairline `#d2d2d7`, blue `#0071e3` → `#2997ff`. Dark card: `#000` / `#0b0b0f`, text `#f5f5f7`, cards `#1d1d1f`,
blue `#0a84ff` / `#2997ff`. Type is Helvetica Neue with Menlo for labels. Orange `#ff6a2a` appears exactly once per
image, as the small dot on the Create step; the Zyvor mark is drawn inline in blue.

## Rebuild

```bash
./docs/social/build-social-cards.sh
```

Needs Google Chrome and macOS `sips` (override the browser with `CHROME=/path/to/chrome`). The script renders all
outputs above and the comparison cards below.

## GitHub social preview

The repository's **Social preview** image (Settings → Social preview) cannot be set through the API or `gh`;
upload `zorvia-share-card.png` there by hand after it changes.

Licence wording follows `LICENSE` (Zyvor Production Licence v2.0). The version chip tracks `Cargo.toml` (currently 0.7.4) and the
CHANGELOG.
