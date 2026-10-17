# Navigation

Every screen is a variant of `Route` in `src/app.rs`, and how it arrives is
declared on that variant with `#[transition(..)]`
([g3-route-transitions](https://github.com/g3techhq/g3-route-transitions)).
Components never contain animation code: they call `animated_navigate(route)`
and the route decides whether that is a push, a sheet or a cross-fade.

## The shell

`AppShell` (`components/shell.rs`) is the one layout: the page in a
`RouteTransitionPageRegion`, g3-ui's `AdaptiveNav` (bottom tabs on a phone, a rail on
a wide screen), and the persistent player, which lives here so the media
element survives navigation. `PageHeader` is each page's header: Back (or the
brand), the title, the page's toolbar, and the Queue, History and Settings
buttons.

## Layers

| Layer | Routes | Motion |
| --- | --- | --- |
| `stack_root` | `Feed`, `Subscriptions`, `Playlists`, `Explore` (the tabs) | cross-fade between tabs |
| `stack_page` | `PlaylistDetail`, `ChannelDetail` | push from the right (iOS) or shared axis (Material) |
| `sheet` | `VideoDetail`, `QueuePage`, `HistoryPage`, `SettingsPage` | rises over the page |

Every route is `#[public]`: a first visit signs itself in as a guest from
whichever page it opened (see [authentication.md](authentication.md)). The
catch-all redirect sends an unknown path to the feed.

## Modifiers in use

| Modifier | Where, and why |
| --- | --- |
| `forward_to = (HistoryPage, SettingsPage)` | Queue, History and Settings are peers: each header offers the other two, so a move between them slides in the order the header lists them. |
| `history = replace` on `VideoDetail` | A new video in a run replaces the watch entry, so Back dismisses the player to the page beneath instead of walking through every video autoplay visited. |
| `handoff_from = (QueuePage, HistoryPage)` | A video opened from Queue or History takes that sheet's place in history, so minimizing lands on the page beneath both. |

## Rules

- Navigate with `animated_navigate(route)`; go back with the header's
  `BackButton`, which pops and falls back to the page's `back_to` route for a
  deep link with no history.
- Build `Route` variants; never parse a path string back into one.
- Per-screen filters that should survive leaving the screen live in
  `AppState` (`feed_filter`, `explore_filter`, `channel_tab`); ones that belong
  to one visit are local signals.
- The appearance (Material or iOS) drives both g3-ui's look and the transition
  platform; change it through settings, never one alone.
