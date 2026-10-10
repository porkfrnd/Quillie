# Project file format

A Quillie project is **one UTF-8 JSON file** with the extension `.quillie`.

One file, not a directory. A project should be copyable, backup-able and
email-able without a manifest, and a project that lives in a folder is a
project that eventually gets half-copied.

## Why JSON and not SQLite

The default, and still correct (ADR-007 is open but JSON is what ships). Reasons
that decided it:

- **A crash must never corrupt it.** A single JSON write is atomic via
  temp-and-rename. A SQLite file that is half-written needs a journal, and the
  journal is another file to get wrong.
- **It is inspectable.** A user can open a project in a text editor and see
  what happened to it. When a project misbehaves, that is worth more than
  query performance nobody needs.
- **It is diffable.** Version control, issue attachments and "what changed?"
  all work for free.
- **Nobody queries it.** The only access pattern is "load it, edit it, save
  it". SQLite's advantage is a query workload Quillie does not have.

The cost is file size for very large projects and parse time on open. Both are
acceptable at the scale where a project is edited by a person, and the migration
path (parse JSON into a SQLite cache) remains available if that ever stops being
true.

## Top level

```json
{
  "schema_version": 1,
  "id": "0a0492be-ffe4-4029-992c-ba952e4b9741",
  "name": "Holiday",
  "frame_rate": { "num": 30000, "den": 1001 },
  "resolution": [1920, 1080],
  "sample_rate": 48000,
  "media": { },
  "sequences": [ ],
  "captions": [ ],
  "markers": [ ],
  "snapshots": [ ],
  "export_presets": [ ]
}
```

`schema_version` is **always first**. A migration reads it before touching
anything else, so a file from a newer build is recognised and refused rather
than partially interpreted.

## Times

Every time is an integer count of **ticks**, where one tick is `1/120000`
second. See [ADR-001](adr/ADR-001-time-base.md).

Times are bare integers in the JSON, not wrapped objects:

```json
"timeline_start": 1200000
```

not

```json
"timeline_start": { "ticks": 1200000 }
```

This is a deliberate readability choice. A human inspecting a project should be
able to see that 1200000 ticks is 10 seconds without a mental conversion. The
`Ticks` newtype serialises transparently, so this costs nothing in Rust.

**No floating-point time anywhere.** A `0.1` in a time field would accumulate
error across a long timeline and would not survive a JSON round trip exactly.

## Frame rates

Stored as an exact rational, never a float:

```json
"frame_rate": { "num": 30000, "den": 1001 }
```

29.97 and 30000/1001 are different numbers. A project at the wrong rate drifts
audibly over an hour, so the difference is preserved exactly.

## Media references

Media is referenced by fingerprint, not by path alone:

```json
"media": {
  "0192...": {
    "path": "footage/a.mov",
    "fingerprint": {
      "size_bytes": 123456789,
      "mtime_unix": 1757000000,
      "head_hash": [1, 2, 3, "..."],
      "tail_hash": [4, 5, 6, "..."]
    },
    "duration": 1800000000,
    "streams": { "video": { "codec": "h264", "width": 3840, "height": 2160 } },
    "rotation": 90,
    "vfr": false
  }
}
```

The path is a convenience. The **fingerprint is the identity**: when the file at
that path no longer matches, the clip shows as missing and offers to relink by
fingerprint. Fingerprinting the whole file would be more correct and would turn
import into a multi-minute stall on a mechanical disk, so the first and last
MiB are hashed instead. See [`crates/quillie-core/src/media/fingerprint.rs`](../crates/quillie-core/src/media/fingerprint.rs).

## Sequences and tracks

A project always has at least one sequence. Additional sequences are aspect
variants that share media and captions.

## Animatable values

```json
"opacity": { "kind": "static", "value": 1.0 }
```

Adjacent tagging, not internal: serde cannot internally tag a newtype variant
containing a bare float, and `Animatable<f32>` is exactly that. The other
variants are:

```json
{ "kind": "keyframed", "value": [ { "time": 0, "value": 0.0, "interp": {...} } ] }
{ "kind": "driven", "value": { "source": {...}, "map": {...} } }
```

## Versioning and migrations

Each schema version is documented here. Migrations are pure functions
`vN -> vN+1` in `quillie-core::project::schema`, and each has a test that loads
a checked-in fixture from the old version.

| Version | Change |
|---|---|
| 1 | Initial format. |

### v1

Initial. Project, sequences, tracks, clips, media, captions, markers, snapshots,
export presets.

Adding a field is backwards compatible: serde's `#[serde(default)]` means an
older file still loads. Removing or renaming one is not, and needs a migration.

`Animatable`, `Interp`, `DataSource` and `Mapping` all ship with their
non-static variants present but empty, so adding keyframes in Phase 4 does not
require a schema bump. That is the point of declaring them early.

## Atomic save

1. Write to `<name>.quillie.tmp` **in the same directory**.
2. `fsync` it.
3. Copy the current file to `<name>.quillie.bak`.
4. `rename` the temp over the target.
5. `fsync` the containing directory.

Same directory is load-bearing: a rename across filesystems is not atomic. The
fsync before the rename is load-bearing too — without it the rename can land
before the data does, leaving an empty file that still parses as valid.

A crash-injection test covers all seven points in this sequence where a crash
can land, asserting that the project always opens and always holds a complete
project. See
[`crates/quillie-app/tests/crash_injection.rs`](../crates/quillie-app/tests/crash_injection.rs).

## Autosave journal

`Project::append_journal` appends one line per op to
`<app-data>/autosave/<project-id>.journal`. Append-only, flushed per call,
because the point is to survive a crash. A crash mid-write loses at most the
last record.

On launch, recovery is offered when the journal is newer than the project file.