"""Minimal dependency-free Parquet writer for SYNTHETIC test fixtures only.

Uncompressed, PLAIN, one row group, REQUIRED flat columns. It exists so the ED0
tests can build synthetic E0-shaped packages without pyarrow. It never reads
Parquet and must not be used with real E0 data.
"""
import struct

BOOL, INT32, INT64, STRING, TS_MICROS_UTC = "bool", "int32", "int64", "string", "ts_micros_utc"
_PHYS = {BOOL: 0, INT32: 1, INT64: 2, STRING: 6, TS_MICROS_UTC: 2}


def _varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def _zz(n):
    return _varint((n << 1) ^ (n >> 63))


class _Struct:
    def __init__(self):
        self.b = bytearray()
        self.last = 0

    def _hdr(self, fid, typ):
        d = fid - self.last
        if 0 < d <= 15:
            self.b.append((d << 4) | typ)
        else:
            self.b.append(typ)
            self.b += _zz(fid)
        self.last = fid

    def i32(self, fid, v):
        self._hdr(fid, 5); self.b += _zz(v); return self

    def i64(self, fid, v):
        self._hdr(fid, 6); self.b += _zz(v); return self

    def boolean(self, fid, v):
        self._hdr(fid, 1 if v else 2); return self

    def string(self, fid, s):
        raw = s.encode("utf-8")
        self._hdr(fid, 8); self.b += _varint(len(raw)) + raw; return self

    def struct(self, fid, child):
        self._hdr(fid, 12); self.b += child.done(); return self

    def list(self, fid, elem_type, items):
        self._hdr(fid, 9)
        if len(items) < 15:
            self.b.append((len(items) << 4) | elem_type)
        else:
            self.b.append(0xF0 | elem_type); self.b += _varint(len(items))
        for it in items:
            self.b += it
        return self

    def done(self):
        return bytes(self.b) + b"\x00"


def _plain(kind, values):
    if kind == BOOL:
        out = bytearray((len(values) + 7) // 8)
        for i, v in enumerate(values):
            if v:
                out[i // 8] |= 1 << (i % 8)
        return bytes(out)
    if kind == INT32:
        return b"".join(struct.pack("<i", v) for v in values)
    if kind in (INT64, TS_MICROS_UTC):
        return b"".join(struct.pack("<q", v) for v in values)
    out = bytearray()
    for v in values:
        raw = v.encode("utf-8")
        out += struct.pack("<I", len(raw)) + raw
    return bytes(out)


def _schema_element(name, kind):
    e = _Struct().i32(1, _PHYS[kind]).i32(3, 0).string(4, name)
    if kind == STRING:
        e.i32(6, 0).struct(10, _Struct().struct(1, _Struct()))
    elif kind == TS_MICROS_UTC:
        unit = _Struct().struct(2, _Struct())
        e.i32(6, 10).struct(10, _Struct().struct(8, _Struct().boolean(1, True).struct(2, unit)))
    return e.done()


def write_parquet(path, columns):
    """columns: list of (name, kind, values); all columns the same length."""
    n = len(columns[0][2])
    assert all(len(c[2]) == n for c in columns)
    body = bytearray(b"PAR1")
    chunks = []
    for name, kind, values in columns:
        data = _plain(kind, values)
        page_hdr = _Struct().i32(1, 0).i32(2, len(data)).i32(3, len(data)).struct(
            5, _Struct().i32(1, n).i32(2, 0).i32(3, 3).i32(4, 3)).done()
        offset = len(body)
        body += page_hdr + data
        size = len(page_hdr) + len(data)
        meta = (_Struct().i32(1, _PHYS[kind]).list(2, 5, [_zz(0)]).list(3, 8, [_varint(len(name.encode())) + name.encode()])
                .i32(4, 0).i64(5, n).i64(6, size).i64(7, size).i64(9, offset).done())
        chunks.append(_Struct().i64(2, offset).struct(3, _Struct_raw(meta)).done())
    root = _Struct().string(4, "schema").i32(5, len(columns)).done()
    schema = [root] + [_schema_element(nm, k) for nm, k, _ in columns]
    rg = _Struct().list(1, 12, chunks).i64(2, len(body) - 4).i64(3, n).done()
    footer = (_Struct().i32(1, 1).list(2, 12, schema).i64(3, n).list(4, 12, [rg])
              .string(6, "pulso-ed0-synthetic").done())
    body += footer + struct.pack("<I", len(footer)) + b"PAR1"
    with open(path, "wb") as f:
        f.write(body)


class _Struct_raw(_Struct):
    """Wrap pre-encoded struct bytes (already terminated) for nesting."""

    def __init__(self, raw):
        super().__init__()
        self._raw = raw

    def done(self):
        return self._raw
