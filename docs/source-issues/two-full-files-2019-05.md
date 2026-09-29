# `two-full-files-2019-05`: a full-file zip holding two full files

**Release:** 2019-05-31.

## What is wrong

Each inner zip of a TRUD release normally holds one XML file. 2019-05-31's `fullfile.zip` holds
two full files, `HSCOrgRefData_Full_20190513.xml` (PublicationDate 2019-05-13, sequence 427) and
`HSCOrgRefData_Full_20190528.xml` (2019-05-28, sequence 442), beside one archive file,
`archive.zip`'s, dated 2019-05-28. Built from the first full file, the release would pair a
2019-05-13 full file with a 2019-05-28 archive: two snapshots a fortnight apart, as if they were
one.

No other TRUD release from 2018-06-29 to 2026-09-25 has more than one file in an inner zip.

## Evidence

```console
$ unzip -o -q hscorgrefdataxml_data_5.0.0_20190531000001.zip fullfile.zip -d 2019-05-31
$ unzip -l 2019-05-31/fullfile.zip
  Length      Date    Time    Name
---------  ---------- -----   ----
413769894  05-14-2019 00:00   HSCOrgRefData_Full_20190513.xml
414373811  05-29-2019 00:06   HSCOrgRefData_Full_20190528.xml
---------                     -------
828143705                     2 files
```

`ods make` names the file it skipped on stderr:

```text
! 2019-05-31: fullfile.zip holds 2 full files; built from HSCOrgRefData_Full_20190528.xml (2019-05-28), skipped HSCOrgRefData_Full_20190513.xml (2019-05-13)
```

## How `ods` resolves it

A release's full and archive files are published together, so `ods make` builds from the full file
and archive file whose `PublicationDate` is the same, and names every file it skipped. When more
than one pair shares a date, it takes the one published nearest TRUD's release date. A `--force`
build has no TRUD date to go by, so there it refuses rather than guess, and so does a zip with no
full file and archive file of the same date. The rule is in [nhs.md](../nhs.md#a-zip-holding-two-full-files).

This release has no dataset in the index yet: its row records the issue, with TRUD's hash and size
for the archive.

## History

- Dataset 0.1.0, before `ods` commit `f79ddd0`: `ods make` read the first XML file listed in each
  inner zip, `HSCOrgRefData_Full_20190513.xml`, and paired it with the 2019-05-28 archive. Nothing
  built this way was published.
- Dataset 0.1.0, from commit `f79ddd0`: resolved as above.
