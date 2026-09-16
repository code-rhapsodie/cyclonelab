# cyclonelab

> `cyclonelab` is a CLI for generating and manipulating [CycloneDX](https://cyclonedx.org/) Software Bills of
> Materials (SBOMs). It ships three subcommands — `validate`, `transform`, `suggest` — and
> supports CycloneDX spec versions **1.5**, **1.6**, and **1.7** (JSON only). This file gives an LLM (ChatGPT, Gemini,
> Claude, ...) enough detail to write correct `cyclonelab` invocations and valid `transform` YAML recipes.

Binary name: `cyclonelab`. Global flags: `-h`/`--help`, `-V`/`--version`. Every subcommand also accepts `--help`.

```
cyclonelab <COMMAND>

Commands:
  validate                 Check that a file is valid JSON and conforms to the CycloneDX schema
  transform                Apply a declarative YAML transformation recipe to a CycloneDX SBOM
  suggest                  Suggest useful component/metadata fields missing from a CycloneDX SBOM
  help                     Print this message or the help of the given subcommand(s)
```

Install: download a signed release binary from the
[Releases page](https://github.com/code-rhapsodie/cyclonelab/releases) (Linux/Windows/macOS, arm64/x86_64), verified
with `gh attestation verify ./cyclonelab-<platform> --repo code-rhapsodie/cyclonelab`.

## `cyclonelab validate`

```
cyclonelab validate <FILE>
```

Reads `FILE`, checks it is well-formed JSON, extracts `specVersion`, and validates it against the matching bundled
CycloneDX JSON Schema for that version. Any other `specVersion` value is an explicit "unsupported specVersion" error.
On success: prints `'<file>' is a valid CycloneDX <version> SBOM.` and exits 0. On failure: prints one
`  - <jsonPath>: <message>` line per schema error and exits non-zero. `transform` reuses the exact same validation
logic (see below).

## `cyclonelab suggest`

```
cyclonelab suggest <FILE>
```

Read-only, best-practice linter: never modifies the file. It walks every CycloneDX "component" object in the
document — `metadata.component`, any `components[]` entry (top-level, nested inside another component, or under
`metadata.tools.components[]`), at any depth — and prints one line per field worth adding that is currently missing:
`supplier`, `authors`, `manufacturer`, `licenses`, `copyright`, `cpe`, `swid`, `omniborId`, `hashes`,
`externalReferences`, `pedigree`, `evidence`, `scope`, `signature`, `cryptoProperties`, `data`, `modelCard`. It also
flags the document's `metadata` object if it has no `supplier`, and flags any `licenses[].license` entry that has a
free-text `name` but no SPDX `id`. Output format: `<jsonPath>: <reason>`, one per line; if nothing is missing it
prints a single "no suggestions" line.

## `cyclonelab transform`

Applies a declarative YAML "recipe" of edit steps to a CycloneDX SBOM, revalidating the document against its
CycloneDX schema after every step, and failing (without writing `OUTPUT_FILE`) the moment a step produces an invalid
document.

```
cyclonelab transform <SBOM_FILE> <TRANSFORM_FILE> <OUTPUT_FILE> [--variable name=value]...
```

- `SBOM_FILE`: input CycloneDX JSON document.
- `TRANSFORM_FILE`: YAML recipe (format below).
- `OUTPUT_FILE`: path written (created/overwritten) with the result; becomes a *template* when the recipe declares
  `foreach` (see below).
- `--variable name=value` (repeatable): overrides a declared `variables:` entry; highest-priority source when
  resolving that variable.

Execution order: (1) validate `SBOM_FILE` is JSON and conforms to the CycloneDX schema for its `specVersion`; (2)
parse `TRANSFORM_FILE`, reject unknown/malformed structure with a `file:line:column: message` error, reject duplicate
step `id`s and `manual` steps with an empty `description`; (3) if the recipe declares `from`, require it to equal the
document's `specVersion`, else error; (4) resolve every declared variable; (5) run `steps` in order — substitute
`{$var}` placeholders in every textual field of the step, apply the action, revalidate the whole document against its
(possibly just-changed) `specVersion` schema, stop on first failure; (6) register `cyclonelab` in `metadata.tools`
(alongside any tools already listed, refreshing its own entry if present rather than duplicating it — handles both
the modern `{components:[...]}` shape and the legacy bare-array `Tools` shape); (7) write `OUTPUT_FILE`.

See "Configuration examples" below for complete, runnable recipes covering the common cases (adding fields,
upgrading a spec version, iterating over a folder with `foreach`).

### Recipe file format

```yaml
from: "1.5"          # optional: specVersion required on the input SBOM (checked before running)

foreach:              # optional: repeat the whole pipeline once per matched file, see "foreach" below
  dir: <path, relative to TRANSFORM_FILE's directory>
  pattern: <single-'*' glob>

variables:            # optional
  <name>:
    env: <ENV_VAR_NAME>     # optional: read from this environment variable
    value: <literal>        # optional: default value (any YAML scalar)
    required: true|false    # optional, default false

steps:
  - id: <string, unique within the file>
    action: add | remove | move | merge | transform | manual | upgrade
    description: <free text, optional (required and must be non-empty for 'manual')>
    when: string | object | array   # optional on most actions, see each action below
    # ... action-specific fields, see below
```

**Variable resolution** (first that succeeds wins, resolved once before any step runs): `--variable name=value` on
the CLI → the `env` environment variable → the declared `value` default → if `required: true` and still unresolved,
an interactive prompt on stdin (an explicit error if stdin isn't a TTY, suggesting `--variable`) → otherwise the
variable stays undefined, which is only an error if some step actually references `{$name}`.

**Placeholders** (all namespaced by their leading sigil):
- `{$name}`: a resolved `variables:` entry (or a `foreach` iteration variable) — usable in any textual field of any
  step (`target`, `source`, `value`, `valueFrom.path`, `OUTPUT_FILE`, ...).
- `{@value}`: the value the current action just computed or matched — the value from `add`'s `valueFrom.generator`,
  or the value matched by `source` for `move`/`transform`.
- `{@item}` / `{@item.<field>}`: the array element currently being mapped by the `transform` action's
  `wrap-in-array` (`item:`), `map-array`, or `legacy-array-to-object` (`build.<key>.map`) strategies. A leaf that is
  *exactly* `{@value}`/`{@item}`/`{@item.field}` and nothing else is substituted preserving its JSON type (arrays and
  objects stay arrays/objects); a placeholder embedded inside a longer string is substituted textually.

**JSONPath subset** used by `target`/`source` (deliberately minimal — no numeric indices, no slices):
- `$` — document root.
- `.field` / `.field.sub` — key access.
- `["field"]` / `['field']` — key access for names that aren't valid bare identifiers, e.g. `$.["$schema"]`.
- `[*]` — every element of an array at this position.
- `..field` — recursive descent: matches `field` at any depth, e.g. `$..author`.
- `[?field==literal]` — among an array's elements at this position, only those whose `field` is a *string* equal to
  `literal` (single condition, strict string equality only — no `!=`, no `&&`/`||`, no nesting). A non-array at this
  position matches nothing (empty result, not an error). Example: `$.externalReferences[?type==distribution].hashes`.

A pattern containing `*`/`..`/`[?...]` can resolve to several locations; when a step has both `source` and `target`
with such a pattern, they must resolve the same number of matched parent locations (paired positionally), otherwise
the step errors.

**`when`** (on `add`, `remove`, `move`, `transform`): optional guard — `string`, `object`, or `array` — that only
lets the step act on a matched location whose *current* value already has that JSON type. Used to disambiguate a
field name that has different shapes in different places (e.g. `$..author`: a plain string on a component, an
`identifiableAction` object on a commit).

### Action `add`

Writes a value at `target`, creating missing intermediate *objects* along the way (never creates array elements). If
`target` already holds a value, it is replaced (unless `when` excludes it).

```yaml
- id: <string>
  action: add
  target: <JSONPath>                  # a trailing '[]' switches to "append to array" mode, like merge
  value: <literal or template>        # native YAML — not a JSON string to re-parse
  valueFrom:                          # optional, exactly one of value/valueFrom.generator/valueFrom.file supplies the raw value
    file: <path, relative to TRANSFORM_FILE's directory>
    generator: uuid | timestamp | hash
    format: <strftime format for generator: timestamp; "text" or "json" (default) for file, see below>
    algo: sha256                      # generator: hash only — sha256 is the only supported algorithm today
    path: <file path, generator: hash only, exclusive with url>
    url: <URL, generator: hash only, exclusive with path>
  when: string | object | array       # optional; only touch target if its current value already has this type — not allowed with target[]
```

- `value` alone: a literal, native YAML — `value: "1.6"` stays a string, `value: [ {a: 1} ]` stays an array/object.
  Never write a JSON-looking string in quotes (`value: '[{"a": 1}]'`) expecting it to become an array — it stays a
  plain string.
- `valueFrom.generator: uuid` — a random UUID v4 string.
- `valueFrom.generator: timestamp` — current UTC time, formatted with `format` (default `%Y-%m-%dT%H:%M:%SZ`).
- `valueFrom.generator: hash` — SHA-256 of a local file (`path`, resolved like `valueFrom.file`) or a downloaded URL
  (`url`); exactly one of `path`/`url` is required.
- `valueFrom.file` — reads a file (hard step error if it's missing or unreadable, never a silent skip) and turns its
  content into the value, per `valueFrom.format`: absent or `"json"` (default) JSON-parses it; `"text"` uses the raw
  file content as-is, as a string — no JSON parsing, no `{$var}` substitution on the content itself — for including a
  non-JSON file (e.g. a license's text) without copying it into the transform YAML; any other `format` value is a
  step error.
- When `valueFrom` is used, the raw generated/read value is exposed as `{@value}` for `value` to wrap (e.g.
  `value: "urn:uuid:{@value}"`); if `value` is omitted, the raw value is written as-is.
- Trailing `[]` on `target` (same convention as `merge`): `value` (or the value resolved via `valueFrom`) must be a
  native YAML array; its elements are appended to the array already at `target` (or become it, if absent) — errors if
  `target` exists and isn't an array, or if `when` is set (append mode only makes sense for a single known value, not
  a conditional rewrite of several wildcard-resolved targets).

Example (hash a foreach artifact into an existing `externalReferences` entry selected by `type`):

```yaml
- id: distribution archive hash
  action: add
  target: $.metadata.component.externalReferences[?type==distribution].hashes
  valueFrom:
    generator: hash
    algo: sha256
    path: "{$artifact_path}"
  value:
    - alg: SHA-256
      content: "{@value}"
```

Example (license evidence included from a file on disk, instead of inlined in the transform YAML — appended to an
existing `evidence.licenses` array):

```yaml
- id: add EUPL license evidence
  action: add
  target: $.metadata.component.evidence.licenses[]
  valueFrom:
    file: ../LICENSE-EUPL-1.2
    format: text
  value:
    - license:
        id: EUPL-1.2
        acknowledgement: declared
        text:
          contentType: text/plain
          content: "{@value}"
```

### Action `remove`

```yaml
- id: <string>
  action: remove
  target: <JSONPath>
  when: string | object | array   # optional
```

Deletes every location `target` currently resolves to (after applying `when`). A `target` that resolves to nothing is
a no-op, not an error. Combine with `..field` to strip a field wherever it occurs in the document.

### Action `move`

```yaml
- id: <string>
  action: move
  source: <JSONPath>
  target: <JSONPath>
  when: string | object | array   # optional
```

Renames/relocates the value at each matched `source` location to the sibling key named by `target`'s last segment
(only the final key of `target` is used — the parent path is inherited from each matched `source`'s own parent, so
`move` composes naturally with `..`/`[*]` recursive patterns to rename a field at every depth it occurs). An absent
`source` location is a no-op.

### Action `merge`

```yaml
- id: <string>
  action: merge
  target: <JSONPath>        # a trailing '[]' switches to "append to array" mode
  value: '<JSON string>'    # a JSON-encoded string, re-parsed after {$var} substitution — unlike add.value
```

Without a trailing `[]` on `target`: deep-merges the parsed `value` object into whatever is already at `target`
(fragment keys win, existing keys not mentioned in the fragment are kept) — or simply writes it if `target` is absent
or not an object (full replacement). With a trailing `[]` (e.g. `target: $.metadata.tools.components[]`): `value`
must itself be a JSON array, and its elements are appended to the existing array at the non-`[]` path (or become the
array, if absent) — an existing non-array value there is an explicit step error.

### Action `transform`

Structural changes a plain `move` can't express: type changes, wrapping into an array, field remapping. (This is the
`transform` *action*, distinct from the `transform` *command* the whole recipe runs under.)

```yaml
- id: <string>
  action: transform
  strategy: wrap-in-array | legacy-array-to-object | map-array
  source: <JSONPath>
  target: <JSONPath>              # may equal source (in-place transform)
  when: string | object | array   # optional
  remove_source: true|false       # optional, default false — deletes the original 'source' once copied (only matters when target != source)
  # + strategy-specific fields below
```

- **`wrap-in-array`** — replaces the scalar/object found at `source` with a one-element array at `target`.
  ```yaml
  strategy: wrap-in-array
  append: true|false   # optional, default false: push onto an existing array at target instead of replacing it
  item:                # optional: rebuild the array element from a template instead of copying the value as-is
    <key>: <template using {@value}>
  ```
- **`map-array`** — maps every element of the `source` array (error if `source` isn't an array) through an `item`
  template into a new array at `target`.
  ```yaml
  strategy: map-array
  item:
    <key>: <template using {@item} (whole element) or {@item.<field>} (a field of an object element)>
  ```
  A template field whose value is *exactly* `{@item.<field>}` for a field absent on that particular element is
  dropped from the built object, instead of leaving the literal placeholder text.
- **`legacy-array-to-object`** — turns a legacy array (e.g. `metadata.tools` as a bare array) into an object with one
  or more built arrays under named keys.
  ```yaml
  strategy: legacy-array-to-object
  build:
    <key>:
      each: value          # currently the only supported mode
      map:
        <field>: <template using {@item}/{@item.<field>}>
  ```

### Action `manual`

Never modifies the document — prints a warning naming the step's `description` whenever any of its target path(s)
actually resolve to something in the document, so a recipe author can flag "this needs a human" for a field this
recipe doesn't (or can't) migrate automatically. **Requires a non-empty `description`** (checked at load time, before
any step runs).

```yaml
- id: <string>
  action: manual
  description: <required, non-empty>
  target: <JSONPath>        # exactly one of target/paths
  # or:
  paths: [<JSONPath>, ...]
```

### Action `upgrade`

Brings the document from its current `specVersion` to `version_target` by injecting, right at this point in the
pipeline, the steps of every bundled upgrade recipe needed to chain between them — as if those steps had been pasted
in. Lets a "business" recipe rely on a field shape introduced by a newer schema version (e.g.
`metadata.tools.components[]`, absent in 1.5) before continuing with its own steps.

This action upgrade all necessary properties to obtain a valid SBOM with the new version. The `specVersion` is updated too.

```yaml
- id: <string>
  action: upgrade
  version_target: <string, e.g. "1.6">
```

`version_target` must be reachable from the document's current `specVersion` by chaining the bundled recipes —
currently `"1.6"` or `"1.7"` (never `"1.5"`, since no recipe leads to it, and never an unknown version); rejected at
load time otherwise, listing the versions actually reachable. If the document is already at or past
`version_target`, this is a no-op.

### `foreach` (repeat the whole pipeline per file)

```yaml
foreach:
  dir: <path, relative to TRANSFORM_FILE's directory — same convention as valueFrom.file/valueFrom.path; an
        already-absolute path is used as-is>
  pattern: <single-'*' glob, matched against file names only>

steps: [...]
```

When present, `SBOM_FILE` is loaded/validated once, then the whole `steps` pipeline runs once per file in `dir`
matching `pattern` (files only, sorted by name), each time against a fresh clone of the loaded document — one
iteration's edits never leak into the next. If nothing matches, the command fails (non-zero exit) without writing
any output file, rather than silently succeeding — a pattern that matches nothing usually means a configuration
mistake (wrong `dir`/`pattern`, artifacts not generated yet), which should not look like success to a caller that
only checks the exit code. Each iteration additionally injects three read-only variables (reserved: declaring a
`variables:` entry with one of these names is a load-time error):

- `{$artifact_name}` — the matched file's name with extension.
- `{$artifact_stem}` — the matched file's name without extension.
- `{$artifact_path}` — the matched file's **absolute** path, regardless of whether `dir` was given as relative or
  absolute — typically fed straight into `valueFrom.path` of a `generator: hash` step. It is kept absolute so it
  resolves correctly there even though `valueFrom.path` independently applies the same "relative to TRANSFORM_FILE's
  directory" rule: a relative `{$artifact_path}` would otherwise get that directory prepended a second time.

With `foreach` present, the CLI's `OUTPUT_FILE` argument becomes a template rendered with the same variables (global + 
current iteration) before each write, e.g.:

```
cyclonelab transform template-sbom.cdx.json recipe.yaml "dist/{$artifact_stem}-sbom.cdx.json" \
  --variable repo=code-rhapsodie/cyclonelab --variable version=1.2.0
```

## CycloneDX specifics

- **Supported `specVersion` values**: `1.5`, `1.6`, `1.7` — JSON format only. The matching JSON Schema is bundled and
  resolved offline (no network fetch during validation). Every other `specVersion` value is rejected with an
  explicit "unsupported specVersion" error, both by `validate` and before `transform` runs.
- **Upgrade paths**: `1.5 → 1.6` and `1.6 → 1.7` are the two bundled upgrade recipes, and are also what the
  `upgrade` action chains internally. There is currently no path that skips or reverses these two steps.
- **`metadata.tools`**: whenever `transform` finishes a run, `cyclonelab` registers itself as a tool — adding or
  refreshing its own entry alongside whatever tools are already listed, in whichever of the two valid
  `metadata.tools` shapes the document already uses: the current object form (`{"components": [...]}`, a full
  CycloneDX `Component` entry) or the deprecated bare-array form (`[{vendor, name, version}, ...]`, added in the
  reduced shape that legacy schema actually allows). Running `transform` again refreshes this generator's own entry
  in place rather than duplicating it (matched by `group`+`name`, or `vendor`+`name` in the legacy array form).
- A `manual` step's warning and the `suggest` command are both advisory only — neither blocks `transform`/`validate`
  from succeeding.

## Configuration examples

### Add fields, generate a UUID, hash a file

```yaml
variables:
  supplier_name:
    env: SBOM_SUPPLIER_NAME
    value: "ACME Corp"
  repo:
    required: true

steps:
  - id: set serial number
    action: add
    target: $.serialNumber
    valueFrom:
      generator: uuid
    value: "urn:uuid:{@value}"

  - id: set timestamp
    action: add
    target: $.metadata.timestamp
    valueFrom:
      generator: timestamp

  - id: set supplier
    action: add
    target: $.metadata.component.supplier
    value:
      name: "{$supplier_name}"
      url: ["https://github.com/{$repo}"]

  - id: hash the built artifact
    action: add
    target: $.metadata.component.hashes
    valueFrom:
      generator: hash
      algo: sha256
      path: "dist/artifact.tar.gz"
    value:
      - alg: SHA-256
        content: "{@value}"
```

Run with:

```
cyclonelab transform sbom.json add-fields.yaml sbom.out.json \
  --variable repo=code-rhapsodie/cyclonelab
```

### Add license evidence from files on disk

Instead of copying license texts into the transform YAML, `valueFrom.file` + `format: text` reads them from files
already in the repo — a missing or unreadable file is a hard step error, so a forgotten license never goes unnoticed.
Each license is added with its own `add` step, appended to the same `evidence.licenses` array via the trailing `[]`
on `target` (real example adapted from `.github/transform-sbom.yaml`, which generates cyclonelab's own release SBOMs
this way):

```yaml
steps:
  - id: add EUPL license evidence
    action: add
    description: >
      Attach the full text of the EUPL-1.2 license as evidence, read from the repo's LICENSE-EUPL-1.2 file rather
      than duplicated in this YAML, so it always matches the actual license text.
    target: $.metadata.component.evidence.licenses[]
    valueFrom:
      file: LICENSE-EUPL-1.2
      format: text
    value:
      - license:
          id: EUPL-1.2
          acknowledgement: declared
          url: "https://spdx.org/licenses/EUPL-1.2.html"
          text:
            contentType: text/plain
            content: "{@value}"

  - id: add AGPL license evidence
    action: add
    description: Same as above, for AGPL-3.0, read from LICENSE-AGPL-3.0.
    target: $.metadata.component.evidence.licenses[]
    valueFrom:
      file: LICENSE-AGPL-3.0
      format: text
    value:
      - license:
          id: AGPL-3.0
          acknowledgement: declared
          url: "https://spdx.org/licenses/AGPL-3.0-only.html"
          text:
            contentType: text/plain
            content: "{@value}"
```

### Upgrade an SBOM to a newer spec version

```yaml
steps:
  - id: upgrade to 1.7
    action: upgrade
    version_target: "1.7"
```

```
cyclonelab transform sbom-1.5.json upgrade.yaml sbom-1.7.json
```

### Remove and rename fields, merge a fragment

```yaml
steps:
  - id: drop internal notes
    action: remove
    target: $..internalNotes

  - id: rename legacy field
    action: move
    source: $.metadata.component.vendor
    target: $.metadata.component.manufacturer

  - id: merge extra properties
    action: merge
    target: $.metadata.component.properties[]
    value: '[{"name": "build:pipeline", "value": "ci"}]'
```

### Iterate over a folder of artifacts (`foreach`)

```yaml
foreach:
  dir: dist/artifacts
  pattern: "*.tar.gz"

steps:
  - id: hash artifact
    action: add
    target: $.metadata.component.externalReferences[?type==distribution].hashes
    valueFrom:
      generator: hash
      algo: sha256
      path: "{$artifact_path}"
    value:
      - alg: SHA-256
        content: "{@value}"
```

```
cyclonelab transform template-sbom.json foreach.yaml "dist/{$artifact_stem}-sbom.json"
```
