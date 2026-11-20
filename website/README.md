# Zorvia docs site

Built with [Docusaurus](https://docusaurus.io/). Serves the live docs at https://zyvorai.github.io/zyvor-zorvia/.

This points directly at the repo's existing `docs/` folder rather than a hand-curated copy. Add/edit docs in `../docs/` as usual.

## Local development

```bash
npm install
npm start
```

## Build

```bash
npm run build
npm run serve
```

## Deployment

Automatic via `.github/workflows/pages.yml` on every push to `main` touching `website/`, `docs/`, or the workflow file.
