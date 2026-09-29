# `loose-archive-xml-2018-12`: an archive XML loose in the release zip

**Release:** 2018-12-14.

## What is wrong

A TRUD release zip holds `fullfile.zip` and `archive.zip`, each with one XML file, and a
newsletter. 2018-12-14's also holds `archive/HSCOrgRefData_Archive_20181210.xml` loose beside
them, outside both inner zips. It is byte-for-byte the XML file inside `archive.zip`, so it adds
nothing, but a reader that takes every XML file in the zip reads the archive twice.

## Evidence

```console
$ unzip -l hscorgrefdataxml_data_12.0.0_20181214000001.zip
  Length      Date    Time    Name
---------  ---------- -----   ----
    86723  11-23-2018 11:43   Newsletter November 18.pdf
        0  12-14-2018 08:16   archive/
 93840236  12-10-2018 22:30   archive/HSCOrgRefData_Archive_20181210.xml
  4092970  12-11-2018 10:00   archive.zip
 17202973  12-11-2018 10:00   fullfile.zip
---------                     -------
115222902                     5 files
$ unzip -p hscorgrefdataxml_data_12.0.0_20181214000001.zip archive/HSCOrgRefData_Archive_20181210.xml | shasum -a 256
c77cb21e3be29fbb768e14e778f755b5fa124986cb3719ca3c66f4daad3b3b80  -
$ unzip -p hscorgrefdataxml_data_12.0.0_20181214000001.zip archive.zip | funzip | shasum -a 256
c77cb21e3be29fbb768e14e778f755b5fa124986cb3719ca3c66f4daad3b3b80  -
```

## How `ods` resolves it

When a release zip holds `fullfile.zip` and `archive.zip`, `ods make` reads the XML inside them
and no XML file beside them, so the loose copy is ignored. The build reads the same two files as
every other release.

This release has no dataset in the index yet: its row records the issue, with TRUD's hash and size
for the archive.

## History

- Dataset 0.1.0: resolved as above. `ods` has read only the inner zips of a release zip that holds
  them since the first build that read both XML files.
