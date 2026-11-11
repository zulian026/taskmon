# Contributing to TaskMon

Thanks for helping out. TaskMon is a small project — Tauri v2 + Rust backend, Vite + TypeScript frontend — and contributions stay close to how the code already works.

## Setup

Follow [Building from source](README.md#building-from-source): install the Tauri system dependencies, then `npm install`. You need Node.js 18+ and a recent stable Rust toolchain.

## Workflow

1. Fork the repository and create a branch from `main` for your change.
2. Make the change, keeping it focused — one feature or fix per pull request.
3. Run the checks below.
4. Open a pull request against `main` with a short description of what changed, why, and how you tested it. If it fixes an issue, link the issue.

## Checks

```bash
npm run build            # TypeScript (strict) + Vite build; must pass
cd src-tauri
cargo test               # 10 tests: descendant computation, diskstats parsing; must pass
npm run tauri build      # full release build, if you touched the backend or packaging
```

If you changed process behaviour, add or extend a test in `src-tauri/src/monitor.rs` — the descendant walk and the diskstats parser are pure functions, so they are easy to cover without a live system. New backend commands go in `src-tauri/src/lib.rs` with the handler next to the code it calls.

## Style

- Match the existing style of the file you are editing. Rust follows the standard `rustfmt` layout; TypeScript is strict (`tsconfig.json`) and the frontend keeps one icon set in `src/main.ts` (`ICONS`) plus the static SVGs in `index.html`.
- Keep the UI dark and minimal: ground `#000`, surface `#111`, lime `#c6f24e` only on data. No gradients, no glow, no shadows on resting surfaces; colour only on data. New strings are in English.
- Layout is containment-first: grid children get `min-width: 0`, tables use fixed layout with explicit column widths and truncated cells. Verify at 960px and 1440px wide before submitting a UI change.
- Never include secrets, credentials, or personal data in code, tests, or screenshots.

## Reporting bugs

Use the bug report template: TaskMon version, how you installed it (AppImage / .deb / .rpm / standalone binary / built from source), Linux distribution, steps to reproduce, what you expected, and what happened instead. Screenshots of the Processes or Performance view help; never paste anything carrying credentials or tokens.
