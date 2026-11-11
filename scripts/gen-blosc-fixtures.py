"""Writes the Blosc fixtures `currents::blosc`'s tests decode: synthetic float32 arrays (no data from
anywhere), compressed the way the Copernicus Marine Zarr stores are (Blosc 1, LZ4, byte shuffle) and in
the other ways a frame can be (no shuffle, one block, stored uncompressed).

Run from the repository root with numcodecs installed: python3 -I scripts/gen-blosc-fixtures.py
The value of element i is ((i * 37) % 1000) / 100 - 5, and every 97th is the stores' fill value."""
import os
import numpy as np
from numcodecs import Blosc

OUT = "crates/isomar-core/src/currents/fixtures"
FILL = np.float32(9.969209968386869e36)


def values(n):
    i = np.arange(n, dtype=np.int64)
    v = (((i * 37) % 1000) / 100.0 - 5.0).astype(np.float32)
    v[i % 97 == 0] = FILL
    return v


def write(name, n, **kw):
    data = Blosc(**kw).encode(values(n))
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(data)
    print(name, n, len(data), "flags", bin(data[2]), "blocksize", int.from_bytes(data[8:12], "little"))


os.makedirs(OUT, exist_ok=True)
# Like a geo-chunked store's chunk: hours by 16 by 8 cells, in blocks of 64 KB (128 hours of the 128 cells).
write("shuffle.blosc", 520 * 16 * 8, cname="lz4", clevel=5, shuffle=Blosc.SHUFFLE, blocksize=16384)
write("noshuffle.blosc", 520 * 16 * 8, cname="lz4", clevel=5, shuffle=Blosc.NOSHUFFLE, blocksize=16384)
write("single.blosc", 100, cname="lz4", clevel=5, shuffle=Blosc.SHUFFLE, blocksize=0)
write("memcpy.blosc", 300, cname="lz4", clevel=0, shuffle=Blosc.NOSHUFFLE, blocksize=0)
