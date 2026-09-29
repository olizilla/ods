# Known source issues

Errors in what a source published, found while building from it, and how `ods` resolves each.
Each page says what is wrong, which releases, the evidence, how `ods` resolves it and since which
dataset version, and the history if the resolution changed.

The [release index](../release-index.md) records each issue's ID on the rows of the releases it
affects, in `source.issues`. `ods cite` and `ods pull` name them for the release they read, with a
link here.

| ID | Releases | What |
| :--- | :--- | :--- |
| [`t1201-recorded-twice`](./t1201-recorded-twice.md) | 2023-03-31, 2023-04-28 | `T1201` is a complete record twice in the archive file, re-recorded for a role change |
| [`fef03-code-reused`](./fef03-code-reused.md) | 2025-05-30 | `FEF03` is a complete record in both files, for two different organisations |
| [`two-full-files-2019-05`](./two-full-files-2019-05.md) | 2019-05-31 | `fullfile.zip` holds two full files, a fortnight apart |
| [`loose-archive-xml-2018-12`](./loose-archive-xml-2018-12.md) | 2018-12-14 | a copy of the archive XML sits loose in the release zip |

## Adding an issue

Write its page here, named by its ID (lower case, words joined by `-`), and list it above. Then add
the ID to each affected release row's `source.issues` in `data/releases.json`, by hand. A release
with no dataset yet gets a row with an empty `datasets` list, with its `source` from TRUD's word.
An ID, once recorded, stays: a page may say the issue is resolved, but the row keeps it.
