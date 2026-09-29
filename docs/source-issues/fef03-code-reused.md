# `fef03-code-reused`: a code reused for a different organisation

**Release:** 2025-05-30.

## What is wrong

`FEF03` is a complete record in both files of the release, for two different organisations. In the
archive file it is BARKING, HAVERING & BRENTWOOD DISTRICT LAUNDRY COMMON SERVICE AGENCY, closed
1993-03-31. In the full file it is MCGILLS DONCASTER, a pharmacy opened 2025-04-23. Nothing in
either file references `FEF03`. An ODS code names one organisation, so a table with a row per code
can hold only one of them.

The releases either side (2025-04-25, 2025-06-27) hold `FEF03` once: the laundry in the archive
before, the pharmacy in the full file after. In the 2026-08-28 release it appears once, as
MCGILLS DONCASTER: the laundry record has gone from the archive, as `ods` resolves it.

## Evidence

From the release zip, count the organisations with code `FEF03` in each inner file (an
organisation's own `OrgId` is indented four spaces):

```console
$ unzip -p hscorgrefdataxml_data_5.0.0_20250530000001.zip archive.zip | funzip | grep -c '^    <OrgId .*extension="FEF03"'
1
$ unzip -p hscorgrefdataxml_data_5.0.0_20250530000001.zip fullfile.zip | funzip | grep -c '^    <OrgId .*extension="FEF03"'
1
```

`ods make` names the record it drops on stderr:

```text
! FEF03 has a complete record in both files: kept the full file's, dropped the archive's (1991-04-01 to 1993-03-31)
```

## How `ods` resolves it

Since dataset version 0.1.0, `ods make` keeps one complete record per code, and the full file's
record wins over the archive's: MCGILLS DONCASTER is kept. The rule is D4 in
[tests.md](../tests.md); [t1201-recorded-twice](./t1201-recorded-twice.md) is the same rule within
one file.

## History

- Dataset 0.1.0: resolved as above from the first build that read both XML files into one
  `orgs` table.
