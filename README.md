# llm_brain — documentation

Docusaurus site with the project's reasoning: a short overview
(`docs/index.md`, < 2000 words) linking to detail pages.

Live: https://pjcau.github.io/llm_brain/

```bash
npm install
npm start              # http://localhost:3010/llm_brain/  (port 3010: 3000 is used by esp32-emu-turbo)
npm run build          # static build in build/
npm run sync-diagrams  # after editing diagrams/*.mmd
```

Layout:

- `docs/` — pages (overview, iteration log, analysis, architecture, models, roadmap, decisions)
- `diagrams/` — Mermaid sources, the single source of the diagrams; `scripts/sync-diagrams.py` copies them into the pages
- the iteration log is `docs/changelog.md`, the decisions are in `docs/decisions.md`
- `.github/workflows/deploy.yml` — builds and publishes to GitHub Pages on every push to `main`
