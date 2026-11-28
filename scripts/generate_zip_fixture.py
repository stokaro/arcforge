#!/usr/bin/env python3
"""Regenerate the independent Python-zipfile interoperability fixture.

Normal Rust builds/tests consume the committed hexadecimal file without Python.
"""

from io import BytesIO
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo


def main() -> None:
    output = BytesIO()
    with ZipFile(output, "w") as archive:
        archive.comment = b"ArcForge independent fixture"
        for name, data, method in [
            ("hello.txt", b"Hello from ArcForge.\n", ZIP_STORED),
            ("folder/Привет.txt", "Привет из ZIP.\n".encode(), ZIP_DEFLATED),
        ]:
            entry = ZipInfo(name, (2024, 1, 2, 3, 4, 6))
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            entry.compress_type = method
            archive.writestr(entry, data)
    root = Path(__file__).resolve().parent.parent
    destination = root / "tests/fixtures/python-zipfile.hex"
    destination.parent.mkdir(parents=True, exist_ok=True)
    data = output.getvalue().hex()
    destination.write_text("\n".join(data[i:i + 96] for i in range(0, len(data), 96)) + "\n")
    print(f"Wrote {destination.relative_to(root)} ({len(data) // 2} ZIP bytes)")


if __name__ == "__main__":
    main()
