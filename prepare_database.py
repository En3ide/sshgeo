#!/usr/bin/env python3
"""prépare les bases embarquées à partir de deux CSV locaux DB-IP
ne télécharge rien et n'est pas nécessaire pour compiler ou lancer le programme
"""
import csv
import hashlib
import ipaddress
import json
from pathlib import Path
import sys

if len(sys.argv) != 3:
    raise SystemExit("Usage: python3 prepare_database.py ipv4.csv ipv6.csv")
output = Path(__file__).resolve().parent / "data"
output.mkdir(exist_ok=True)
metadata = {}
for version, name in zip((4, 6), sys.argv[1:]):
    source = Path(name)
    target = output / f"geoip-v{version}.bin"
    size = 4 if version == 4 else 16
    # on vérifie l'ordre des plages car la recherche Rust s'appuie dessus
    count = 0
    previous = -1
    with source.open(encoding="utf-8", newline="") as inp, target.open("wb") as out:
        for start, end, country in csv.reader(inp):
            first, last = ipaddress.ip_address(start), ipaddress.ip_address(end)
            if (first.version != version or last.version != version
                    or int(first) <= previous or int(last) < int(first)
                    or len(country) != 2 or not country.isascii() or not country.isupper()):
                raise ValueError(f"Plage invalide ou non triée : {start},{end},{country}")
            # le format est identique pour toutes les entrées d'une même version IP
            out.write(int(first).to_bytes(size, "big"))
            out.write(int(last).to_bytes(size, "big"))
            out.write(country.encode("ascii"))
            previous = int(last)
            count += 1
    # les empreintes permettent de retrouver les fichiers utilisés pour cette version
    metadata[str(version)] = {"ranges": count,
        "csv_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "binary_sha256": hashlib.sha256(target.read_bytes()).hexdigest()}
    print(target.name, count, "plages", target.stat().st_size, "octets")
(output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
