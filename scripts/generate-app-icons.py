import argparse
import pathlib
import shutil
import struct
import subprocess


APP_ID = "io.github.battlemountain2.Hermes"
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
ICNS_SIZES = {
    b"ic07": 128,
    b"ic08": 256,
    b"ic09": 512,
    b"ic10": 1024,
    b"ic11": 32,
    b"ic12": 64,
    b"ic13": 256,
    b"ic14": 512,
}


def icns(images):
    entries = [(kind, images[size]) for kind, size in ICNS_SIZES.items()]
    table = b"".join(kind + struct.pack(">I", len(data) + 8) for kind, data in entries)
    body = b"TOC " + struct.pack(">I", len(table) + 8) + table
    body += b"".join(kind + struct.pack(">I", len(data) + 8) + data for kind, data in entries)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def generate(root):
    renderer = shutil.which("rsvg-convert")
    if renderer is None:
        raise RuntimeError("Install librsvg's rsvg-convert to regenerate the icon assets")
    source = root / "data/icons/scalable/apps" / f"{APP_ID}.svg"
    images = {}
    for size in SIZES:
        destination = root / f"data/icons/{size}x{size}/apps" / f"{APP_ID}.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([renderer, "--width", str(size), "--height", str(size), str(source), "--output", str(destination)], check=True)
        images[size] = destination.read_bytes()
    iconset = root / "packaging/macos/Hermes.iconset"
    iconset.mkdir(parents=True, exist_ok=True)
    for size in (16, 32, 128, 256, 512):
        (iconset / f"icon_{size}x{size}.png").write_bytes(images[size])
        (iconset / f"icon_{size}x{size}@2x.png").write_bytes(images[size * 2])
    (iconset.parent / "Hermes.icns").write_bytes(icns(images))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Regenerate Hermes Linux and macOS icon assets from the approved SVG")
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
    generate(parser.parse_args().root)
