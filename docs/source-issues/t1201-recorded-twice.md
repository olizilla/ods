# `t1201-recorded-twice`: one organisation, re-recorded for a role change

**Releases:** 2023-03-31 and 2023-04-28.

## What is wrong

`T1201`, NHS LOGISTICS AUTHORITY, is a complete record twice in the archive file of these two
releases, not a record and a `refOnly` stub. Both records have the same name, address and
telephone, for consecutive periods: 2000-04-01 to 2003-03-31 holding role `RO169`, then 2003-04-01
to 2004-09-30 holding `RO189`. A succession links `T1201` to itself. An ODS code names one
organisation, so a table with a row per code can hold only one of them.

The releases either side (2023-02-24, 2023-05-26) hold `T1201` once. In the 2026-08-28 release it
appears once, for the later period: NHS England resolved it the same way `ods` does.

## Evidence

From the release zip, count the organisations with code `T1201` in each inner file (an
organisation's own `OrgId` is indented four spaces; references to it from other organisations are
indented further):

```console
$ unzip -p hscorgrefdataxml_data_3.0.0_20230331000001.zip archive.zip | funzip | grep -c '^    <OrgId .*extension="T1201"'
2
$ unzip -p hscorgrefdataxml_data_3.0.0_20230331000001.zip fullfile.zip | funzip | grep -c '^    <OrgId .*extension="T1201"'
0
```

`hscorgrefdataxml_data_4.0.0_20230428000001.zip` (2023-04-28) gives the same counts. `ods make`
names the record it drops on stderr:

```text
! T1201 has two complete records in HSCOrgRefData_Archive_20230328.xml: kept 2003-04-01 to 2004-09-30, dropped 2000-04-01 to 2003-03-31
```

## How `ods` resolves it

Since dataset version 0.1.0, `ods make` keeps one complete record per code: the full file's over
the archive's, and within one file the record whose operational period starts later. Here both
are in the archive, so the 2003-04-01 record is kept, the one NHS England later kept. Two complete
records in one file with no operational start on either, or with the same start, fail the build:
the source hasn't said which is later, and `ods` doesn't guess. Two stubs for one code fail too.
The rule is D4 in [tests.md](../tests.md); [fef03-code-reused](./fef03-code-reused.md) is the
same rule across the two files.

## History

- Dataset 0.1.0: resolved as above from the first build that read both XML files into one
  `orgs` table.
