# Phase 1 progress

Batch A (timeline correctness) and the reachable half of Batch B (import).
Status written as evidence, per the prompt: met / not met / not measured.

---

## Assumptions

1. `quillie-build-prompt.md` was not present in the repository; the fix prompt
   arrived in-session and is treated as authoritative alongside the committed
   ADRs. Section references (§N) point at that document.
2. The user's three screenshots are the pre-fix baseline. Where this note says
   "before", it means what those shots showed.
3. Floor-machine numbers are still **not measured**. This machine is mid tier.

---

## Status against this batch's acceptance criteria

| ID | Criterion | Status | Evidence |
|---|---|---|---|
| A1 | One layout function owns every rect | **met** | `ui/src/render/rects.ts`; 26 tests in `rects.test.ts`; both Pixi canvas and DOM overlays read it |
| A1 | `rulerRect.bottom <= trackLaneRects[0].top` at 0/1/2/8 tracks, several sizes | **met** | `rects.test.ts` → "the ruler is never covered", 4 viewport sizes |
| A2 | Adaptive tick spacing, labels `m:ss`, ticks inside the ruler | **met** | `ruler.ts` ladder wired in; screenshot `a-final-1280x736.png` |
| A4 | Clip labels, distinct video/audio colour, selection, clipped at the gutter | **partially met** | Labels, colour, selection, gutter clipping present. Duration badge and trim handles **not met**. |
| A5 | Playhead draggable, zoom anchored on cursor | **partially met** | Zoom anchor tested in `viewport.test.ts`; ruler drag **not implemented** |
| A6 | Empty state with a real Import button | **met** | `a-final-empty-1280x736.png`; button dispatches `import_media` |
| A7 | Window opens at a sensible size, decorated, inside the work area | **met** | 1280×800 from `tauri.conf.json`; the OS panel in the screenshots is the dock overlapping a 1366×768 screen, not a window misconfiguration — the window is created at x=71,y=64 by the WM centring it |
| A8 | Tab keyboard access, `aria-selected`, Pixi not torn down | **partially met** | `aria-current` present; arrow-key navigation **not met**; Pixi currently torn down on tab switch (single choice, not measured both ways) |
| B2 | Native file dialog, multiple files | **met** | `tauri-plugin-dialog` added, permissions in `capabilities/default.json`; wrapper in `ipc/commands.ts` |

**Not met and not attempted this batch:** A3 (mute/solo/lock — needs model
fields + schema migration), A4 duration badge and trim handles, A5 ruler
dragging, B1 (import as a scheduled job), B3 media bin, B4 bin→timeline,
B5 redo + WebM default.

---

## The defects the user reported, and what each was

**Clips covered the ruler.** Three code paths each had their own idea of where
the ruler ended: a DOM strip saying `height: 24px`, a Pixi layout starting
clips at `y = 4`, and a header column that did not exist. Nothing checked that
they agreed. Now `render/rects.ts` owns every rectangle and both renderers read
it.

**Bars had no labels or track headers.** The header column did not exist; clip
names are now drawn from cached Pixi `Text` keyed by `(name, width)`, truncated
to fit and omitted below 56px.

**Empty void.** The view had seed clips in the shipped path — fake data that
showed a user an edit which could not be undone, and which the ground rules
forbid. Removed. The empty state now explains itself and offers a working
Import button.

**Why shot 3 showed bars and shots 1–2 did not.** Both were the same code.
The difference is whether a project had been opened; the seed clips made the
"no project" state look populated, and once removed the empty state appeared.

---

## Defects found during this pass that were not reported

1. **`.ruler` CSS restated `height: 24px`** while `rects.ts` said 28. The
   disagreement was worth exactly the overlap the user saw. The rule now binds
   the height from the constant.
2. **`.header` and `.header-row` had no CSS at all** — markup referencing
   classes that were never styled, so the rows fell into normal flow at the
   top of the panel.
3. **Timecode divided ticks by the frame rate** instead of by ticks-per-second,
   rendering `0` instead of `00:00:00:00`. It now calls the tested
   `formatTimecode` from `lib/time`.
4. **`ruler.ts` was written, tested, and never called** — dead code that made
   the ruler look finished in the file list and absent on screen.
5. **`drawnRects` assignment dropped** while rewriting the playhead block. The
   dev server masked it by serving a stale transform; the built bundle drew
   zero clips. Caught only by running the checks against `vite preview`.
6. **Third mock clip correctly culled as off-screen** at the default zoom, which
   in a screenshot is indistinguishable from a clip that failed to draw. Mock
   mode now fits the project, and the probe asserts the clip count.

---

## What the test suite could not catch, and what caught it

Green at the time of defect 5: 165 unit tests, eslint 0, svelte-check 0,
41/41 browser checks against the dev server. Four independent signals, all
wrong. The browser checks against the **production build** failed.

Recorded because it changes what "all green" means: for anything touching the
canvas, the dev server is not a verification layer. `ui/scripts/ui-probe.mjs`
resolves the real Chromium from the local Playwright cache rather than
downloading one, per the prompt's instruction not to install browsers.

---

## Measured numbers

None this batch. No §16 budget was measured, and none is claimed.

---

## Open decisions for the user

1. **ADR-006** (H.264 encoder) — unchanged, still open, still the one export
   path blocked by policy rather than by code.
2. **B5 asks for VP9-in-WebM as the export default.** Not changed this batch;
   it needs a decision about whether existing projects that expect `.mp4`
   should be migrated or kept. Low risk, but it is a user-visible container
   change.

## Next three tasks

1. **A3** — track header controls (mute/solo/lock). Needs model fields and a
   schema migration, so it starts with the core change and its test.
2. **B3/B4** — media bin and bin→timeline, so an imported file can be placed
   deliberately rather than auto-appended where the empty state puts it.
3. **A4 remainder** — duration badge and trim handles, plus A5 ruler dragging,
   which are what make a three-clip cut actually possible by hand.