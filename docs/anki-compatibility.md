# Anki compatibility spec

Target: Anki 25.x/26.x collections (schema 11 through 18) and AnkiConnect API version 6.
Facts below come from the pinned upstream sources in [Sources](#sources) and from inspecting a read-only copy of `/home/user/.local/share/Anki2/main/collection.anki2`. That local file is schema 18: `col.id=1`, `ver=18`, 351 notes, 372 cards, 41,519 revlog rows, and empty strings in all five JSON columns of `col`.

## 1\. Package files

An `.apkg`/`.colpkg` is a ZIP archive. Anki chooses the collection entry and the media-map format from the `meta` entry. If `meta` is absent, the presence of `collection.anki21` selects legacy 2, otherwise legacy 1.

| Package version | `meta` (protobuf) | collection entry | collection bytes | media entry | media files | media map |
| --- | --- | --- | --- | --- | --- | --- |
| Legacy 1 | absent | `collection.anki2` | no zstd; normal ZIP compression | JSON | `0`, `1`, ... plain | `{"0": "foo.png", "1": "bar.mp3"}` |
| Legacy 2 | absent | `collection.anki21` | no zstd; normal ZIP compression | JSON | numbered, plain | same JSON string map |
| Latest | present, `PackageMetadata{version=3}` | `collection.anki21b` | zstd frame, ZIP entry stored | zstd frame of protobuf `MediaEntries` | numbered, zstd frames, ZIP entries stored | `entries: [{name, size, sha1}, ...]` |

`meta` is `proto/anki/import_export.proto` message `PackageMetadata`: field 1 varint, values `0=UNKNOWN, 1=LEGACY_1, 2=LEGACY_2, 3=LATEST`. Version 3 encodes as bytes `08 03`. The `meta` ZIP entry itself is stored uncompressed. `MediaEntry` uses field 1 `name`, field 2 `size`, field 3 `sha1` (20 raw bytes); field 255 `legacy_zip_filename` is import-only and records gaps in old maps.

Every package also contains a dummy `collection.anki2` (a tiny schema-11 collection with one note) so older clients can open the file and show an "exported by newer Anki" message. Read the entry named by `meta`, never the dummy. Legacy 1/2 packages imply schema 11; latest implies schema 18.

`.apkg` contains the selected notes, their note types, templates, decks, deck configs and referenced media. `.colpkg` contains the whole collection file plus the media folder (when media is requested). Both use the same envelope writer: `meta`, collection, dummy collection, numbered media, `media`. Anki checkpoints WAL and closes the collection before copying it into either package.

## 2\. SQLite collection

Canonical schema 11 DDL (`rslib/src/storage/schema11.sql`). Anki accepts `col.ver` 11 through 18 and upgrades on open.

```sql
CREATE TABLE col (
  id integer PRIMARY KEY, crt integer NOT NULL, mod integer NOT NULL,
  scm integer NOT NULL, ver integer NOT NULL, dty integer NOT NULL,
  usn integer NOT NULL, ls integer NOT NULL,
  conf text NOT NULL, models text NOT NULL, decks text NOT NULL,
  dconf text NOT NULL, tags text NOT NULL);

CREATE TABLE notes (
  id integer PRIMARY KEY, guid text NOT NULL, mid integer NOT NULL,
  mod integer NOT NULL, usn integer NOT NULL, tags text NOT NULL,
  flds text NOT NULL,
  -- integer affinity is deliberate: integer sfld values sort numerically
  sfld integer NOT NULL, csum integer NOT NULL,
  flags integer NOT NULL, data text NOT NULL);

CREATE TABLE cards (
  id integer PRIMARY KEY, nid integer NOT NULL, did integer NOT NULL,
  ord integer NOT NULL, mod integer NOT NULL, usn integer NOT NULL,
  type integer NOT NULL, queue integer NOT NULL, due integer NOT NULL,
  ivl integer NOT NULL, factor integer NOT NULL, reps integer NOT NULL,
  lapses integer NOT NULL, left integer NOT NULL, odue integer NOT NULL,
  odid integer NOT NULL, flags integer NOT NULL, data text NOT NULL);

CREATE TABLE revlog (
  id integer PRIMARY KEY, cid integer NOT NULL, usn integer NOT NULL,
  ease integer NOT NULL, ivl integer NOT NULL, lastIvl integer NOT NULL,
  factor integer NOT NULL, time integer NOT NULL, type integer NOT NULL);

CREATE TABLE graves (usn integer NOT NULL, oid integer NOT NULL, type integer NOT NULL);

CREATE INDEX ix_notes_usn ON notes (usn);
CREATE INDEX ix_cards_usn ON cards (usn);
CREATE INDEX ix_revlog_usn ON revlog (usn);
CREATE INDEX ix_cards_nid ON cards (nid);
CREATE INDEX ix_cards_sched ON cards (did, queue, due);
CREATE INDEX ix_revlog_cid ON revlog (cid);
CREATE INDEX ix_notes_csum ON notes (csum);
```

Schema 18 keeps the five tables above unchanged except `graves`:

```sql
CREATE TABLE graves (
  oid integer NOT NULL, type integer NOT NULL, usn integer NOT NULL,
  PRIMARY KEY (oid, type)) WITHOUT ROWID;
CREATE INDEX idx_graves_pending ON graves (usn);
CREATE INDEX idx_cards_odid ON cards (odid) WHERE odid != 0;
```

Schema 18 also adds normalized config tables and indexes; all five `col` JSON columns are empty strings in the observed schema-18 file. Tables: `config(KEY text PRIMARY KEY, usn, mtime_secs, val blob) WITHOUT ROWID`, `notetypes(id PRIMARY KEY, name, mtime_secs, usn, config blob)`, `fields(ntid, ord, name, config blob, PRIMARY KEY(ntid,ord)) WITHOUT ROWID`, `templates(ntid, ord, name, mtime_secs, usn, config blob, PRIMARY KEY(ntid,ord)) WITHOUT ROWID`, `decks(id PRIMARY KEY, name, mtime_secs, usn, common blob, kind blob)`, `deck_config(id PRIMARY KEY, name, mtime_secs, usn, config blob)`, `tags(tag PRIMARY KEY, usn, collapsed boolean, config blob) WITHOUT ROWID`. Names use `COLLATE unicase`; notetype, deck, preset, field and template blobs are protobuf; `config.val` holds serialized JSON. Indexes: `idx_decks_name` unique, `idx_fields_name_ntid` unique, `idx_notetypes_name` unique, `idx_notetypes_usn`, `idx_templates_name_ntid` unique, `idx_templates_usn`. A Rust reader must register a `unicase` collation (casefold comparison) before querying these tables.

### 2.1 `col` JSON keys to preserve (schema 11)

`conf`: `activeDecks`, `curDeck`, `newSpread`, `collapseTime`, `timeLim`, `estTimes`, `dueCounts`, `curModel`, `nextPos`, `sortType`, `sortBackwards`, `addToCur`, `dayLearnFirst`, `schedVer`, `creationOffset`, `sched2021`. Anki also reads optional keys such as `lastUnburied`; unknown keys are migrated into the `config` table at schema 14, so round-trip them.

`models`: object keyed by note type id. Per type: `id`, `name`, `type` (0 standard, 1 cloze), `mod`, `usn`, `sortf`, `did`, `flds`, `tmpls`, `css`, `latexPre`, `latexPost`, `latexsvg`, `req`, `originalId`, `originalStockKind`. Field keys: `name`, `ord`, `sticky`, `rtl`, `plainText`, `font`, `size`, `collapsed`, `description`, `excludeFromSearch`, `id`, `tag`, `preventDeletion`. Template keys: `name`, `ord`, `qfmt`, `afmt`, `bqfmt`, `bafmt`, `did`, `bfont`, `bsize`, `id`. Older files also contain `vers` and model-level `tags`.

`decks`: object keyed by deck id. Keys: `id`, `name`, `conf`, `desc`, `lrnToday`, `revToday`, `newToday`, `timeToday`, `collapsed`, `browserCollapsed`, `dyn`, `usn`, `mod`, `extendNew`, `extendRev`, plus `desiredRetention`, `reviewLimit`, `newLimit`, `reviewLimitToday`, `newLimitToday`; filtered decks add `terms`, `return`, `reschedule`, `previewDelay`.

`dconf`: object keyed by preset id. Keys: `id`, `name`, `mod`, `usn`, `maxTaken`, `autoplay`, `timer`, `replayq`, `dyn`, `newMix`, `interdayLearningMix`, `newPerDayMinimum`, `newSortOrder`, `reviewOrder`, `newGatherPriority`, `buryInterdayLearning`, `fsrsWeights`, `fsrsParams5`, `fsrsParams6`, `desiredRetention`, `sm2Retention`, `paramSearch` variants (`weightSearch`), `stopTimerOnAnswer`, `secondsToShowQuestion`, `secondsToShowAnswer`, `questionAction`, `answerAction`, `waitForAudio`, `ignoreRevlogsBeforeDate`, `easyDaysPercentages`. Nested `new`: `order`, `delays`, `bury`, `perDay`, `initialFactor`, `ints`. Nested `rev`: `maxIvl`, `hardFactor`, `ease4`, `ivlFct`, `perDay`, `bury`. Nested `lapse`: `leechFails`, `mult`, `leechAction`, `delays`, `minInt`.

`tags`: object mapping tag name to `{"usn": int, "collapsed": bool}`. Hierarchical tags use `::`.

## 3\. Row encodings

`notes.flds`: fields joined by U+001F. Split on `'\x1f'`; do not trim. Empty trailing fields are significant.

`notes.tags`: space-delimited with a leading and trailing space, for example `" Asia Europe "`. Anki splits on ASCII space and U+3000 and drops empty tokens. Tag comparison is case insensitive with case preservation from the `tags` table.

`notes.guid`: 1 to 11 chars from the 91-character alphabet ``a-zA-Z0-9!#$%&()*+,-./:;<=>?@[]^_`{|}~``. New notes use `anki_base91(random u64)`: repeated division by 91, digits reversed, zero maps to the empty string. The GUID is the identity used by import; never regenerate it on re-import.

`notes.csum`: first four bytes of SHA-1 of HTML-stripped field 0, interpreted big-endian as `u32`. Anki strips tags while preserving media filenames before hashing. Example from `rslib/src/notes/mod.rs`: `field_checksum("test") == 2840236005`.

`notes.sfld`: HTML-stripped text of field `sortf` (field 0 when `sortf == 0`). The column has integer affinity, so numeric strings sort numerically in SQLite.

`notes.flags`: bits 1 to 7 are flag colors. `notes.data` is an empty string or JSON for add-ons.

`cards.type`: 0 new, 1 learning, 2 review, 3 relearning. `cards.queue`: 0 new, 1 learning (intraday), 2 review, 3 day learning/relearning, 4 preview repeat, -1 suspended, -2 sibling buried, -3 manually buried. Read queue first; `type` alone does not tell whether a card is due.

`cards.due`: queue 0 is an insertion position; queue 1 and 4 are Unix seconds; queue 2 and 3 are days since `col.crt` (day rollover is configurable, default 04:00). Negative queues have unused `due`. `cards.odue`/`odid` hold original due and deck while a card is in a filtered deck.

`cards.left`: remaining learning/relearning steps. Current Anki uses the raw count and reads `left % 1000`; older clients add 1000 to mark "remaining today". `cards.ivl`: days for review cards, negative seconds for learning intervals. `cards.factor`: ease percent times 10, for example 2500. `cards.data`: JSON with optional `pos` (original new-card position), `s` (FSRS stability, 4 decimals), `d` (difficulty, 3 decimals), `dr` (desired retention, 2 decimals), `decay` (positive decay, 3 decimals), `lrt` (last review Unix seconds), `cd` (add-on JSON string). Observed local value: `{"pos":205,"s":75.9547,"d":7.13,"dr":0.9,"decay":0.5,"lrt":1794656149}`.

`revlog`: `id` is the answer time in Unix milliseconds. `ease` is the button 1 to 4, 0 for manual rescheduling. `ivl`/`lastIvl` are positive days or negative seconds. `time` is milliseconds taken. `factor` is ease percent times 10 for SM-2, or FSRS difficulty normalized to 100 to 1100. `type`: 0 learning, 1 review, 2 relearning, 3 filtered/cram, 4 manual, 5 rescheduled. `graves.type`: 0 card, 1 note, 2 deck.

Timestamps: `col.crt`, `notes.mod`, `cards.mod` and due day counts use seconds. Revlog `id` is Unix milliseconds. `col.mod`, `col.scm`, `col.ls` are milliseconds.

## 4\. Learning steps and scheduling constants

Defaults from `deckconfig/schema11.rs` and `scheduler/states/mod.rs`: learning steps `[1, 10]` minutes, relearning steps `[10]` minutes, graduating intervals good 1 day and easy 4 days, starting ease 2.5, hard multiplier 1.2, easy multiplier 1.3, interval modifier 1.0, maximum interval 36500 days, leech threshold 8 lapses, 20 new cards and 200 reviews per day.

Learning answers (`scheduler/states/steps.rs`): Again returns to step 0 (600 s with defaults). Hard on step 0 is the mean of step 0 and step 1 (330 s), or 1.5 times a single step capped at one day beyond it. Good advances one step. Step delays over one day are rounded to whole days. A card whose next delay crosses the day rollover moves from queue 1 to queue 3 and its due becomes a day number.

SM-2 review intervals (`scheduler/states/review.rs`): initial ease 2.5, floor 1.3; Again subtracts 0.2, Hard subtracts 0.15, Easy adds 0.15. On-time Hard is `max(1, interval * hard_multiplier)` and at least previous interval plus 1; Good is `(interval + days_late/2) * ease`; Easy is `(interval + days_late) * ease * easy_multiplier`. Early reviews use elapsed days instead of the scheduled interval. Interval fuzz ranges are `2.5-7 days: 15%`, `7-20: 10%`, `>20: 5%`, seeded by card id and reps. Leech detection triggers at the threshold and every half threshold after it.

## 5\. AnkiConnect subset (API version 6)

HTTP POST to `http://127.0.0.1:8765` with `Content-Type: application/json`. Request: `{"action": "...", "version": 6, "params": {...}}`; `params` defaults to `{}`. An optional `key` must match the configured apiKey. Reply: `{"result": <value>, "error": null}`, or `{"result": null, "error": "<message>"}`. Version 4 and lower return the bare result.

| Action | Params | Result |
| --- | --- | --- |
| `version` | none | `6` |
| `deckNames` | none | `string[]` |
| `modelNames` | none | `string[]` |
| `modelFieldNames` | `{"modelName":"Basic"}` | `string[]` in field order |
| `addNote` | `{"note":{...}}` | new note id (`int64`) |
| `findCards` | `{"query":"deck:Default"}` | `int64[]`, Anki search syntax |
| `cardsInfo` | `{"cards":[id,...]}` | array of card objects, `{}` for a missing id |
| `answerCards` | `{"answers":[{"cardId":id,"ease":3},...]}` | `boolean[]`, true when the card existed |
| `changeDeck` | `{"cards":[id,...],"deck":"Target"}` | `null`; removes cards from filtered decks first |
| `suspend` | `{"cards":[id,...],"suspend":true}` | `boolean` |
| `unsuspend` | `{"cards":[id,...]}` | `null` |
| `createDeck` | `{"deck":"A::B"}` | deck id (`int64`) |

`addNote.note` fields: `deckName`, `modelName`, `fields` (name to HTML string, matched case insensitively), optional `tags` (`string[]`), optional `options` (`allowDuplicate`, `duplicateScope`, `duplicateScopeOptions` with `deckName`, `checkChildren`, `checkAllModels`), and optional `audio`/`video`/`picture` media objects. Empty questions or duplicate checks raise an error.

`cardsInfo` objects contain `cardId`, `fields` (field name to `{"value": html, "order": n}`), `fieldOrder`, `question`, `answer`, `modelName`, `ord`, `deckName`, `css`, `factor`, `interval`, `note`, `type`, `queue`, `due`, `reps`, `lapses`, `left`, `mod`. `suspend` sets queue -1; `unsuspend` restores the queue derived from card type. `answerCards` uses ease 1 (Again) through 4 (Easy) and requires a started card timer in Anki's implementation.

## 6\. Scheduling algorithms

Anki uses the Rust `fsrs` crate, version `6.6.2` at the pinned revision. It accepts 17 parameters (FSRS-4.5, converted to 21), 19 (FSRS-5, decay 0.5 appended), 21 (FSRS-6), or an empty list (defaults). Defaults: `[0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542]`. The last value is the positive decay stored on the card; the engine uses `w20 = -decay`. FSRS-5 stores `0.5`.

Let `S` be stability in days, `D` difficulty in 1 to 10, `r` rating 1 to 4, `t` elapsed days, and `d` the positive decay. Clamp `S` to 0.001 to 36500 and `D` to 1 to 10.

```text
F = 0.9^(-1/d) - 1
R(t) = (1 + F*t/S)^(-d)
interval(S, DR) = S/F * (DR^(-1/d) - 1), DR in (0,1)

S0(r) = w[r-1]
D0(r) = w4 - exp(w5*(r-1)) + 1
Dnext = w7*(D0(4) - (D + (10-D)*(-w6*(r-3))/9)) + (D + (10-D)*(-w6*(r-3))/9)

Ssuccess = S*(w8 * (11-D) * S^(-w9) * (exp((1-R)*w10)-1)
               * [w15 if r=2 else 1] * [w16 if r=4 else 1] + 1)
Sfailure = min(w11 * D^(-w12) * ((S+1)^w13 - 1) * exp((1-R)*w14),
               S / exp(w17*w18))
Sshort(t=0) = S * exp(w17*(r-3+w18)) * S^(-w19)  (factor maxed at 1 when r>=2)
```

First review of a card initializes S and D directly. FSRS-5 adds `w17,w18` for short-term stability over FSRS-4.5. FSRS-6 adds `w19` (short-term decay) and `w20` (variable decay). Stability and difficulty, desired retention, decay and last review time are stored per card in `cards.data`. Anki computes the next four states, then maps them to the usual type/queue/due fields. With FSRS disabled, Anki uses the SM-2 variant in section 4.

## 7\. Pitfalls

- Import identity is `notes.guid`, not `notes.id`. An existing GUID updates the note when its mtime is newer, otherwise it is a duplicate. A conflicting note type is skipped. For a new note whose id is already used, Anki adds 999 until free; cards use the same `+999` rule. Re-map `nid`/`did` and original due dates when moving between collections.
- Duplicate detection for notes uses `(mid, csum)`, so identical first fields collapse unless duplicates are allowed.
- `usn` is `-1` for local-only edits. Sync servers use `col.usn` and increment it; changed rows and `graves` rows carry the new value. Deletions without a grave row can resurrect on sync.
- Write `mod`/`mtime` in the right unit and preserve incoming note mtimes on import. `col.mod`, `col.scm`, `col.ls` are milliseconds; note/card `mod` are seconds.
- Anki opens collections with `PRAGMA journal_mode=wal`, `locking_mode=exclusive`, `page_size=4096`, `busy_timeout=0`. Copying only the main file while a writer is active can lose data. Use `PRAGMA wal_checkpoint(TRUNCATE)` with no writer, or copy `.anki2`, `-wal` and `-shm` together.
- Media names in the numbered ZIP entries are the numbers, not filenames. Use the media map. Normalize names to NFC, remove `[ ] < > : " / ? * ^ \\ |`, ASCII controls and unassigned codepoints, replace NBSP with space, prefix Windows device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) with `_`, append `_` to trailing space/dot, and truncate to 120 bytes. Hash raw bytes with SHA-1 for the latest media map.
- On schema 18, writing config JSON into `col` has no effect. Update the normalized tables instead. On schema 11, dropping unknown JSON keys loses add-on settings.
- `due` for review cards is relative to `col.crt`. Do not compare a review `due` with Unix time. A learning card due after rollover belongs to queue 3 with a day-based due.
- `col.ver` 11 is required for maximum client compatibility; latest packages downgrade the schema on export.

## Sources

Anki, pinned commit `be56085c32a9d02b506fa880cfa63c756c1779fc`. Prefix: `https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/`

- `rslib/src/storage/schema11.sql`
- `rslib/src/storage/upgrades/mod.rs`, `upgrades/schema18_upgrade.sql`
- `rslib/src/storage/sqlite.rs`, `storage/collection_timestamps.rs`, `storage/sync.rs`
- `rslib/src/storage/note/mod.rs`, `storage/card/mod.rs`, `storage/card/data.rs`
- `rslib/src/notes/mod.rs`, `rslib/src/tags/mod.rs`, `rslib/src/media/files.rs`
- `rslib/src/import_export/package/meta.rs`, `package/colpkg/export.rs`, `package/apkg/export.rs`, `package/apkg/import/notes.rs`, `package/apkg/import/cards.rs`
- `proto/anki/import_export.proto`, `proto/anki/cards.proto`
- `rslib/src/card/mod.rs`, `rslib/src/revlog/mod.rs`
- `rslib/src/scheduler/answering/learning.rs`, `scheduler/states/steps.rs`, `scheduler/states/review.rs`, `scheduler/states/fuzz.rs`, `scheduler/fsrs/memory_state.rs`
- `rslib/src/config/schema11.rs`, `deckconfig/schema11.rs`, `decks/schema11.rs`, `notetype/schema11.rs`

FSRS-rs v6.6.2: `https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/model.rs` and `https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/inference.rs`. Algorithm wiki: `https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm`.

AnkiConnect 23.10.29.0: `https://github.com/FooSoft/anki-connect/blob/23.10.29.0/plugin/__init__.py`, `plugin/web.py`, `README.md`. The repository moved to `https://git.sr.ht/~foosoft/anki-connect`.
