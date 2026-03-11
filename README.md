<h1 align="center">niri-autostart</h1>
<p align="center">Declarative autostart and layout restoration for niri.</p>

<p align="center">
    <a href="#about">About</a> | <a href="#configuration">Configuration</a> | <a href="#status">Status</a>
</p>

## About

`niri-autostart` reads a KDL file, subscribes to the `niri` IPC event stream, keeps an in-memory model of outputs, workspaces and windows, and converges the compositor toward the declared layout.

It is intended to replace ad-hoc startup shell scripts with a small event-driven tool:

- no `sleep`
- no blind polling loops
- no respawn of windows that already exist on the right workspace
- geometry, focus and workspace activation are restored from config

## Features

- Declarative KDL config with `output`, `workspace`, `column` and `window`
- Uses `niri-ipc` types directly
- Waits on real `event-stream` state changes
- Reuses existing windows by exact `app-id`
- Prefers the matching tiled window on the target workspace when duplicates exist
- Can be launched directly from `startup.kdl`

## Configuration

Example:

```kdl
autostart {
    output "eDP-1" {
        workspace "code" {
            column {
                width {
                    proportion 1.0
                }

                window app-id="cursor" {
                    command "cursor"
                    height {
                        proportion 1.0
                    }
                }
            }
        }
    }
}
```

The default config path is:

```text
~/.config/niri-autostart/config.kdl
```

Typical startup entry in `niri`:

```kdl
spawn-at-startup "/home/geles/.local/bin/niri-autostart" "--config" "/home/geles/.config/niri-autostart/config.kdl"
```

## Status

`niri-autostart` currently works as a `oneshot` startup tool for a declarative multi-workspace layout.

It is focused on one job:

- open missing applications
- reuse existing ones when possible
- move them to the right workspace
- restore tiling geometry
- leave every workspace focused on its first window
