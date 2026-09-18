You are generating a `cyclonelab` transform recipe — a declarative YAML file that edits a CycloneDX SBOM.

Before writing anything, read these two references carefully:

1. cyclonelab reference (CLI usage, recipe file format, all actions, JSONPath subset, placeholders, examples):
   https://raw.githubusercontent.com/code-rhapsodie/cyclonelab/v1.x/llms.md

2. CycloneDX JSON Schema for the spec version I'm targeting (use this to know which fields/paths are valid and what shape they must have):
   https://github.com/CycloneDX/specification/tree/master/schema
   (pick the schema file matching my target `specVersion` below, e.g. `bom-1.6.schema.json`)

My target CycloneDX specVersion: <1.5 | 1.6 | 1.7>

What I want the recipe to do:
<describe your goal here — e.g. "add a supplier and a UUID serialNumber", "upgrade from 1.5 to 1.6",
"remove the internalNotes field wherever it appears", "hash a release artifact into externalReferences", etc.>

(Optional) Here is my existing CycloneDX SBOM, to use as the base/context for the transformation —
infer current field values, existing paths, and specVersion from it instead of guessing:

```json
<paste your CycloneDX SBOM here, or delete this block if you don't have one>
```

Requirements for your answer:
- Output ONLY a valid cyclonelab transform recipe in YAML, following the exact file format documented in llms.md
  (top-level from/foreach/variables/steps, correct action names, correct field names per action).
- Use the minimal JSONPath subset supported by cyclonelab ($, .field, ["field"], [*], ..field,
  [?field==literal]) — do not use numeric indices or slices, they are not supported.
- Every target/source path and every value you add must be valid according to the CycloneDX schema for the
  specVersion above.
- If the SBOM I provided is missing something needed to complete the goal, add it explicitly with an add step
  rather than assuming it exists.
- After the YAML, give me the exact cyclonelab transform command line to run it (input file, recipe file, output
  file, and any --variable needed).
- If anything about my goal is ambiguous or underspecified, ask me before generating the recipe.