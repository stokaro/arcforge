#!/usr/bin/env python3
"""Regenerate dependency notices offline; no compilation or cache mutation.

Requires Python 3.10+ and cargo-about 0.9.3. Run after `cargo fetch --locked`.
The application and build orchestration remain Rust; this is an optional
maintainer-only notice-generation utility, not an application dependency.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import string
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
LICENSES = ROOT / "licenses"
VERSION = "cargo-about 0.9.3"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(text.replace("\r\n", "\n").encode("utf-8"))


def fence(text: str) -> str:
    longest = max((len(m.group()) for m in re.finditer(r"`+", text)), default=0)
    ticks = "`" * max(4, longest + 1)
    separator = "" if text.endswith("\n") else "\n"
    return f"{ticks}text\n{text}{separator}{ticks}\n"


def font_names(path: Path) -> dict[str, list[str]]:
    """Read the SFNT name records containing original copyright/license metadata."""
    data = path.read_bytes()
    count = struct.unpack_from(">H", data, 4)[0]
    offset = None
    for index in range(count):
        tag, _, start, _ = struct.unpack_from(">4sIII", data, 12 + index * 16)
        if tag == b"name":
            offset = start
            break
    if offset is None:
        raise ValueError(f"font has no name table: {path.name}")
    _, count, strings = struct.unpack_from(">HHH", data, offset)
    result: dict[str, set[str]] = {}
    labels = {0: "copyright", 1: "family", 13: "license", 14: "license_url"}
    for index in range(count):
        platform, _, _, name, length, start = struct.unpack_from(">6H", data, offset + 6 + index * 12)
        if name not in labels:
            continue
        raw = data[offset + strings + start:offset + strings + start + length]
        text = raw.decode("utf-16-be" if platform in (0, 3) else "mac_roman").strip("\x00")
        result.setdefault(labels[name], set()).add(text)
    return {key: sorted(values) for key, values in sorted(result.items())}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo-about", default="cargo-about", help="path to the pinned cargo-about executable")
    args = parser.parse_args()
    version = subprocess.check_output([args.cargo_about, "--version"], text=True).strip()
    if version != VERSION:
        raise SystemExit(f"expected {VERSION}, found {version}")
    initial_lock = digest(ROOT / "Cargo.lock")
    with tempfile.TemporaryDirectory(prefix="arcforge-notices-") as directory:
        output = Path(directory) / "licenses.json"
        subprocess.run([
            args.cargo_about, "generate", "--frozen", "--workspace", "--all-features", "--fail",
            "--format", "json", "--output-file", str(output),
        ], cwd=ROOT, check=True)
        report = json.loads(output.read_text(encoding="utf-8"))
    if digest(ROOT / "Cargo.lock") != initial_lock:
        raise SystemExit("Cargo.lock changed during notice generation; retry after dependency changes finish")

    packages = {f"{entry['package']['name']}@{entry['package']['version']}": entry["package"] for entry in report["crates"]}
    mpl_package = packages.get("option-ext@0.2.0")
    if mpl_package is None:
        raise SystemExit("Review the corresponding MPL source distribution after changing option-ext")
    mpl_lock = next((block for block in (ROOT / "Cargo.lock").read_text().split("[[package]]")
                     if re.search(r'^name = "option-ext"$', block, re.M)), "")
    mpl_checksum = re.search(r'^checksum = "([0-9a-f]{64})"$', mpl_lock, re.M)
    if mpl_checksum is None or digest(LICENSES / "source/option-ext-0.2.0.crate") != mpl_checksum[1]:
        raise SystemExit("The distributed option-ext source is not the exact locked registry package")
    source_map = json.loads((LICENSES / "upstream-sources.json").read_text(encoding="utf-8"))
    upstream: dict[str, list[dict]] = {}
    for source in source_map["sources"]:
        for package in source["packages"]:
            upstream.setdefault(package, []).append(source)
    metadata_only = {item["package"]: item for item in source_map["metadata_only"]}

    blocks = []
    missing = []
    for index, license in enumerate(report["licenses"], 1):
        used = sorted(f"{u['crate']['name']}@{u['crate']['version']}" for u in license["used_by"])
        text = license["text"]
        synthetic = not license.get("source_path")
        if synthetic:
            # This line belongs to cargo-about's generic SPDX template, not an upstream notice.
            text = text.replace("Copyright (c) <year> <copyright holders>\n\n", "")
        blocks.append(f"## {index}. {license['name']} ({license['id']})\n\n")
        blocks.append("Applies to: " + ", ".join(f"`{name}`" for name in used) + ".\n\n")
        if synthetic:
            blocks.append("Standard SPDX terms; original source notices are retained in the companion inventory.\n\n")
        elif "<copyright holders>" in text:
            blocks.append("This upstream file itself contains a copyright template placeholder; its text is preserved unchanged.\n\n")
        blocks.append(fence(text) + "\n")
        if synthetic:
            for name in used:
                package = packages[name]
                directory = Path(package["manifest_path"]).parent
                notices = [p for p in directory.iterdir() if p.is_file() and re.search(r"licen[sc]e|copying|notice|copyright", p.name, re.I)]
                if not notices and name not in upstream and name not in metadata_only and not name.startswith("arcforge"):
                    missing.append(name)
    if missing:
        raise SystemExit("Review missing upstream notices before regenerating: " + ", ".join(sorted(set(missing))))

    notice_texts: dict[str, str] = {}

    def notice_link(text: str) -> str:
        normalized = text.replace("\r\n", "\n")
        key = hashlib.sha256(normalized.encode("utf-8")).hexdigest()
        notice_texts[key] = normalized
        return f"[Full notice](#notice-{key}).\n\n"

    notices = ["# Original dependency source notices\n\n", "Generated from the same graph as dependencies.md. Every package/source below links to its complete original notice. Exact texts are deduplicated by SHA-256 after CRLF-to-LF normalization; original source-file hashes remain recorded separately. No license paragraphs or copyright statements are omitted. This conservative inventory includes build/dev and non-desktop target dependencies.\n\n"]
    for name, package in sorted(packages.items()):
        directory = Path(package["manifest_path"]).parent
        notices.append(f"## {name}\n\nDeclared license: `{package.get('license') or 'license-file'}`.\n\n")
        if package.get("repository"):
            notices.append(f"Upstream: <{package['repository']}>.\n\n")
        if package.get("authors"):
            notices.append("Manifest authors (not a substituted copyright notice): " + "; ".join(package["authors"]) + ".\n\n")
        candidates = sorted(p for p in directory.rglob("*") if p.is_file() and re.search(r"licen[sc]e|copying|notice|copyright", p.relative_to(directory).as_posix(), re.I) and p.stat().st_size < 1_000_000)
        if name.startswith("arcforge"):
            candidates = [ROOT / "LICENSE"]
        for candidate in candidates:
            raw = candidate.read_bytes()
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError:
                continue
            relative = candidate.relative_to(directory).as_posix() if candidate.is_relative_to(directory) else "repository LICENSE"
            notices.append(f"### Packaged source: {relative}\n\nSHA-256: `{hashlib.sha256(raw).hexdigest()}`.\n\n" + notice_link(text))
        for source in upstream.get(name, []):
            path = ROOT / source["file"]
            notices.append(f"### Pinned upstream notice\n\nSource: <{source['source_url']}>.\n\n")
            if source.get("note"):
                notices.append(source["note"] + "\n\n")
            notices.append(notice_link(path.read_text(encoding="utf-8")))
        if name in metadata_only:
            item = metadata_only[name]
            notices.append(item["note"] + f" Source: <{item['source_url']}>. The full selected terms are in dependencies.md.\n\n")
        # Preserve published copyright headers where a generator crate lacks a NOTICE file.
        if name.startswith(("gl_generator@", "khronos_api@")):
            source = directory / ("lib.rs" if name.startswith("gl_generator@") else "src/lib.rs")
            header = []
            for line in source.read_text(encoding="utf-8").splitlines():
                if not line.startswith("//"):
                    break
                header.append(line)
            notices.append("### Published source copyright header\n\n" + notice_link("\n".join(header)))
        # Generated protocol/API bindings can carry notices in the input XML/IDL,
        # separate from the Rust wrapper's crate-level license.
        for pattern in ("*.xml", "*.idl"):
            for source in sorted(directory.rglob(pattern)):
                text = source.read_text(encoding="utf-8", errors="strict")
                fragments = re.findall(r"<(?:copyright|comment)\b[^>]*>.*?</(?:copyright|comment)>|<!--.*?-->|/\*.*?\*/", text, re.S)
                for fragment in fragments:
                    if re.search(r"copyright|SPDX-License-Identifier", fragment, re.I):
                        notices.append(f"### Protocol source: {source.relative_to(directory).as_posix()}\n\n" + notice_link(fragment))

    notices.append("# Complete notice texts\n\n")
    for key, text in sorted(notice_texts.items()):
        notices.append(f"## Notice {key}\n\n" + fence(text) + "\n")

    font_package = packages.get("epaint_default_fonts@0.37.0")
    if font_package is None:
        raise SystemExit("Review embedded font files for the new epaint_default_fonts version")
    font_root = Path(font_package["manifest_path"]).parent / "fonts"
    font_dir = LICENSES / "fonts"
    font_dir.mkdir(exist_ok=True)
    provenance = {"crate": "epaint_default_fonts", "version": "0.37.0", "files": {}}
    for name in ["Hack-Regular.txt", "UFL.txt", "OFL.txt", "emoji-icon-font-mit-license.txt", "egui-icons.txt"]:
        raw = (font_root / name).read_bytes()
        # Commit LF consistently; record source hash and whether CRLF was normalized.
        normalized = raw.replace(b"\r\n", b"\n")
        (font_dir / name).write_bytes(normalized)
        provenance["files"][name] = {"source_sha256": hashlib.sha256(raw).hexdigest(), "normalized_crlf": raw != normalized}
    font_docs = ["# Embedded font notices\n\n", "Source: the locked epaint_default_fonts 0.37.0 crate. Full original font licenses are copied beside this file. Copyright and license metadata below are read from each font's original SFNT name table.\n\n", "Hack, Ubuntu Light, and the egui icon subset are unconditional upstream font assets. Noto Emoji and the full emoji-icon font are feature-gated; their notices are included conservatively and do not claim they are active in every binary.\n\n"]
    font_docs.append("ArcForge also embeds the unmodified legacy Open Sans Regular and Semibold Version 1.10 files from Debian fonts-open-sans 1.11-2. Their separate [Apache-2.0 license](OpenSans-Apache-2.0.txt), [original copyright records](OpenSans-NOTICE.txt), and [exact source/hash inventory](../../docs/reference/typography.md) apply to those files. System Apple/Microsoft fonts are read on the user's machine and are not redistributed.\n\n")
    for name in ["Hack-Regular.ttf", "Ubuntu-Light.ttf", "egui-icons.ttf", "NotoEmoji-Regular.ttf", "emoji-icon-font.ttf"]:
        values = font_names(font_root / name)
        provenance["files"][name] = {"source_sha256": digest(font_root / name), "name_metadata": values}
        font_docs.append(f"## {name}\n\n")
        for key, texts in values.items():
            font_docs.append(f"### {key}\n\n" + "\n".join(fence(text) for text in texts) + "\n")
    write(font_dir / "README.md", "".join(font_docs))
    write(font_dir / "provenance.json", json.dumps(provenance, indent=2, ensure_ascii=False) + "\n")

    template = string.Template((ROOT / "scripts/dependency-licenses.md.in").read_text(encoding="utf-8"))
    write(LICENSES / "dependencies.md", template.substitute(package_count=len(packages), body="".join(blocks)))
    write(LICENSES / "dependency-source-notices.md", "".join(notices))
    files = [ROOT / "Cargo.lock", ROOT / "Cargo.toml", ROOT / "about.toml", ROOT / "scripts/dependency-licenses.md.in", Path(__file__), ROOT / "LICENSE"]
    files += list(ROOT.glob("crates/*/Cargo.toml")) + list(ROOT.glob("apps/*/Cargo.toml")) + [ROOT / "xtask/Cargo.toml"]
    files += list(ROOT.glob("crates/*/assets/fonts/*.ttf"))
    files += [p for p in LICENSES.rglob("*") if p.is_file() and p.name != "dependencies.lock.json"]
    for optional in [ROOT / ".gitattributes", ROOT / "rust-toolchain.toml"]:
        if optional.exists():
            files.append(optional)
    snapshot = {"schema": 1, "generator": VERSION, "scope": "active locked graph; all targets; workspace all-features; build/dev included", "package_count": len(packages), "files": {p.relative_to(ROOT).as_posix(): digest(p) for p in sorted(set(files))}}
    if digest(ROOT / "Cargo.lock") != initial_lock:
        raise SystemExit("Cargo.lock changed before snapshot finalization; regenerate")
    write(LICENSES / "dependencies.lock.json", json.dumps(snapshot, indent=2) + "\n")
    print(f"Generated complete terms/source/font notices for {len(packages)} packages; snapshot covers {len(snapshot['files'])} files.")


if __name__ == "__main__":
    main()
