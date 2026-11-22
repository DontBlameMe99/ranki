# Anki card type compatibility research

Scope: concrete spec for Ranki to support Basic, reversed, Cloze and built-in
Image Occlusion cards. Evidence: Anki source at pinned commit
`be56085c32a9d02b506fa880cfa63c756c1779fc` (unless another commit is named),
the Anki manual, and a read-only copy of the local collection at
`~/.local/share/Anki2/main/collection.anki2`. The copy was made under `.tmp/`
and queried with `mode=ro`; the original was not modified. The local collection
is schema 18: `col.models` is an empty string, so note types were decoded from
the normalized `notetypes`, `fields`, and `templates` tables. It contains 351
notes, 372 cards, and no Image Occlusion note.

Ranki code read: `crates/model/src/lib.rs`, `crates/ui/src/card_view.rs`,
`crates/ui/src/app.rs`, `crates/storage/src/schema.rs`,
`crates/storage/src/records.rs`, `crates/format/src/merge.rs`, and
`docs/anki-compatibility.md`.

## 1. How Anki represents note types and cards

A note type is stored in `col.models` as a JSON object keyed by model id on
schema 11. On schema 18 the JSON columns are empty and the data lives in the
protobuf `notetypes`, `fields`, and `templates` tables. Anki accepts both.
Ranki already reads both paths in `storage::records::{models_from_json,
models_from_tables}`, but `format::merge::read_col` replaces an empty schema-18
`col.models` with stubs `{"id":..,"name":..,"did":1,"flds":[],"tmpls":[]}`.
That is the direct cause of imported cards with no template. Fix the import
path to build full model JSON from normalized tables before merging. See
`docs/anki-compatibility.md` section 2.1 and `crates/format/src/merge.rs:218`.

Cards are independent SQLite rows. `card.nid` points to the note, `card.ord`
selects a template (standard) or a cloze number minus one (cloze), and
`type`, `queue`, `due`, `ivl`, etc. schedule that row. Scheduling does not know
the note type. One note with two templates has two cards with the same `nid`
and `ord` 0 and 1. Each has its own review history. Card generation happens on
note add/edit, not at review. Anki uses `(mid, csum)` for duplicate checks and
`notes.guid` as import identity; see `docs/anki-compatibility.md` sections 3
and 7.

## 2. Exact stock note types

### Basic

Fields, in order: `Front`, `Back`. One template, `ord` 0, name `Card 1`:

```text
qfmt: {{Front}}
afmt: {{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}
```

Local model `1735253330886` (name `Basic`) matches this exactly. The collection
has 126 cards with `mid=1735253330886`, all `ord=0`. Local model ids are
collection-specific; never hardcode them.

Source: `rslib/src/notetype/stock.rs` (`basic()`), local collection.

### Basic (and reversed card)

Fields: `Front`, `Back`. Two templates, `ord` 0 and 1, named `Card 1` and
`Card 2`:

```text
ord 0 qfmt: {{Front}}
ord 0 afmt: {{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}
ord 1 qfmt: {{Back}}
ord 1 afmt: {{FrontSide}}\n\n<hr id=answer>\n\n{{Front}}
```

Local model `1735253283095` matches this. Its 42 cards are split 21 at `ord=0`
and 21 at `ord=1`. `Basic (optional reversed card)` adds field `Add Reverse`
and wraps template 1 qfmt in `{{#Add Reverse}}{{Back}}{{/Add Reverse}}`.

Source: `rslib/src/notetype/stock.rs` (`basic_forward_reverse()`,
`basic_optional_reverse()`), local collection.

### Cloze

Model `type`/kind is `1` (Cloze). Fields: `Text`, `Back Extra`. Field tags mark
`Text` as the clozable field and `Back Extra` as extra. One template, named
`Cloze`:

```text
qfmt: {{cloze:Text}}
afmt: {{cloze:Text}}<br>\n{{Back Extra}}
```

Local model `1735253341970` matches this, with zero notes. A genanki variant
(`1550428389`) uses the same fields and formats.

Cloze syntax in a field value: `{{c1::answer}}`, `{{c1::answer::hint}}`,
`{{c1,2::shared}}` (one deletion on two cards), nested clozes, and MathJax with
`}}` inside it. `c0` is ignored. Hints are displayed in square brackets on the
question: `[...]` without a hint, `[hint]` with one. On the answer, the active
cloze is shown as plain text (or a highlight span for image occlusion);
inactive clozes are shown in full. If the card ordinal has no matching cloze,
Anki renders the cloze filter as empty.

Card generation scans all note fields for distinct positive cloze numbers.
For each number `n`, it creates one card with `ord = min(n - 1, 499)`. The
renderer uses `context.card_ord + 1`, so `ord=0` is `c1`. Cloze note types
always use the first template regardless of `ord`. Anki never creates a cloze
card for `c0`. Sources: `rslib/src/cloze.rs`, `rslib/src/notetype/cardgen.rs`,
`rslib/src/notetype/mod.rs`, `rslib/src/template.rs`,
`https://docs.ankiweb.net/manual/templates/generation`.

## 3. Built-in Image Occlusion (Anki 23+)

Source: `rslib/src/image_occlusion/notetype.rs`,
`rslib/src/image_occlusion/imageocclusion.rs`,
`rslib/src/image_occlusion/imagedata.rs`, `rslib/src/cloze.rs`,
`ts/routes/image-occlusion/shapes/to-cloze.ts`,
`ts/routes/image-occlusion/review.ts`, and `rslib/src/image_occlusion/notetype.css`.
All at `https://github.com/ankitects/anki/tree/be56085c32a9d02b506fa880cfa63c756c1779fc/`.

The stock type is localized as `Image Occlusion`, kind Cloze, with
`original_stock_kind` ImageOcclusion. Fields in order: `Occlusion`, `Image`,
`Header`, `Back Extra`, `Comments`, with tags `Occlusions=0`, `Image=1`,
`Header=2`, `BackExtra=3`, `Comments=4`. The editor locates fields by tag and
falls back to positions 0..3 when tags are absent and there are at least four
fields. Do not identify the type by the localized name alone. There is no such
note type in the local collection, so all IO details below are source-based.

The `Image` field holds plain HTML such as `<img src="diagram.png">`; the image
bytes live in `collection.media`. The `Occlusion` field does **not** hold JSON.
It holds a sequence of cloze markers, one per shape, joined with `<br>`:

```text
{{c1::image-occlusion:rect:left=.2325:top=.3261:width=.202:height=.0975}}<br>
{{c2::image-occlusion:ellipse:left=.1:top=.2:width=.3:height=.4:rx=.15:ry=.2}}<br>
{{c3::image-occlusion:polygon:points=0,0 10,10 20,0}}<br>
{{c4::image-occlusion:text:text=label:fs=14:scale=1:angle=0}}
```

Grammar: `image-occlusion:` followed by shape (`rect`, `ellipse`, `polygon`,
`text`) then colon-separated `key=value` pairs. Backslash escapes `\` and `:`.
Recognized keys: `left`, `top`, `angle`, `fill`, `width`, `height`, `rx`, `ry`,
`points` (`x,y x,y` pairs), `oi` (occlude inactive, `"1"`), `text`, `scale`,
`fs` (font size). `get_image_cloze_data` turns these into `data-shape`,
`data-left`, `data-points`, `data-occludeInactive`, `data-font-size`, etc.
Unknown properties are ignored on re-save, so preserve the raw field string.
Coordinates are emitted as normalized fractions by `toNormal` and converted
back to absolute canvas pixels by `toAbsolute`; treat them as image-relative
floats, not pixels. `{{c1,2::...}}` associates one shape with several cards.

The single template is:

```text
qfmt:
{{#Header}}<div>{{Header}}</div>{{/Header}}\n<div style="display: none">{{cloze:Occlusion}}</div>\n<div id="err"></div>
<div id="image-occlusion-container">\n    {{Image}}\n    <canvas id="image-occlusion-canvas"></canvas>\n</div>\n<script>
try { anki.imageOcclusion.setup(); }
catch (exc) { document.getElementById("err").innerHTML = `<error><br><br>${exc}`; }
</script>
afmt: <qfmt>
<div><button id="toggle">Toggle masks</button></div>
{{#Back Extra}}<div>{{Back Extra}}</div>{{/Back Extra}}
```

The hidden cloze div emits mask `div`s: question + active gives `class="cloze"`
(occluding mask), answer + active gives `class="cloze-highlight"`, inactive
gives `class="cloze-inactive"`, and `c0` always gives a mask. The reviewer's
canvas draws active masks red, inactive masks yellow only when `oi=1`, and
highlight masks as a red outline; `Toggle masks` is shown only when `oi=1`.
CSS variables on `#image-occlusion-canvas` set the colors.

To keep Anki compatibility on export, preserve exactly: the `Occlusion` field
string including ordinals, escaping and `<br>`; the `Image` field `<img src>`;
all five field names and tags; model kind Cloze and `original_stock_kind`;
template qfmt/afmt including `{{cloze:Occlusion}}`, the image container, the
canvas id and the `anki.imageOcclusion.setup()` call; and the CSS variables.
Ambiguity: the cloze string has no version marker and names are localized. The
field tags / `original_stock_kind` are the reliable identification signals.

## 4. Rendering semantics Ranki must match

`{{FrontSide}}` is replaced by the fully rendered question HTML when rendering
the answer. It does not replay front-side audio. Sources:
`rslib/src/template.rs`, `https://docs.ankiweb.net/manual/templates/fields`.

Field replacement in Anki is case-sensitive (`{{Front}}` differs from
`{{front}}`). Ranki's `substitute_fields` currently matches case-insensitively;
keep that for backward compatibility but do not rely on it for new templates.
Anki injects special fields `Tags`, `Type`, `Deck`, `Subdeck`, `Card`,
`CardFlag`, `CardID`, and `c<N>`; `{{Card}}` is the template name and `{{c1}}`
is a cloze-conditional key. Source: `rslib/src/notetype/render.rs`.

Conditionals: `{{#Field}}...{{/Field}}` renders children when the field is
nonempty; `{{^Field}}...{{/Field}}` when it is empty. Whitespace and empty
`<br>`/`<div>` tags count as empty. An unknown key is a template error.
Nested blocks and `{{#c1}}` / `{{^c1}}` cloze conditionals are supported.
Source: `rslib/src/template.rs`.

`{{cloze:Text}}` applies the `cloze` filter to the whole field value with the
card's ordinal. On the question it emits `<span class="cloze" ...>[...]</span>`
or `[hint]`; on the answer it reveals the answer inside a `cloze` span.
Inactive clozes get `class="cloze-inactive"`. Multi-ordinal clozes carry
`data-ordinal="1,2"`. Source: `rslib/src/cloze.rs`.

`{{Image}}` is ordinary field replacement. The resulting `<img src="name">`
is resolved against `collection.media` by the browser; Ranki already extracts
and decodes such references in `card_view::media_refs`. `[sound:name.mp3]` is a
text reference in field HTML that Anki's reviewer extracts and plays; it is
not converted to an `<audio>` tag by the template engine. FrontSide does not
duplicate it. Sources: `https://docs.ankiweb.net/manual/importing/text-files`,
`https://docs.ankiweb.net/manual/templates/fields`, and
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/qt/aqt/sound.py`.

## 5. Minimal Ranki implementation plan

Storage and format:

1. Extend `NoteModel` with `kind: NoteKind` (`Normal=0`, `Cloze=1`),
   `original_stock_kind: Option<i32>`, and template `ord: u16`. Keep
   `fields: Vec<String>` for compatibility; add parallel `field_tags:
   Vec<Option<u32>>` only if the IO editor is built. Preserve raw model JSON or
   protobuf blobs in a `raw: Option<Vec<u8>>`/JSON field so export can
   round-trip unknown keys. Do not change `Card.ordinal: u16`.
2. In `storage::records::models_from_json`, parse `type`, `originalStockKind`,
   `flds[].tag`, and `tmpls[].ord`. In `models_from_tables`, decode the same
   values from protobuf config blobs instead of only CSS/qfmt/afmt. Sort
   templates by `ord`, not by table row order.
3. In `format::merge::read_col`, replace the `modern_id_map` stub for
   `notetypes` with a full model JSON builder reused by storage. This fixes
   WorldFlags and all schema-18 imports. Preserve model ids unless they
   collide; `merge_models` already reallocates colliding ids and remaps
   `notes.mid`.
4. In `ui::app::render_templates`, replace
   `model.templates.get(usize::from(card.ordinal))` with
   `template_for(model, card)` that looks up `template.ord == card.ordinal`
   for normal models and uses the first template for cloze models. Keep a
   fallback to index 0 for existing models without `ord`.

Renderer (`card_view`):

5. Add `TemplateNode` parsing (`Text`, `Replacement{key, filters}`,
   `Conditional`, `NegatedConditional`) and `parse_template(&str)`.
6. Add `render_template(template, context) -> RenderedHtml` where context has
   fields, note tags, template name, and `card_ord`. Handle `FrontSide`,
   conditionals, `{{cloze:Field}}`, and known filters (`text`, `hint`, `type`)
   as no-op or simple text for now. Keep `parse_markup` for the egui text
   subset.
7. Add `cloze_ordinals(fields: &[String]) -> BTreeSet<u16>` using the same
   tokenization rules: `{{cN::`, comma-separated positive numbers, nested
   clozes, MathJax-aware closing braces. Add `cloze_html(text, ord, side)
   -> ClozeRender` returning segments with `Active{hint, answer}`,
   `Inactive`, and `ImageMask(ShapeMask)`.
8. Add `ShapeMask { shape: ShapeKind, properties: BTreeMap<String,String>,
   ordinals: Vec<u16> }`, `parse_image_occlusion(text)` and
   `mask_to_egui(mask, image_size, side)`. Draw active masks as translucent red
   rect/ellipse/polygon, inactive masks as yellow only when `oi=1`, and
   highlights as red outlines. Reuse `ImageCache`; do not embed JS or canvas in
   egui.
9. Display reversed card ordinal in the UI: `{{Card}}` / template name from
   `template_for`, plus a small `Card 1 of 2` label derived from
   `model.templates.len()`. The scheduler needs no change.
10. Preserve `[sound:...]` and `<img src=...>` in rendered HTML even when
    shown as separate egui widgets. Missing media should produce a caption,
    never a panic or a rewritten field.

Deferred: full HTML/CSS layout, JavaScript template filters, MathJax, TTS,
typed answers, image-occlusion editing, and schema-18 table writes. Reading and
re-exporting schema-11 JSON is enough for the first pass.

## 6. Test cases

- Stock Basic: local model `1735253330886` renders question `Front` and answer
  containing question, `<hr id=answer>`, and `Back`.
- Reversed: model `1735253283095` with `ord=1` renders `Back` first and
  `Front` on the answer; `ord=0` remains the forward card. Both keep separate
  `card.id` and scheduling rows.
- Cloze: `{{c1::one}} {{c2::two::hint}}` renders `[...] two` for `ord=0` and
  `one [hint]` for `ord=1`; answer reveals `one two`. `{{c1,2::shared}}` is
  active on both cards. `{{c0::never}}` generates no card. A note with `c3`
  generates `ord=2`. Order of generated ordinals is sorted.
- Optional reversed: empty `Add Reverse` yields only `ord=0`; nonempty yields
  `ord=1`; the conditional marker itself never appears on the card.
- Image occlusion: parse the four example markers in section 3; assert shape,
  normalized coordinates, `points`, `oi`, `fs`, escaped `text`, and multi-card
  ordinals. Assert question emits masks, answer emits highlight for the active
  shape, and inactive shapes appear only with `oi=1`.
- Import: schema-18 package with empty `col.models` yields models with fields,
  templates, `kind`, CSS and template `ord`; no `This card has no template.`
- Collisions: import two models with the same id; note `mid` and card `ord`
  remain consistent with the remapped model.
- Missing media: `<img src="gone.png">` and `[sound:gone.mp3]` produce captions
  and do not alter note fields.

## Sources

Pinned prefix: `https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/`

- Rust: `rslib/src/{notetype/{stock,cardgen,mod,render},template,template_filters,cloze}.rs`,
  `rslib/src/image_occlusion/{notetype,notetype.css,imageocclusion,imagedata}.rs` (CSS path without `.rs`).
- TypeScript: `ts/routes/image-occlusion/shapes/to-cloze.ts`, `ts/routes/image-occlusion/review.ts`.
- Manual: `https://docs.ankiweb.net/manual/{templates/fields,templates/generation,editing,importing/text-files,templates/styling}`.
- Local `docs/anki-compatibility.md`.
