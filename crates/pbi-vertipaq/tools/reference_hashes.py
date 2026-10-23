#!/usr/bin/env python3
"""Reference column fingerprints for the VertiPaq decoder tests (pbi-vertipaq).

Runs pbixray (MIT, https://github.com/Hugoberry/pbixray) **read-only** over a PBIX
file and prints, for every table and column, only aggregate fingerprints:

    table  <name>  rows=<n>
    column <table>  <column>  type=<TOM DataType code>  nulls=<n>  distinct=<n>  sha256=<hex>

It never prints data values. The hash is SHA-256 over the canonical serialization that
`tests/vertipaq_fixture.rs` computes from the decoded Arrow arrays:

    per row:  b"N\n"                              for null / BLANK
              b"V" + len(text) + b":" + text + b"\n"   otherwise (len = UTF-8 byte length)

    text by TOM DataType (pbi_model DataType):
      2  String    the string itself
      6  Int64     decimal integer
      8  Double    16 lower-case hex digits of the IEEE-754 bit pattern
      9  DateTime  integer microseconds since 1970-01-01T00:00:00 (naive), rounded to
                   the nearest microsecond (pbixray's nanosecond timestamps carry the
                   floating-point residue of the stored OLE date doubles)
      10 Decimal   integer value x 10^4 (Decimal128 scale 4 raw value)
      11 Boolean   "true" / "false"
      other        str(value)

`distinct` counts distinct non-null canonical texts.

Usage (outside the repository's Python environment; pbixray is not a dependency):

    python reference_hashes.py path/to/file.pbix
"""

import hashlib
import struct
import sys
from decimal import Decimal

import pandas as pd
from pbixray import PBIXRay

EPOCH_1970 = pd.Timestamp("1970-01-01")


def canonical_text(value, type_code):
    if type_code == 2:
        return str(value)
    if type_code == 6:
        return str(int(value))
    if type_code == 8:
        return format(struct.unpack("<Q", struct.pack("<d", float(value)))[0], "016x")
    if type_code == 9:
        ts = pd.Timestamp(value).round("us")
        delta = ts - EPOCH_1970
        return str(delta // pd.Timedelta(microseconds=1))
    if type_code == 10:
        return str(int(Decimal(value) * 10000))
    if type_code == 11:
        return "true" if bool(value) else "false"
    return str(value)


def fingerprint(series, type_code):
    digest = hashlib.sha256()
    nulls = 0
    distinct = set()
    for value in series.tolist():
        if value is None or (not isinstance(value, str) and pd.isna(value)):
            nulls += 1
            digest.update(b"N\n")
            continue
        text = canonical_text(value, type_code).encode("utf-8")
        distinct.add(text)
        digest.update(b"V" + str(len(text)).encode() + b":" + text + b"\n")
    return nulls, len(distinct), digest.hexdigest()


def main(path):
    model = PBIXRay(path)
    schema = model._metadata.source.schema_df
    for table in sorted(model.tables):
        df = model.get_table(table)
        print(f"table\t{table}\trows={len(df)}")
        columns = schema[schema["TableName"] == table]
        for _, meta in columns.iterrows():
            name = meta["ColumnName"]
            type_code = int(meta["DataType"])
            nulls, distinct, sha = fingerprint(df[name], type_code)
            print(
                f"column\t{table}\t{name}\ttype={type_code}\tnulls={nulls}"
                f"\tdistinct={distinct}\tsha256={sha}"
            )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: reference_hashes.py <file.pbix>")
    main(sys.argv[1])
