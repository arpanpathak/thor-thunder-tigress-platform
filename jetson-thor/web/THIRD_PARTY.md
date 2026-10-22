# Third-party libraries in the chat page

The page loads each library the first time a reply or an attached file needs
it. The agent serves them under versioned names with a year-long cache, so a
browser downloads each one once. Their licences are in `licenses/`.

| File | From | Licence | Used for |
|---|---|---|---|
| `katex-0.19.0.min.js` | npm `katex@0.19.0`, `dist/katex.min.js` | MIT | formulas |
| `katex-0.19.0.css` | npm `katex@0.19.0`, `dist/katex.min.css`, with the 20 `woff2` fonts inlined as data URLs (the `woff`/`ttf` fallbacks dropped) | MIT | formulas |
| `mermaid-12.1.0.min.js` | npm `mermaid@12.1.0`, `dist/mermaid.min.js` | MIT | diagrams |
| `pdf-6.4.299.min.mjs` | npm `pdfjs-dist@6.4.299`, `build/pdf.min.mjs` | Apache-2.0 | reading attached PDFs |
| `pdf.worker-6.4.299.min.mjs` | npm `pdfjs-dist@6.4.299`, `build/pdf.worker.min.mjs` | Apache-2.0 | reading attached PDFs |

npm tarball integrity, checked when the files were taken:

```
katex@0.19.0        sha512-v6Tznz3zJ7u3niRCoDTsumM2+HA2XXcCu+WAacCeHD2z3p9A9Ks987o5FzfTGBN0e8A0vjEgIDvLbICrXpdw/Q==
mermaid@12.1.0      sha512-wlVCp+8eTupfCeeFvoZNNiTuHrvag0P2jz/ILgb/f/6jkVokUefOcujefi8qUe/j2asHiSePncVsz/xzzA80LQ==
pdfjs-dist@6.4.299  sha512-AVl138zALtfaAPvADulE0PZThbYzCBS79nL4pOSL/6Sm/4AH5A21BD9VHt97OlCuzJuCpmeZtAtkinisF4Vb1g==
```

## Updating one

1. Take the file from the npm tarball (`npm pack <name>@<version>`, check its
   `integrity` against `npm view <name>@<version> dist.integrity`).
2. Name it with the new version, and change the name in the page
   (`rich.js` or `attachments.js`) and in the agent's `LIBRARIES` list
   (`crates/thor-tigress-agent/src/server/routes.rs`). A new name is what makes
   browsers fetch the new version instead of the one they keep.
3. Update this table and the licence file.
