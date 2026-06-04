# niri-autostart v1 Implementation Plan

## Runtime Model

- `niri-autostart` is a `oneshot` binary in v1.
- It uses two IPC sockets:
  - one command socket for `Request::*` and `Action::*`
  - one event socket for `Request::EventStream`
- It does not use `sleep`.
- It advances only by observing `niri` state transitions coming from the event stream.

## IPC Types

The implementation uses `niri-ipc = "=26.4.0"` directly and treats its IPC types as canonical:

- `Request`
- `Response`
- `Event`
- `Action`
- `Output`
- `Workspace`
- `Window`
- `WindowLayout`
- `SizeChange`

`event-stream` is decoded directly back into `niri_ipc::Event` values.

## Config Model

The config is separate from `niri` config and is parsed with `knuffel`.

Root shape:

```kdl
autostart {
    output "HDMI-A-1" {
        workspace "firework" {
            column {
                width {
                    proportion 0.33333
                }
                window app-id="kitty" floating=true {
                    command "terminal" "-e" "fastfetch"
                    height {
                        fixed 284
                    }
                }
            }
        }
    }
}
```

Rules in v1:

- exact `app-id` fallback only for app ids that are unique in the config
- repeated app ids are allowed
- repeated app ids are identified by runtime state after `spawn`, not by title/class markers
- no regex matching
- no PID matching as the primary identity
- no include files

Default config path:

- `~/.config/niri-autostart/config.kdl`
- overridable via `--config`

Runtime state path:

- `/tmp/niri-autostart/windows.json`
- overridable via `--state`

## State and Reduction

The runtime state keeps:

- `outputs: HashMap<String, niri_ipc::Output>`
- `workspaces: HashMap<u64, niri_ipc::Workspace>`
- `windows: HashMap<u64, niri_ipc::Window>`
- derived indices:
  - workspace name to id
  - app-id to window ids
  - `(workspace_id, column, row)` to window id
- last `ConfigLoaded` status

The persisted runtime state keeps, per config position:

- workspace name
- column index
- row index
- niri `window_id`
- observed `app_id`
- observed PID, when niri provides one
- command argv from the config

Reducer behavior:

- `WorkspacesChanged` replaces the full workspace map
- `WindowsChanged` replaces the full window map
- patch events mutate only the affected records
- `WindowLayoutsChanged` updates stored layouts in place

## Reconcile Flow

Bootstrap:

1. Connect the event socket.
2. Start `Request::EventStream`.
3. Spawn one blocking reader thread.
4. Collect initial full state from `WorkspacesChanged` and `WindowsChanged`.
5. Query outputs separately via `Request::Outputs`.
6. Start reconcile.

Reconcile order:

1. outputs in config order
2. workspaces in config order
3. columns left to right
4. windows top to bottom

Window handling:

- if a live recorded `window_id` exists for the config position, use it
- otherwise, if the app id is unique in the config, fall back to exact `app-id`
- otherwise, spawn and wait for a new window with the expected `app_id`
- after spawn, record the resulting `window_id` in runtime state
- if it exists on another workspace, move it
- first window of a column is treated as the anchor of that column
- later windows are merged into the column via `ConsumeWindowIntoColumn`
- final width/height normalization happens after the whole column is assembled

Extra windows:

- ignored in v1
- only windows declared in config are managed

## Important Limitation

`niri` does not expose `OutputsChanged` in `event-stream`, so outputs must be refreshed through explicit `Request::Outputs` calls before geometry-sensitive steps.
