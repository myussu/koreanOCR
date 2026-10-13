"""Download only approved comparison files; pin revisions and verify publisher hashes."""
import argparse
import hashlib
import json
import time
import urllib.request
from pathlib import Path


def get_json(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def download(repository, names, folder):
    revision = get_json(f"https://huggingface.co/api/models/{repository}")["sha"]
    entries = get_json(f"https://huggingface.co/api/models/{repository}/tree/{revision}")
    folder.mkdir(parents=True, exist_ok=True)
    records = []
    for name in names:
        entry = next(e for e in entries if e["path"] == name)
        expected = entry.get("lfs", {}).get("oid")
        target = folder / name
        if target.exists():
            digest = hashlib.file_digest(target.open("rb"), "sha256").hexdigest()
            if not expected or target.stat().st_size != entry["size"] or digest != expected:
                raise ValueError(f"Refusing to replace existing file: {target}")
            records.append(dict(file=name, bytes=entry["size"], sha256=digest, publisher_sha256=expected))
            continue
        partial = target.with_name(target.name + ".part")
        digest = hashlib.sha256()
        size = 0
        last = time.monotonic()
        print(f"Downloading {repository}/{name}: {entry['size']} bytes, revision {revision}", flush=True)
        with urllib.request.urlopen(f"https://huggingface.co/{repository}/resolve/{revision}/{name}", timeout=120) as response, partial.open("xb") as output:
            while chunk := response.read(8 * 1024 * 1024):
                output.write(chunk)
                digest.update(chunk)
                size += len(chunk)
                if time.monotonic() - last > 15:
                    print(f"{name}: {size / 1e9:.2f} GB", flush=True)
                    last = time.monotonic()
        actual = digest.hexdigest()
        if size != entry["size"] or (expected and actual != expected):
            raise ValueError(f"Publisher size/checksum mismatch: {name}")
        partial.rename(target)
        records.append(dict(file=name, bytes=size, sha256=actual, publisher_sha256=expected))
    manifest = folder / "download.json"
    with manifest.open("x", encoding="utf-8") as output:
        json.dump(dict(repository=repository, revision=revision, files=records), output, indent=2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    download("mradermacher/MiLMMT-46-4B-v1.0-GGUF", ["MiLMMT-46-4B-v1.0.Q8_0.gguf"], args.out / "milmmt-4b-q8")
    download("quickmt/quickmt-ko-en", ["model.bin", "config.json", "source_vocabulary.json", "target_vocabulary.json", "src.spm.model", "tgt.spm.model"], args.out / "quickmt-ko-en")
