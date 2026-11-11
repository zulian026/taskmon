# TaskMon

TaskMon is a Windows Task Manager-style task manager for Linux, built with
Tauri v2 and Rust.

## Features

- **Processes** — live process list from `/proc` via `sysinfo`: PID, user,
  CPU%, memory, status, run time. Sortable columns, search, row detail
  (command line, executable, working directory, parent PID, disk I/O),
  End task (SIGTERM) and Force kill (SIGKILL).
- **Performance** — CPU and memory history (last 2 minutes), per-core usage,
  load average, RAM/swap meters, disk usage, network throughput, top
  processes by CPU.
- **Startup apps** — freedesktop autostart entries from
  `~/.config/autostart` and `/etc/xdg/autostart`. Toggling a system entry
  writes a user override with `Hidden=true`, the standard mechanism.
- **Services** — systemd services via `systemctl list-units --output=json`,
  user and system scope, with start / stop / restart. System actions may
  ask for authentication through polkit.

## Design

True-black-flat: ground `#000`, surface `#111`, lime `#c6f24e` only on
data. No gradients, no shadows. Fonts are the system UI fonts (Adwaita
Sans / Adwaita Mono, falling back to Noto Sans), no bundled webfonts.
Style dealt with taste-skill (`true-black-flat`, seed 1637137832).
Two themes: dark (default) and light, toggled from the top bar and
remembered in localStorage. In light mode the data accent is a darker
lime green (`#65a30d`) so it stays readable on white. Icons are a
single inline SVG set (16px, 1.5px stroke, currentColor): nav items,
search, theme toggle (sun/moon), End task, Refresh, service actions
(play/stop/restart), KPI labels and the sidebar footer. Layout is
containment-first: every grid child has `min-width: 0`, tables use
fixed layout with truncated name/description columns, and the KPI
values shrink instead of spilling into the next cell.

## Build

```sh
npm ci
npm run tauri build
```

Requires the Tauri v2 Linux dependencies (GTK 3, WebKitGTK 4.1, libayatana
appindicator). On the dev VM the `-dev` packages were restored from cached
`.deb` files in `~/tmp-webkit/` with `dpkg -i --force-depends` after a VM
reset; see `AGENTS.md`.

## Layout

- `src/` — frontend (Vite + TypeScript, no framework)
- `src-tauri/src/monitor.rs` — sampling thread, snapshot, kill, diskstats
- `src-tauri/src/startup.rs` — autostart `.desktop` parsing and toggling
- `src-tauri/src/services.rs` — systemctl listing and actions
- `src-tauri/src/lib.rs` — Tauri commands and app setup

## License

MIT
