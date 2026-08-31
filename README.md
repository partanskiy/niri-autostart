<h1 align="center">niri-autostart</h1>
<p align="center">Declarative autostart and layout restoration for niri.</p>

<p align="center">
    <a href="#about">About</a> | <a href="#configuration">Configuration</a> | <a href="#installation">Installation</a> | <a href="#status">Status</a>
</p>

## About

`niri-autostart` reads a KDL file, subscribes to the `niri` IPC event stream, keeps an in-memory model of workspaces and windows, and converges the compositor toward the declared layout. Workspaces can optionally be assigned to a specific output.

It is intended to replace ad-hoc startup shell scripts with a small event-driven tool:

- no `sleep`
- no blind polling loops
- no respawn of windows that already exist on the right workspace
- geometry, focus and workspace activation are restored from config

## Features

- Declarative KDL config with `workspace`, optional workspace `output`, `column` and `window`
- Uses `niri-ipc` types directly
- Waits on real `event-stream` state changes
- Tracks windows it spawned in a runtime state file under `/tmp`
- Reuses recorded live `window_id`s; falls back to exact `app-id` for app ids that are unique in the config
- Can be launched directly from `startup.kdl`

## Installation

### AUR

```sh
paru -S niri-autostart
```

Or prebuilt binary

```sh
paru -S niri-autostart-bin
```

### Binary releases

You can download a binary release [here](https://github.com/partanskiy/niri-autostart/releases)

## Configuration

Example:

```kdl
autostart {
    workspace "code" {
        column {
            width {
                proportion 1.0
            }

            window app-id="kitty" {
                command "terminal" "-e" "nvim"
                height {
                    proportion 1.0
                }
            }
        }

        column {
            width {
                proportion 1.0
            }

            window app-id="kitty" {
                command "terminal" "-e" "fish" "-lc" "cl"
                height {
                    proportion 1.0
                }
            }
        }
    }

    workspace "internet" {
        column {
            width {
                proportion 0.65
            }

            window app-id="zen" {
                command "zen-browser"
                height {
                    proportion 1.0
                }
            }
        }

        column {
            width {
                proportion 0.35
            }

            window app-id="org.telegram.desktop" {
                command "telegram-desktop"
                height {
                    proportion 1.0
                }
            }
        }
    }

    workspace "notes" output="HDMI-A-1" {
        column {
            width {
                fixed 960
            }

            window app-id="obsidian" {
                command "obsidian"
                height {
                    proportion 1.0
                }
            }
        }
    }

    workspace "firework" {
        column {
            width {
                proportion 0.33333
            }

            window app-id="kitty" {
                command "terminal" "-e" "fastfetch" "--dynamic-interval" "500" "--hide-cursor" "true"
                height {
                    fixed 284
                }
            }

            window app-id="kitty" {
                command "terminal" "-e" "tty-clock" "-sc"
                height {
                    fixed 207
                }
            }

            window app-id="kitty" {
                command "terminal" "-e" "cava" "-p" "~/.config/cava/themes/noctalia"
                height {
                    fixed 392
                }
            }

            window app-id="kitty" {
                command "terminal" "-e" "cmatrix"
                height {
                    fixed 172
                }
            }
        }
        column {
            width {
                proportion 0.66667
            }

            window app-id="kitty" {
                command "terminal" "-e" "btop"
                height {
                    fixed 661
                }
            }

            window app-id="kitty" {
                command "terminal" "-e" "asciiquarium"
                height {
                    fixed 404
                }
            }
        }
    }

    workspace "scratch" {
        column {
            width {
                fixed 720
            }

            window app-id="kitty" floating=true {
                command "terminal"
                height {
                    proportion 1.0
                }
            }
        }
    }
}
```

This example shows the full schema:

- multiple `workspace` blocks with optional `output` assignment
- `column` width as `fixed` or `proportion`
- `window app-id` as the expected app id after spawn and as a unique-app fallback
- repeated `app-id` values are allowed
- `command` as an argv-style list
- `height` as `fixed` or `proportion`
- optional `floating=true`

For repeated `app-id` values, `niri-autostart` does not require title or class
markers. It records the concrete niri `window_id` that appeared after each `spawn`
in:

```text
/tmp/niri-autostart/windows.json
```

On later runs in the same niri session, live recorded window ids are reused. If the
state file is missing, windows with repeated app ids are spawned again because
there is no reliable way to distinguish unrelated existing windows with the same
app id.

The default config path is:

```text
~/.config/niri-autostart/config.kdl
```

Typical startup entry in `niri`:

```kdl
spawn-at-startup "/home/user/.local/bin/niri-autostart" "--config" "/home/user/.config/niri-autostart/config.kdl"
```

## Status

`niri-autostart` currently works as a `oneshot` startup tool for a declarative multi-workspace layout.

It is focused on one job:

- open missing applications
- reuse existing ones when possible
- move them to the right workspace
- restore tiling geometry
- leave every workspace focused on its first window
