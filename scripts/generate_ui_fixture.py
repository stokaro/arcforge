#!/usr/bin/env python3
"""Create a deterministic, original-content ZIP for native ArcForge screenshots.

This is optional maintainer tooling. Normal builds and tests do not need Python.
The optional large entry lets a real extraction stay visible long enough to
observe progress/cancel without adding large binary fixtures to the repository.
"""

import argparse
from hashlib import sha256
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo


def entry(name: str, method: int = ZIP_DEFLATED) -> ZipInfo:
    info = ZipInfo(name, (2026, 1, 15, 10, 30, 0))
    info.create_system = 3
    info.external_attr = (0o40755 if name.endswith("/") else 0o100644) << 16
    info.compress_type = ZIP_STORED if name.endswith("/") else method
    return info


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--large-mib", type=int, default=0, choices=range(0, 513), metavar="0..512")
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
