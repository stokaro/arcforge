#!/usr/bin/env python3
"""Create a deterministic, original-content ZIP for native ArcForge screenshots.

This is optional maintainer tooling. Normal builds and tests do not need Python.
The optional large entry lets a real extraction stay visible long enough to
observe progress/cancel without adding large binary fixtures to the repository.
"""

import argparse
from hashlib import sha256
from pathlib import Path
import struct
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo
import zlib


def entry(name: str, method: int = ZIP_DEFLATED) -> ZipInfo:
    info = ZipInfo(name, (2026, 1, 15, 10, 30, 0))
    info.create_system = 3
    info.external_attr = (0o40755 if name.endswith("/") else 0o100644) << 16
    info.compress_type = ZIP_STORED if name.endswith("/") else method
    return info


def color_study_png() -> bytes:
    """Original RGB test artwork; no reference-application assets or libraries."""
    width, height = 960, 640
    palette = [
        (36, 85, 143), (55, 126, 180), (111, 179, 202),
        (44, 109, 91), (105, 154, 103), (176, 189, 125),
        (173, 92, 72), (217, 156, 83), (232, 205, 145),
    ]
    rows = bytearray()
    for y in range(height):
        rows.append(0)  # PNG row filter: none.
        for x in range(width):
            color = (242, 240, 233)
            column, column_x = divmod(x - 72, 280)
            row, row_y = divmod(y - 62, 176)
            if 0 <= column < 3 and 0 <= row < 3 and column_x < 256 and row_y < 152:
                color = palette[row * 3 + column]
                if (column_x - 128) ** 2 + (row_y - 76) ** 2 < 48 ** 2:
                    color = tuple(min(255, channel + 25) for channel in color)
            rows.extend(color)

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--large-mib", type=int, default=0, choices=range(0, 513), metavar="0..512")
    parser.add_argument("--images", action="store_true", help="include original PNG preview artwork")
    args = parser.parse_args()
    contents = [
        ("Documents/", b""),
        ("Documents/Guide.txt", b"ArcForge workspace guide\n\nBrowse the archive in Outline view.\nSelect files or folders to extract.\nChoose a new destination directory.\nThe source archive stays unchanged.\n"),
        ("Documents/Release-notes.md", b"# Release notes\n\n- ZIP and ZIP64 browsing\n- Text and hexadecimal previews\n- Selected or complete extraction\n"),
        ("Documents/Notes/", b""),
        ("Documents/Notes/Привет.txt", "Проверка имён файлов и текста.\nАрхив остаётся без изменений.\n".encode()),
        ("Source/", b""),
        ("Source/main.rs", b'fn main() {\n    println!("Hello from ArcForge");\n}\n'),
        ("Source/Cargo.toml", b'[package]\nname = "workspace-example"\nversion = "0.1.0"\nedition = "2024"\n'),
        ("Data/", b""),
        ("Data/settings.json", b'{\n  "view": "outline",\n  "theme": "light",\n  "keep_source": true\n}\n'),
        ("Data/sample.bin", bytes(range(256)) * 8),
        ("README.txt", b"ArcForge example archive\n\nAn original fixture for interface checks.\n\nDocuments contains guides and notes.\nSource contains a small Rust example.\nData contains text and binary samples.\n\nNo private or third-party files are used.\n"),
    ]
    if args.images:
        contents.extend([
            ("Photos/", b""),
            ("Photos/Color-study.png", color_study_png()),
        ])
    # Exclusive creation keeps reruns from overwriting a user's existing file.
    with ZipFile(args.output, "x") as archive:
        archive.comment = b"ArcForge native interface fixture, original MIT content"
        for name, data in contents:
            archive.writestr(entry(name), data)
        if args.large_mib:
            # Stream repeated original bytes; no large in-memory allocation.
            chunk = bytes(range(256)) * 4096
            with archive.open(entry("Data/large-progress.bin"), "w") as output:
                for _ in range(args.large_mib):
                    output.write(chunk)
    data = args.output.read_bytes()
    print(f"{args.output}: {len(data)} ZIP bytes; sha256 {sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()
