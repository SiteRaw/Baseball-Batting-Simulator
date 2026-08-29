# Baseball Duel

A single at-bat pitcher vs. batter duel built with **macroquad 0.3** + **egui-macroquad 0.14**.  
You control **both** sides — press `Tab` to switch roles.

---

## Build

```bash
# Requires Rust 1.75+ (MSRV of macroquad 0.3 + egui 0.21)
# On Linux you may need: sudo apt install libasound2-dev libx11-dev libxi-dev
cargo run --release
```

The game runs on Windows, macOS, and Linux.  
Data files are written to `data/` next to the executable on first launch if they don't already exist.

---

## Controls

### Pitching Mode (default)
| Input | Action |
|-------|--------|
| Mouse move | Aim crosshair — see "Aiming" below |
| `Space` / egui "THROW" | Throw pitch |
| Mouse **scroll wheel** | Cycle selected pitch |
| `1`–`9` | Select pitch by number |
| `Tab` | Switch to batting mode |

Mouse click is intentionally *not* the throw trigger — it's freed up so
aiming and scrolling through pitches never accidentally fires a pitch.

### Batting Mode
| Input | Action |
|-------|--------|
| Mouse move | Position PCI (batting cursor) |
| `Space` | Swing |
| `LMB` / egui button | Trigger AI pitch (skips the auto-pitch wait) |
| `Tab` | Switch to pitching mode |

**Auto-pitch:** toggle in the side panel. When on, the AI pitcher throws
automatically every few seconds (adjustable) — no clicking required between
pitches. The windup gives two visual cues: a calm blue "get ready" pulse
early on, crossfading into a sharp multicolor flash right at release.

### Any Mode
| Input | Action |
|-------|--------|
| `Space` / `Enter` / `LMB` | Advance from result screen |
| egui "Reload" button | Reload all data files without restarting |

---

## Aiming (Pitching Mode)

The yellow crosshair shows where a **generic straight fastball** (90 mph,
plain backspin, no side/gyro spin) would cross the plate if released toward
that point. The pitch you've actually selected is thrown along that same
release direction — but its own movement then carries it away from the
reticle, just like a real breaking ball drifts off your intended spot. A
4-seamer stays close to the reticle; a sweeper or curveball will drift
noticeably. You have to aim "into" the break.

---

## Batted Balls

Contact now plays out fully: the ball flies, bounces (with energy loss and
spin bleed-off each impact, up to 4 bounces), and rolls to a stop or clears
the fence. Small dust-puff markers mark each bounce point, and an orange
ring marks where it finally comes to rest.

---

## Batter Contact Traits

Batters can have a natural swing tendency, set per-player in `players.txt`:

- `trait_vert`  = `up` / `down` / `none` — rewards high or low contact (launch angle)
- `trait_horiz` = `pull` / `push` / `none` — rewards early (pull-side) or late (opposite-field) contact
- `trait_bonus` = how strong the resulting power swing is

An "up + pull" hitter gets his power bonus from swinging slightly **early**
and **under** the ball — swinging late and on top of it works against his
natural stroke. The included `batter_ai` is set up as a mild pull hitter by
default; edit `players.txt` to change either batter's profile.

---

## Reach Penalty & Timing-First Contact

This is primarily a **timing** game. Missing the green PCI reticle by a bit
costs less than it used to — contact quality now leans heavily on swing
timing, with PCI accuracy only softly modulating it. Separately, if you
position the PCI **outside the strike zone** (reaching for an off-plate
pitch), you gradually lose power the further out you reach — the reticle
itself tints from green toward orange/red as a visual cue.

---

## Bug Fixes

- **Popups** now show distance in the result text, and the classifier no
  longer labels a well-struck, deep steep fly ball a "Popup" — a real popup
  has to actually be short and/or weakly hit; a towering shot with real
  carry now correctly falls through to the fly-ball / home-run logic.
- Fixed a rare bug where a hard-topped grounder could "sink through the
  ground" in the batted-ball simulation instead of bouncing.
- Fixed the strike zone box being drawn *smaller* than the actual called-
  strike boundary — some pitches that visually missed the box outright were
  being called strikes. The box is now drawn at its true rulebook size; the
  "touches the zone" leniency lives entirely in the decision logic (which
  expands the true zone by one ball radius), never in the box you see.

---

## Home Run Distance

Home runs are no longer capped at the fence distance for their reported
distance — the ball keeps flying under normal physics past the wall until
it actually comes back down, so a real moonshot can post a distance well
north of 450 ft instead of reading as "the fence distance plus a few feet."

---

## Fielder-Aware Hit/Out Classification

Ground balls, liners, and fly balls are no longer decided by exit velocity
and launch angle alone. Each ball's landing angle from home plate is
compared against approximate infield (1B/2B/3B/SS) and outfield (LF/CF/RF)
positions: hit close to a fielder's angle, it's more likely fielded for an
out; hit into a gap between two fielders, it's more likely to sneak through
for a hit. Harder contact and, for fly balls, less hang time both cut into
a fielder's effective range. Out descriptions now name the fielder involved
("Ground out to SS", "Fly out to CF").

---

## Release FX Timing

Windup length now depends on role: pitching mode stays short and snappy
(0.5s) so manual throws never feel sluggish, while batting mode keeps a
full 1s windup — its "get ready" pulse still starts up to a second early,
during the tail of the existing "Next pitch in Xs" auto-pitch countdown, so
the extra length reads as anticipation rather than added wait.

---

## Predicted-Landing Preview (Pitching Mode)

Alongside the yellow "generic fastball" aim reticle, a light, translucent
green marker and short approach trail now show where the pitch you've
actually selected will really cross the plate, given the same release
direction — a quick visual read on how much a breaking ball will drift off
your aim before you commit to the throw.

---

## Strikeout / Walk FX

A brief full-screen flourish plays on a strikeout (fire/embers) or a walk
(ice storm) — purely cosmetic. Disable it in `game.txt`:

```
[fx]
enable_outcome_fx = 0
```

---

## Field Layout

The ground is now properly zoned, concentric from home plate outward:
infield dirt (and mound) sit inside a green outfield that runs out to the
fence, with everything beyond the fence — foul territory corners, behind
the wall — rendered as plain dirt/soil so the fence reads as a clear
boundary. 1st, 2nd, and 3rd base are marked at their real 90-ft spacing with
a faint baseline diamond, making it easier to visualize where a groundout,
liner, or bloop single would land relative to the infield.

---

## Aerial Mini-Map (Batting Mode)

Tracks where the most recent ball in play first landed — drawn on top of
the side panel, not behind it. The dot only appears once the on-field
animation actually reaches that spot (it never shows the result before you
can see it happen), and it stays put through any number of non-BIP pitches
afterward, only getting replaced once the next ball in play similarly lands.

---

## Physics Model

- **Drag + Magnus (Sawicki & Hubbard 2003)** — piecewise Cl model, gyro spin removed before computing lift
- **3-iteration aim solver** — adjusts initial direction so the ball crosses the intended plate location
- **Command noise** — Gaussian scatter applied to aim before sim (configurable per pitcher)
- **Batted ball** — same aerodynamics; EV/LA/spray from a contact quality model

---

## Data Files (`data/`)

All tuneable constants live in plain `key = value` text files.  
Edit them while the game is running, then hit **Reload** in the side panel.

### `pitches.txt`
Defines the pitch arsenal shared by both pitchers.

```
[pitch]
name           = 4-Seam Fastball
key            = FF          # ID used in ai.txt and hotkey list
velocity_mph   = 96
backspin_rpm   = 2150        # + = lift (4-seam ride),  − = drop (curveball)
sidespin_rpm   = -350        # + = moves right (RHP glove-side), − = arm-side run
gyrospin_rpm   = 200         # bullet-spin; reduces movement, adds no lift
```

Included arsenal: 4-Seam, Sinker, Slider, Sweeper, Curveball, Changeup,
Splitter, **Cutter**, **Forkball**, **Knuckle-curve**, and **Knuckleball**.

The knuckleball is a special case: real knuckleball movement comes from
seam-induced turbulence that the Magnus-lift model here doesn't capture. Any
pitch with very low total spin (< ~350 rpm) instead gets a small random
in-flight "flutter" — you can still aim it, but you can't fully control
where it drifts, same as the real pitch.

### `players.txt`
Batter and pitcher attributes.

```
[batter_user]
side      = R         # R or L (handedness)
power_ev  = 107       # max exit velocity (mph) on a perfect contact
pci_rx    = 1.05      # PCI half-width  (feet)
pci_rz    = 0.55      # PCI half-height (feet)

[pitcher_user]
command_sigma = 0.35  # Gaussian noise on pitch location (feet at plate)
```

### `physics.txt`
Ball constants and release point.

### `game.txt`
Strike zone dimensions, batting contact model coefficients, park fence distances,
and view / camera parameters.

### `ai.txt`
AI pitcher pitch-mix weights per count state, zone rates, and AI batter recognition /
timing / PCI noise parameters.

---

## Pitch Movement Chart

The egui panel shows HB (horizontal break) and IVB (induced vertical break) in inches
for each pitch, computed once at startup via a no-gravity control simulation.

- **HB+** = moves toward the catcher's right (RHP glove-side)
- **IVB+** = rises vs. gravity (backspin lift)

---

## Customisation Ideas

- Add new pitches to `pitches.txt` (they appear automatically in the hotkey list and AI weights)
- Tune `game.txt` `time_scale` (< 1.0 = slow-motion) to practice recognition
- Increase `pci_rx` / `pci_rz` in `players.txt` for an easier batting target
- Tighten `ai.txt` `recognition_sigma` to make the AI batter sharper
